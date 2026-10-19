//! 流程编排（策划案 2.0 · Phase 4.1）。
//!
//! 设计取舍：不做通用工作流编辑器。内置固定模板，顺序执行；每一步都走
//! `run_agent_loop_traced`，因此工具调用、审批门、审计、记忆落库全部复用。
//!
//! 状态机：pending → running → done / failed / cancelled。
//! 取消只保证"不再开始下一步"；正在执行的回合需要用户拒绝审批或等它结束。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::state::EngineState;
use crate::{chain_log, chat, memory};

#[derive(Clone, Debug, serde::Serialize)]
pub struct WorkflowStep {
    pub title: String,
    pub prompt: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WorkflowEvent {
    pub run_id: String,
    pub workflow_id: String,
    pub status: String,
    pub step: usize,
    pub total: usize,
    pub title: String,
    pub detail: String,
}

fn wf(id: &str, name: &str, desc: &str, steps: &[(&str, &str)]) -> Workflow {
    Workflow {
        id: id.to_string(),
        name: name.to_string(),
        description: desc.to_string(),
        steps: steps
            .iter()
            .map(|(t, p)| WorkflowStep {
                title: t.to_string(),
                prompt: p.to_string(),
            })
            .collect(),
    }
}

/// 内置模板。`{{input}}` = 用户输入；`{{previous}}` = 上一步结果。
pub fn builtin_workflows() -> Vec<Workflow> {
    vec![
        wf(
            "research-report",
            "资料整理与报告",
            "围绕一个主题收集资料、核对依据、生成结构化报告（3 步）",
            &[
                (
                    "拆解问题",
                    "请围绕以下主题，列出需要确认的 5–8 个关键问题，以及每项需要什么资料。只输出清单，不要展开。\n\n主题：{{input}}",
                ),
                (
                    "收集资料",
                    "根据上一阶段的问题清单，使用可用工具收集信息（优先 web_read 与工作目录文件）。\n\
                     规则：不要编造；找不到就写\"未找到\"；每条结论标注来源。\n\n\
                     主题：{{input}}\n\n上一阶段清单：\n{{previous}}",
                ),
                (
                    "生成报告",
                    "把收集到的信息整理成结构化报告：摘要、关键结论（含依据/来源）、未决问题、建议下一步。\n\
                     如果过程中产生了文件，请说明路径。\n\n主题：{{input}}\n\n上一阶段结果：\n{{previous}}",
                ),
            ],
        ),
        wf(
            "project-review",
            "项目巡检",
            "扫描工作目录，找出结构/风险/可改进点并给出整改清单（3 步）",
            &[
                (
                    "盘点结构",
                    "请查看当前工作目录，列出顶层结构与关键文件（不要读取超大文件）。\n\n补充要求：{{input}}",
                ),
                (
                    "识别风险",
                    "基于上一阶段的盘点，找出：潜在 bug、安全风险、重复代码、缺失测试、配置隐患。\n\
                     每条给出文件/位置与理由。\n\n上一阶段结果：\n{{previous}}",
                ),
                (
                    "整改清单",
                    "把风险整理成按优先级排序的整改清单（P0/P1/P2），每条包含：问题、影响、建议动作、验证方式。\n\n\
                     上一阶段结果：\n{{previous}}",
                ),
            ],
        ),
        wf(
            "summarize-doc",
            "文档摘要",
            "读取指定文档并提炼摘要与行动项（2 步）",
            &[
                (
                    "阅读与摘要",
                    "请读取并总结以下文档/路径。输出：一句话结论、5 个要点、关键数字/日期。\n\n目标：{{input}}",
                ),
                (
                    "行动项",
                    "基于摘要，提炼可执行的行动项（负责人/截止时间未知就标注待定），并指出仍需确认的问题。\n\n\
                     摘要：\n{{previous}}",
                ),
            ],
        ),
    ]
}

pub fn find_workflow(id: &str) -> Option<Workflow> {
    builtin_workflows().into_iter().find(|w| w.id == id)
}

fn render(template: &str, input: &str, previous: &str) -> String {
    template
        .replace("{{input}}", input)
        .replace("{{previous}}", previous)
}

fn emit(app: &tauri::AppHandle, ev: WorkflowEvent) {
    let _ = app.emit("engine://workflow", ev);
}

/// 启动一个流程（异步线程）。返回 run_id。
pub fn start_workflow(
    app: tauri::AppHandle,
    workflow_id: &str,
    input: &str,
) -> Result<String, String> {
    let wf = find_workflow(workflow_id).ok_or_else(|| format!("没有流程「{workflow_id}」"))?;
    let run_id = format!(
        "w{}-{}",
        crate::now_ms(),
        crate::SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let d = memory::db(&app)?;
    d.create_workflow_run(&run_id, &wf.id, wf.steps.len())?;

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let st = app.state::<EngineState>();
        let mut m = st.workflow_cancels.lock().map_err(|e| e.to_string())?;
        m.insert(run_id.clone(), cancel.clone());
    }

    let app2 = app.clone();
    let input = input.trim().to_string();
    let run_id2 = run_id.clone();
    std::thread::spawn(move || {
        let result = execute(&app2, &wf, &run_id2, &input, &cancel);
        let (status, err) = match result {
            Ok(()) if cancel.load(Ordering::SeqCst) => ("cancelled".to_string(), None),
            Ok(()) => ("done".to_string(), None),
            Err(e) => ("failed".to_string(), Some(e)),
        };
        if let Ok(d) = memory::db(&app2) {
            let step = if status == "done" { wf.steps.len() } else { 0 };
            let _ = d.update_workflow_run(&run_id2, &status, step, err.as_deref());
        }
        emit(
            &app2,
            WorkflowEvent {
                run_id: run_id2.clone(),
                workflow_id: wf.id.clone(),
                status: status.clone(),
                step: wf.steps.len(),
                total: wf.steps.len(),
                title: wf.name.clone(),
                detail: err.unwrap_or_else(|| format!("流程{status}")),
            },
        );
        if let Ok(mut m) = app2.state::<EngineState>().workflow_cancels.lock() {
            m.remove(&run_id2);
        }
        chain_log(&format!("workflow: {run_id2} → {status}"));
    });
    Ok(run_id)
}

fn execute(
    app: &tauri::AppHandle,
    wf: &Workflow,
    run_id: &str,
    input: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    // 与普通回合共用同一把锁：流程运行期间用户不能同时发消息。
    let st = app.state::<EngineState>();
    let _guard = st
        .turn_lock
        .try_lock()
        .map_err(|_| "已有回合在进行中，无法启动流程".to_string())?;

    let workdir = st.workdir.lock().map_err(|e| e.to_string())?.clone();
    let started = *st.session_started.lock().map_err(|e| e.to_string())?;
    if !started {
        crate::commands::start_session_inner(app, &workdir)?;
    }
    let provider = chat::active_id();
    let mut previous = String::new();
    for (i, step) in wf.steps.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        if let Ok(d) = memory::db(app) {
            let _ = d.update_workflow_run(run_id, "running", i, None);
        }
        emit(
            app,
            WorkflowEvent {
                run_id: run_id.to_string(),
                workflow_id: wf.id.clone(),
                status: "running".to_string(),
                step: i + 1,
                total: wf.steps.len(),
                title: step.title.clone(),
                detail: format!("第 {}/{} 步：{}", i + 1, wf.steps.len(), step.title),
            },
        );
        let prompt = render(&step.prompt, input, &previous);
        let turn = crate::agent::run_agent_loop_traced(app, &prompt)?;
        if let Err(e) = memory::persist_turn(
            app,
            &provider,
            &workdir,
            &prompt,
            &turn.reply,
            &turn.tool_calls,
        ) {
            chain_log(&format!("workflow: 步骤落库失败：{e}"));
        }
        previous = turn.reply;
    }
    Ok(())
}

pub fn cancel_workflow(app: &tauri::AppHandle, run_id: &str) -> Result<(), String> {
    let st = app.state::<EngineState>();
    let m = st.workflow_cancels.lock().map_err(|e| e.to_string())?;
    let flag = m
        .get(run_id)
        .ok_or_else(|| format!("流程「{run_id}」没有在运行"))?;
    flag.store(true, Ordering::SeqCst);
    chain_log(&format!("workflow: 请求取消 {run_id}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_workflows_have_steps_and_unique_ids() {
        let list = builtin_workflows();
        assert!(list.len() >= 3);
        let mut ids: Vec<&str> = list.iter().map(|w| w.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), list.len());
        assert!(list.iter().all(|w| !w.steps.is_empty()));
    }

    #[test]
    fn template_renders_input_and_previous() {
        let s = render("主题：{{input}}；上一步：{{previous}}", "A", "B");
        assert_eq!(s, "主题：A；上一步：B");
    }

    #[test]
    fn find_workflow_is_exact() {
        assert!(find_workflow("research-report").is_some());
        assert!(find_workflow("nope").is_none());
    }
}
