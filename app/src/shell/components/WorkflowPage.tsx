// 流程编排（策划案 2.0 · Phase 4.1）
//
// 固定模板 + 顺序执行。每一步都走 Agent 回合循环，因此工具调用/审批门/审计/
// 记忆落库全部复用；本页只负责启动、进度展示与取消。
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Icon } from "../icons";
import type { Workflow, WorkflowEvent, WorkflowRun } from "../types";

export function WorkflowPage() {
  const [workflows, setWorkflows] = useState<Workflow[]>([]);
  const [runs, setRuns] = useState<WorkflowRun[]>([]);
  const [selected, setSelected] = useState("");
  const [input, setInput] = useState("");
  const [activeRun, setActiveRun] = useState<string | null>(null);
  const [progress, setProgress] = useState<WorkflowEvent | null>(null);
  const [err, setErr] = useState("");
  const [msg, setMsg] = useState("");

  const loadRuns = useCallback(async () => {
    try {
      setRuns(await invoke<WorkflowRun[]>("list_workflow_runs"));
    } catch (e) {
      setErr(`读取流程记录失败：${String(e)}`);
    }
  }, []);

  useEffect(() => {
    invoke<Workflow[]>("list_workflows")
      .then((w) => {
        setWorkflows(w);
        setSelected((cur) => cur || w[0]?.id || "");
      })
      .catch((e) => setErr(`读取流程模板失败：${String(e)}`));
    void loadRuns();
  }, [loadRuns]);

  useEffect(() => {
    const un = listen<WorkflowEvent>("engine://workflow", (e) => {
      const ev = e.payload;
      setProgress(ev);
      if (ev.status !== "running") {
        setActiveRun(null);
        void loadRuns();
      }
    });
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, [loadRuns]);

  const wf = workflows.find((w) => w.id === selected);

  const run = async () => {
    if (!wf) return;
    setErr("");
    setMsg("");
    try {
      const runId = await invoke<string>("run_workflow", { id: wf.id, input: input.trim() });
      setActiveRun(runId);
      setProgress({
        run_id: runId,
        workflow_id: wf.id,
        status: "running",
        step: 0,
        total: wf.steps.length,
        title: wf.name,
        detail: "已启动，等待第一步…",
      });
      setMsg(`流程已启动：${runId}`);
      void loadRuns();
    } catch (e) {
      setErr(`启动失败：${String(e)}`);
    }
  };

  const cancel = async () => {
    if (!activeRun) return;
    try {
      await invoke("cancel_workflow", { runId: activeRun });
      setMsg("已请求取消：当前步骤结束后停止");
    } catch (e) {
      setErr(`取消失败：${String(e)}`);
    }
  };

  return (
    <div className="page-inner">
      <h1 className="page-title">流程</h1>
      <p className="page-sub">
        固定模板、顺序执行；每一步都是完整 Agent 回合，<b>工具调用仍走审批门</b>。
        取消只保证不再开始下一步。
      </p>

      {err && (
        <div className="banner warn" role="alert">
          <Icon name="info" size={16} />
          <span className="banner-text">{err}</span>
        </div>
      )}
      {msg && !err && (
        <div className="banner" role="status">
          <Icon name="check" size={16} />
          <span className="banner-text">{msg}</span>
        </div>
      )}

      <div className="section">
        <h2 className="section-title">模板</h2>
        <div className="plugin-list">
          {workflows.map((w) => (
            <label className={`plugin-card catalog-row${w.id === selected ? " on" : ""}`} key={w.id}>
              <div className="pc-head">
                <div className="pc-title">
                  <Icon name="flow" size={16} />
                  <b>{w.name}</b>
                  <span className="badge">{w.steps.length} 步</span>
                </div>
                <input
                  type="radio"
                  name="workflow"
                  checked={w.id === selected}
                  onChange={() => setSelected(w.id)}
                />
              </div>
              <div className="pc-desc">{w.description}</div>
              <div className="tc-hint">
                {w.steps.map((s, i) => `${i + 1}. ${s.title}`).join("　→　")}
              </div>
            </label>
          ))}
        </div>
      </div>

      <div className="section">
        <h2 className="section-title">运行</h2>
        <div className="card">
          <div className="field">
            <div className="field-label">
              <b>任务输入</b>
              <span>会替换模板里的 {"{{input}}"}；留空也可以运行</span>
            </div>
            <div className="field-control">
              <input
                className="input"
                style={{ minWidth: 420 }}
                placeholder="例如：总结 D:\资料\产品手册.pdf；或：巡检 D:\Mica"
                value={input}
                spellCheck={false}
                onChange={(e) => setInput(e.target.value)}
              />
              <button className="btn primary" disabled={!wf || !!activeRun} onClick={() => void run()}>
                <Icon name="zap" size={14} />
                {activeRun ? "运行中…" : "启动流程"}
              </button>
              {activeRun && (
                <button className="btn danger" onClick={() => void cancel()}>
                  <Icon name="stop" size={14} />
                  取消
                </button>
              )}
            </div>
          </div>

          {progress && (
            <div style={{ marginTop: "var(--s-3)" }}>
              <div className="row gap-2" style={{ alignItems: "center" }}>
                <span className={`badge ${progress.status === "failed" ? "danger" : progress.status === "done" ? "ok" : "accent"}`}>
                  {progress.status}
                </span>
                <b>{progress.title}</b>
                <span className="muted">
                  {progress.step}/{progress.total}
                </span>
              </div>
              <div
                style={{
                  height: 6,
                  borderRadius: 3,
                  background: "var(--surface-3)",
                  marginTop: "var(--s-2)",
                  overflow: "hidden",
                }}
              >
                <div
                  style={{
                    width: `${progress.total ? Math.round((progress.step / progress.total) * 100) : 0}%`,
                    height: "100%",
                    background: "var(--accent)",
                    transition: "width .3s ease",
                  }}
                />
              </div>
              <p className="card-desc" style={{ marginTop: "var(--s-2)" }}>
                {progress.detail}
              </p>
            </div>
          )}
        </div>
      </div>

      <div className="section">
        <div className="skill-toolbar">
          <h2 className="section-title" style={{ margin: 0 }}>
            最近运行
          </h2>
          <span className="spacer" />
          <button className="btn ghost sm" onClick={() => void loadRuns()}>
            <Icon name="refresh" size={14} />
            刷新
          </button>
        </div>
        {runs.length === 0 ? (
          <p className="page-sub">还没有运行记录。</p>
        ) : (
          <div className="card flush" style={{ marginTop: "var(--s-2)" }}>
            <table className="table">
              <thead>
                <tr>
                  <th>流程</th>
                  <th>状态</th>
                  <th className="num">进度</th>
                  <th>开始时间</th>
                  <th>错误</th>
                </tr>
              </thead>
              <tbody>
                {runs.map((r) => (
                  <tr key={r.id}>
                    <td className="mono">{r.workflow_id}</td>
                    <td>
                      <span
                        className={`badge ${
                          r.status === "failed" ? "danger" : r.status === "done" ? "ok" : r.status === "cancelled" ? "warn" : "accent"
                        }`}
                      >
                        {r.status}
                      </span>
                    </td>
                    <td className="num tabular">
                      {r.current_step}/{r.total_steps}
                    </td>
                    <td className="muted">{formatTs(r.created_at)}</td>
                    <td className="muted ellipsis" title={r.error ?? ""}>
                      {r.error ?? ""}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}

function formatTs(ms: string): string {
  const n = Number(ms);
  if (!Number.isFinite(n) || n <= 0) return ms;
  const d = new Date(n);
  const pad = (x: number) => String(x).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
