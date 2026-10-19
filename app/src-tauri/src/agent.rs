//! Agent 回合循环：工具标记解析、原生 tool_calls、审批门、自检。
//!
//! 拆分自原单文件 `lib.rs`（可扩展性分析报告 v2 · S1）。
//! `use super::*` 继承根模块的导入与共享项，避免重复声明。

use super::*;

// ---------- 审批 ----------

/// 引擎对"必须逐次确认"的工具（MCP / web_read）在审批文本前加此前缀。
/// 前端 `classify` 会把它归为 danger → 不提供「本会话全部允许」，自动放行规则也不生效。
pub(crate) const FORCE_ASK_PREFIX: &str = "[强制逐次确认]";

fn hash_code(code: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    code.hash(&mut h);
    h.finish()
}

/// 随机审批 id（u32 范围，避免超过 JS `Number.MAX_SAFE_INTEGER` 后丢精度）。
/// `RandomState` 由 OS 随机种子初始化，碰撞时重取；取不到时退回 `SEQ` 并掩到 32 位。
fn next_approval_id(existing: &HashMap<u64, ApprovalEntry>) -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    for _ in 0..16 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(crate::now_ms());
        h.write_u32(std::process::id());
        let id = (h.finish() & 0xFFFF_FFFF) as u64;
        if id != 0 && !existing.contains_key(&id) {
            return id;
        }
    }
    loop {
        let id = (SEQ.fetch_add(1, Ordering::Relaxed) & 0xFFFF_FFFF) as u64;
        if id != 0 && !existing.contains_key(&id) {
            return id;
        }
    }
}

pub(crate) fn request_approval(app: &tauri::AppHandle, code: &str) -> Result<bool, String> {
    // 安全设计：无任何自动批准旁路。工具一律等待用户在 UI 上主动点「允许」；
    // 超时（2 分钟）或 UI 未响应 → 按拒绝处理（fail-closed）。
    let state = app.state::<EngineState>();
    let (tx, rx) = mpsc::channel::<bool>();

    // 敏感路径兜底：审批卡上明确标出“配置/密钥”，并强制 danger。
    // 注意：执行用的仍是原 `code`，警告只进展示文本。
    let forced = security::sensitive_path_hit(code);
    let shown = match forced {
        Some(reason) => {
            let warning = format!("# 安全警告：该操作涉及 Mica {reason}，已强制人工确认");
            if let Some(rest) = code.strip_prefix(FORCE_ASK_PREFIX) {
                format!("{FORCE_ASK_PREFIX}\n{warning}\n{}", rest.trim_start_matches('\n'))
            } else {
                format!("{FORCE_ASK_PREFIX}\n{warning}\n{code}")
            }
        }
        None => code.to_string(),
    };

    let id = {
        let mut m = state.approvals.lock().map_err(|e| e.to_string())?;
        let id = next_approval_id(&m);
        m.insert(
            id,
            ApprovalEntry {
                tx,
                code_hash: hash_code(code),
                forced: forced.is_some() || code.starts_with(FORCE_ASK_PREFIX),
                created_at_ms: crate::now_ms(),
            },
        );
        id
    };
    emit_ev(app, "engine://tool", "request", &format!("{id}|{}", shown));
    let allow = rx.recv_timeout(Duration::from_secs(120)).unwrap_or(false);
    {
        let mut m = state.approvals.lock().map_err(|e| e.to_string())?;
        m.remove(&id);
    }
    Ok(allow)
}

// ---------- Agent 回合循环 ----------

/// 一个回合的完整结果：最终回复 + 本次发生的工具调用（供记忆层落库）。
pub(crate) struct AgentTurn {
    pub(crate) reply: String,
    pub(crate) tool_calls: Vec<crate::memory::ToolCallRecord>,
}

/// 兼容旧调用方：只要最终文本。
pub(crate) fn run_agent_loop(app: &tauri::AppHandle, first_message: &str) -> Result<String, String> {
    Ok(run_agent_loop_traced(app, first_message)?.reply)
}

pub(crate) fn run_agent_loop_traced(
    app: &tauri::AppHandle,
    first_message: &str,
) -> Result<AgentTurn, String> {
    let workdir = app.state::<EngineState>().workdir.lock().map_err(|e| e.to_string())?.clone();
    let mut message = first_message.to_string();
    // 【S3】工具轮次上限 / 超时提示来自 config/agent.json（运行时读取）
    let cfg = config::agent_cfg();
    let mut records: Vec<crate::memory::ToolCallRecord> = Vec::new();

    for iter in 0..=cfg.max_tool_iter {
        // 【Phase 2】统一走 ChatProvider（网页 / Ollama）。原生 tool_calls 可能非空。
        // 【P4.5】提示词体量指标：每轮记录输入/输出字符数，便于观察长对话是否逼近网页上限。
        chain_log(&format!(
            "turn[{iter}]: provider={} 输入 {} 字符",
            chat::active_id(),
            message.chars().count()
        ));
        let turn = chat::send_turn(app, &message)?;
        let reply = turn.text.clone();
        chain_log(&format!(
            "turn[{iter}]: 回复 {} 字符，原生工具调用 {} 个",
            reply.chars().count(),
            turn.tool_calls.len()
        ));

        // ── ⓪ 原生 tool_calls（Ollama / OpenAI 兼容协议）────────────────────
        if !turn.tool_calls.is_empty() {
            if iter == cfg.max_tool_iter {
                return Ok(AgentTurn {
                    reply: iter_limit_msg(&reply, cfg.max_tool_iter),
                    tool_calls: records,
                });
            }
            let mut results = Vec::new();
            for call in turn.tool_calls {
                let name = call.name.clone();
                match execute_json_call(app, &workdir, &cfg, &call, &mut records)? {
                    CallResult::Continue(r) => {
                        results.push(format!("[工具回传][{name}]\n{r}"));
                    }
                    CallResult::Error(e) => {
                        results.push(format!("[工具错误][{name}]\n{e}"));
                    }
                    CallResult::Denied(suffix) => {
                        return Ok(AgentTurn {
                            reply: format!("{reply}\n\n{suffix}"),
                            tool_calls: records,
                        });
                    }
                }
            }
            message = results.join("\n\n");
            continue;
        }

        // ── ① 旧协议（策划案 2.0 明确要求保留，行为与升级前一致）────────────────
        if let Some(call) = tools::parse_call(&reply) {
            // 未知工具名 → 拒绝执行，绝不猜测（也不消耗一轮审批）
            let Some(code) = tools::to_powershell(&call) else {
                audit_log(&format!("未知工具被拒绝: {}", call.name));
                return Ok(AgentTurn {
                    reply: format!("{reply}\n\n[工具「{}」不存在，已忽略]", call.name),
                    tool_calls: records,
                });
            };
            if iter == cfg.max_tool_iter {
                return Ok(AgentTurn {
                    reply: iter_limit_msg(&reply, cfg.max_tool_iter),
                    tool_calls: records,
                });
            }
            match run_approved_powershell(app, &workdir, &call.name, &code, &cfg)? {
                ToolOutcome::Done(result) => {
                    records.push(crate::memory::ToolCallRecord {
                        name: call.name.clone(),
                        args: code.clone(),
                        result: result.clone(),
                        approved: true,
                    });
                    message = format!("[工具回传][{} 执行结果]\n{result}", call.name);
                    continue;
                }
                ToolOutcome::Denied => {
                    records.push(crate::memory::ToolCallRecord {
                        name: call.name.clone(),
                        args: code.clone(),
                        result: String::new(),
                        approved: false,
                    });
                    return Ok(AgentTurn {
                        reply: format!("{reply}\n\n[工具已被用户拒绝，未执行]"),
                        tool_calls: records,
                    });
                }
            }
        }

        // ── ② 新协议：通用 JSON 工具调用（**支持一次一批**） ──────────────────
        // 模型可以在一轮里输出多个 <tool_call> 块，这里全部收集、按顺序执行，
        // 然后把所有结果**一次性**回传 —— 不再是"一轮一个工具"。
        // 单调用场景仍走原来的宽容解析（插件标记格式等），行为不变。
        let parsed_calls: Vec<Result<tools::JsonCall, String>> = {
            let batch = tools::parse_json_calls(&reply);
            if batch.is_empty() {
                tools::parse_json_call(&reply)
                    .or_else(|| tools::parse_lenient_call(&reply))
                    .into_iter()
                    .collect()
            } else {
                batch
            }
        };
        if !parsed_calls.is_empty() {
            if iter == cfg.max_tool_iter {
                return Ok(AgentTurn {
                    reply: iter_limit_msg(&reply, cfg.max_tool_iter),
                    tool_calls: records,
                });
            }
            // 格式错误优先回传（不执行任何一个，先让模型修格式）
            if let Some(Err(e)) = parsed_calls.iter().find(|c| c.is_err()) {
                audit_log(&format!("工具调用格式错误：{e}"));
                emit_ev(app, "engine://state", "tool", "工具调用格式错误，已回传模型");
                message = format!(
                    "[工具调用格式错误] {e}\n请检查格式：标记之间必须是一个 JSON 对象，含 name 与 arguments。"
                );
                continue;
            }
            // 逐个执行（顺序执行：审批与副作用都要求确定性），结果合并回传
            let mut outs: Vec<String> = Vec::new();
            let mut denied: Option<String> = None;
            for c in parsed_calls.iter().flatten() {
                match execute_json_call(app, &workdir, &cfg, c, &mut records)? {
                    CallResult::Continue(r) => outs.push(format!("[工具回传][{}]\n{r}", c.name)),
                    CallResult::Error(e) => outs.push(format!("[工具错误][{}]\n{e}", c.name)),
                    CallResult::Denied(suffix) => {
                        denied = Some(suffix);
                        break;
                    }
                }
            }
            if let Some(suffix) = denied {
                return Ok(AgentTurn {
                    reply: format!("{reply}\n\n{suffix}"),
                    tool_calls: records,
                });
            }
            let n = outs.len();
            chain_log(&format!("tool: 一批 {n} 个调用已执行，合并回传"));
            emit_ev(
                app,
                "engine://state",
                "auto_tool",
                &format!("一批 {n} 个工具调用 → 已合并回传"),
            );
            message = outs.join("\n\n");
            continue;
        }

        // ── ④ 出现工具标记但没能识别 → 不要当成最终答案 ──────────────────────
        // 实测（2026-10-17，日志 tail 含识别尾、finished=true 却 0 个工具调用）：
        // 模型调用了一个**未注册**的工具（agent_getjpg），标记虽在回复里，
        // 但 parse_call 只按已注册工具名匹配 → 匹配不上 → 被当成普通文本返回，
        // 界面看起来就像一条半截回复。必须明确告知模型，而不是默默结束回合。
        if let Some(name) = tools::suspicious_tool_marker(&reply) {
            if iter == cfg.max_tool_iter {
                return Ok(AgentTurn {
                    reply: iter_limit_msg(&reply, cfg.max_tool_iter),
                    tool_calls: records,
                });
            }
            let shown = if name.is_empty() {
                "（无法解析出名字）".to_string()
            } else {
                format!("「{name}」")
            };
            audit_log(&format!("未识别的工具调用块：{name}"));
            // 诊断落盘：模型到底写了什么，必须能拿到原文（否则只能反复猜）
            let dump = format!(r"{}\unrecognized_tool.txt", crate::config::data_dir());
            let _ = std::fs::write(&dump, &reply);
            emit_ev(
                app,
                "engine://state",
                "tool",
                &format!("工具块未识别 {shown}，已回传模型"),
            );
            message = format!(
                "[工具调用未被识别] 你的回复里出现了工具调用标记，但工具 {shown} 没有注册，或标记不完整。\n\
                 可用工具以提示词里的清单为准；请改用清单里存在的工具名，并保持标记完整后重试。"
            );
            continue;
        }

        // ── 没有工具调用 → 本回合结束 ────────────────────────────────────────
        return Ok(AgentTurn { reply, tool_calls: records });
    }
    unreachable!()
}

enum ToolOutcome {
    Done(String),
    Denied,
}

/// 执行一次通用 JSON 工具调用（三条路径共用）：内置 / MCP / PowerShell。
enum CallResult {
    /// 工具结果，回合继续。
    Continue(String),
    /// 可回传给模型的错误，回合继续。
    Error(String),
    /// 用户拒绝，回合结束（返回追加到回复后的说明）。
    Denied(String),
}

fn execute_json_call(
    app: &tauri::AppHandle,
    workdir: &str,
    cfg: &config::AgentCfg,
    call: &tools::JsonCall,
    records: &mut Vec<crate::memory::ToolCallRecord>,
) -> Result<CallResult, String> {
    let args_text = call.arguments.to_string();

    // web_read 是**出站网络请求**：可能被用来探测内网/泄露查询参数 → 必须人工审批。
    // 审批卡展示完整 URL（用户看到的就是将要请求的地址）。
    if call.name == "web_read" {
        let Some(url) = call.arguments.get("url").and_then(|v| v.as_str()) else {
            return Ok(CallResult::Error("web_read 需要字符串参数 url".to_string()));
        };
        // 本地先做协议校验，避免为一个注定失败的请求打扰用户审批。
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(CallResult::Error(
                "web_read 只允许 http/https URL".to_string(),
            ));
        }
        let shown = format!("{FORCE_ASK_PREFIX}\n网页读取请求\nURL: {url}");
        emit_ev(app, "engine://state", "tool", "请求 web_read（等待审批）");
        let allow = request_approval(app, &shown)?;
        if !allow {
            audit_log(&format!("web_read 被用户拒绝：{url}"));
            records.push(crate::memory::ToolCallRecord {
                name: call.name.clone(),
                args: args_text,
                result: String::new(),
                approved: false,
            });
            return Ok(CallResult::Denied("[网页读取已被用户拒绝，未执行]".to_string()));
        }
        let out = match builtin::execute("web_read", &call.arguments) {
            Ok(v) => v,
            Err(e) => format!("[错误] {e}"),
        };
        audit_log(&format!("web_read 读取：{url} → {} 字符", out.chars().count()));
        records.push(crate::memory::ToolCallRecord {
            name: call.name.clone(),
            args: args_text,
            result: out.clone(),
            approved: true,
        });
        // 【可见性】自动放行的工具（内置 / 记忆 / 浏览器）不过审批门，
        // 若只写日志，界面就看不到它到底调了什么 —— 模型可以随口编"我用了 X"。
        // 这里发一条 auto_tool 事件，前端加入"已执行"面板。
        emit_ev(
            app,
            "engine://state",
            "auto_tool",
            &format!("{} → {} 字符", call.name, out.chars().count()),
        );
        return Ok(CallResult::Continue(out));
    }

    // 内置工具：纯计算、无副作用 → 不打扰用户（策划案 P1.5）
    // 真实浏览器工具（离屏 Chromium）：需要 AppHandle 与 webview。
    // 审批策略：浏览/读取/点击**不审批**（与内置 web_read 同类，都是取内容）；
    // browse_fill 会提交表单、可能把数据发出去 → **走审批门**。
    if matches!(
        call.name.as_str(),
        "browse" | "browse_click" | "browse_fill" | "browse_read" | "see"
    ) {
        let arg = |k: &str| {
            call.arguments
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let wait = call
            .arguments
            .get("wait_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(6000);
        // 看图：把本地图片附加到 DeepSeek 输入区。
        // 不审批 —— 只读本地文件，交给页面自身上传，不产生外部写。
        if call.name == "see" {
            let path = arg("path");
            let out = if path.trim().is_empty() {
                "[工具错误] see 需要 path".to_string()
            } else {
                match crate::browser::attach_file(app, &path) {
                    Ok(t) => t,
                    Err(e) => format!("[工具错误] {e}"),
                }
            };
            records.push(crate::memory::ToolCallRecord {
                name: call.name.clone(),
                args: args_text,
                result: out.clone(),
                approved: true,
            });
            emit_ev(
                app,
                "engine://state",
                "auto_tool",
                &format!("see → {}", out.chars().count()),
            );
            return Ok(CallResult::Continue(out));
        }
        let out = match call.name.as_str() {
            "browse" => {
                let url = arg("url");
                if url.trim().is_empty() {
                    "[工具错误] browse 需要 url".to_string()
                } else {
                    match crate::browser::browse(app, &url, wait) {
                        Ok(t) => t,
                        Err(e) => format!("[工具错误] {e}"),
                    }
                }
            }
            "browse_click" => {
                let sel = arg("selector");
                match crate::browser::click(app, &sel, wait) {
                    Ok(t) => t,
                    Err(e) => format!("[工具错误] {e}"),
                }
            }
            "browse_fill" => {
                let sel = arg("selector");
                let text = arg("text");
                let submit = call.arguments.get("submit").and_then(|v| v.as_bool()).unwrap_or(false);
                let shown = format!(
                    "浏览器填表\n选择器: {sel}\n内容: {text}\n提交表单: {}",
                    if submit { "是" } else { "否" }
                );
                match request_approval(app, &shown) {
                    Ok(false) => return Ok(CallResult::Denied("[浏览器填表已被用户拒绝]".into())),
                    Ok(true) => match crate::browser::fill(app, &sel, &text, submit) {
                        Ok(t) => t,
                        Err(e) => format!("[工具错误] {e}"),
                    },
                    Err(e) => format!("[工具错误] 审批失败：{e}"),
                }
            }
            _ => match crate::browser::read_current(app) {
                Ok(t) => t,
                Err(e) => format!("[工具错误] {e}"),
            },
        };
        records.push(crate::memory::ToolCallRecord {
            name: call.name.clone(),
            args: args_text,
            result: out.clone(),
            approved: true,
        });
        // 【可见性】自动放行的工具（内置 / 记忆 / 浏览器）不过审批门，
        // 若只写日志，界面就看不到它到底调了什么 —— 模型可以随口编"我用了 X"。
        // 这里发一条 auto_tool 事件，前端加入"已执行"面板。
        emit_ev(
            app,
            "engine://state",
            "auto_tool",
            &format!("{} → {} 字符", call.name, out.chars().count()),
        );
        return Ok(CallResult::Continue(out));
    }
    // 记忆类工具：需要 AppHandle 访问 SQLite。只写本应用的记忆库、不碰用户文件，
    // 因此不经审批（与 now/calc 同类"无外部副作用"），但**会落盘**，故单独记录。
    if matches!(call.name.as_str(), "remember" | "forget" | "recall") {
        let key = call.arguments.get("key").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let out = match crate::memory::db(app) {
            Err(e) => format!("[工具错误] 记忆库不可用：{e}"),
            Ok(db) => match call.name.as_str() {
                "remember" => {
                    let value = call.arguments.get("value").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                    if key.is_empty() || value.is_empty() {
                        "[工具错误] remember 需要非空的 key 与 value".to_string()
                    } else {
                        match db.memory_set(&key, &value) {
                            Ok(()) => {
                                chain_log(&format!("memory: 模型写入长期记忆「{key}」"));
                                format!("已记住：{key} = {value}（下次会话会自动注入提示词）")
                            }
                            Err(e) => format!("[工具错误] 写入记忆失败：{e}"),
                        }
                    }
                }
                "forget" => match db.memory_delete(&key) {
                    Ok(()) => format!("已删除记忆：{key}"),
                    Err(e) => format!("[工具错误] 删除记忆失败：{e}"),
                },
                _ => match db.memory_list() {
                    Ok(rows) => {
                        let picked: Vec<String> = rows
                            .into_iter()
                            .filter(|r| key.is_empty() || r.key == key)
                            .map(|r| format!("{} = {}", r.key, r.value))
                            .collect();
                        if picked.is_empty() { "（没有匹配的长期记忆）".to_string() } else { picked.join("\n") }
                    }
                    Err(e) => format!("[工具错误] 读取记忆失败：{e}"),
                },
            },
        };
        records.push(crate::memory::ToolCallRecord {
            name: call.name.clone(),
            args: args_text,
            result: out.clone(),
            approved: true,
        });
        // 【可见性】自动放行的工具（内置 / 记忆 / 浏览器）不过审批门，
        // 若只写日志，界面就看不到它到底调了什么 —— 模型可以随口编"我用了 X"。
        // 这里发一条 auto_tool 事件，前端加入"已执行"面板。
        emit_ev(
            app,
            "engine://state",
            "auto_tool",
            &format!("{} → {} 字符", call.name, out.chars().count()),
        );
        return Ok(CallResult::Continue(out));
    }
    if let Some(b) = builtin::find(&call.name) {
        emit_ev(app, "engine://state", "tool", &format!("执行内置工具 {}", b.name));
        let out = match builtin::execute(&call.name, &call.arguments) {
            Ok(v) => v,
            Err(e) => format!("[错误] {e}"),
        };
        chain_log(&format!("builtin: {} → {} 字符", call.name, out.chars().count()));
        records.push(crate::memory::ToolCallRecord {
            name: call.name.clone(),
            args: args_text,
            result: out.clone(),
            approved: true,
        });
        // 【可见性】自动放行的工具（内置 / 记忆 / 浏览器）不过审批门，
        // 若只写日志，界面就看不到它到底调了什么 —— 模型可以随口编"我用了 X"。
        // 这里发一条 auto_tool 事件，前端加入"已执行"面板。
        emit_ev(
            app,
            "engine://state",
            "auto_tool",
            &format!("{} → {} 字符", call.name, out.chars().count()),
        );
        return Ok(CallResult::Continue(out));
    }

    // ★ 阶段 C：MCP 工具（命名 `mcp:<server>.<tool>`）
    // **必须经同一道审批门**（fail-closed），不新增任何旁路。
    if let Some(rest) = call.name.strip_prefix("mcp:") {
        let Some((server, tool)) = rest.split_once('.') else {
            return Ok(CallResult::Error(format!(
                "MCP 工具名应形如 mcp:<server>.<tool>，收到「{}」。",
                call.name
            )));
        };
        let Some(bridge) = mcp_bridge(app) else {
            return Ok(CallResult::Error("当前没有启用任何 MCP server。".to_string()));
        };
        // 审批卡展示的就是这次调用本身（用户看到什么，就执行什么）
        let pretty = serde_json::to_string_pretty(&call.arguments)
            .unwrap_or_else(|_| call.arguments.to_string());
        let shown = format!(
            "{FORCE_ASK_PREFIX}\nMCP 工具调用\nserver: {server}\ntool:   {tool}\n参数:\n{pretty}"
        );
        emit_ev(app, "engine://state", "tool", &format!("请求 MCP {server}.{tool}（等待审批）"));
        let allow = request_approval(app, &shown)?;
        if !allow {
            audit_log(&format!("MCP[{server}] 工具[{tool}] 被用户拒绝"));
            emit_ev(app, "engine://state", "tool", "MCP 工具已被用户拒绝，回合结束");
            records.push(crate::memory::ToolCallRecord {
                name: call.name.clone(),
                args: args_text,
                result: String::new(),
                approved: false,
            });
            return Ok(CallResult::Denied("[MCP 工具已被用户拒绝，未执行]".to_string()));
        }
        emit_ev(app, "engine://state", "tool", &format!("执行 MCP {server}.{tool}"));
        let args: serde_json::Map<String, serde_json::Value> =
            call.arguments.as_object().cloned().unwrap_or_default();
        let out = match bridge.call_with_deadlines(server, tool, args, None, None) {
            Ok(v) => v,
            Err(e) => format!("[错误] {e}"),
        };
        records.push(crate::memory::ToolCallRecord {
            name: call.name.clone(),
            args: args_text,
            result: out.clone(),
            approved: true,
        });
        // 【可见性】自动放行的工具（内置 / 记忆 / 浏览器）不过审批门，
        // 若只写日志，界面就看不到它到底调了什么 —— 模型可以随口编"我用了 X"。
        // 这里发一条 auto_tool 事件，前端加入"已执行"面板。
        emit_ev(
            app,
            "engine://state",
            "auto_tool",
            &format!("{} → {} 字符", call.name, out.chars().count()),
        );
        return Ok(CallResult::Continue(out));
    }

    // 其余一律走 PowerShell 通道 → **同一道审批门**（含插件模板工具与 powershell 本体）
    let Some(code) = json_call_to_powershell(call) else {
        audit_log(&format!("未知工具被拒绝: {}", call.name));
        emit_ev(app, "engine://state", "tool", &format!("未知工具 {}", call.name));
        return Ok(CallResult::Error(
            "没有名为该工具的能力。可用工具见提示词里的工具清单；不要臆造工具名。".to_string(),
        ));
    };
    match run_approved_powershell(app, workdir, &call.name, &code, cfg)? {
        ToolOutcome::Done(result) => {
            records.push(crate::memory::ToolCallRecord {
                name: call.name.clone(),
                args: code,
                result: result.clone(),
                approved: true,
            });
            Ok(CallResult::Continue(result))
        }
        ToolOutcome::Denied => {
            records.push(crate::memory::ToolCallRecord {
                name: call.name.clone(),
                args: code,
                result: String::new(),
                approved: false,
            });
            Ok(CallResult::Denied("[工具已被用户拒绝，未执行]".to_string()))
        }
    }
}

fn iter_limit_msg(reply: &str, max: usize) -> String {
    format!(
        "【已达单轮工具调用上限 {max} 次，已停止】\n\n\
         这是防止模型陷入工具死循环的护栏。要继续，可把 config/agent.json 的 \
         maxToolIterations 调大（无需重编译，下一轮生效），或直接说「继续」让我接着做。\n\n\
         最后一条回复：\n{reply}"
    )
}

/// 把 JSON 工具调用映射成"将要执行的 PowerShell 代码"。
/// `None` = 未知工具（拒绝，不猜）。
fn json_call_to_powershell(call: &tools::JsonCall) -> Option<String> {
    // powershell 本体：直接取 arguments.code
    if call.name == "powershell" {
        return call.arguments.get("code").and_then(|v| v.as_str()).map(|s| s.to_string());
    }
    // 插件模板工具：把 arguments 里的 arg 代入模板
    let tc = tools::ToolCall { name: call.name.clone(), payload: arg_of(&call.arguments) };
    tools::to_powershell(&tc)
}

/// 插件模板工具的 `{{arg}}` 取值：优先 `arg` 字段；没有就取 `arguments` 里第一个字符串值。
fn arg_of(args: &serde_json::Value) -> String {
    if let Some(s) = args.get("arg").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(o) = args.as_object() {
        if let Some((_, v)) = o.iter().find(|(_, v)| v.is_string()) {
            return v.as_str().unwrap_or("").to_string();
        }
    }
    String::new()
}

/// 走**审批门**再执行 PowerShell —— 三条路径（旧协议 / JSON 插件工具 / JSON powershell）
/// 共用这一处，确保没有任何旁路。
fn run_approved_powershell(
    app: &tauri::AppHandle,
    workdir: &str,
    tool_name: &str,
    code: &str,
    cfg: &config::AgentCfg,
) -> Result<ToolOutcome, String> {
    emit_ev(app, "engine://state", "tool", &format!("请求执行 {tool_name}（等待审批）"));
    let allow = request_approval(app, code)?;
    if !allow {
        audit_log(&format!("工具[{tool_name}]被用户拒绝:\n{code}"));
        emit_ev(app, "engine://state", "tool", "工具已被用户拒绝，回合结束");
        // 【E-08】拒绝后直接返回最终回复，不再 continue。
        // 原逻辑会让模型换皮重试（如 Remove-Item → ri -r -fo），最多循环 6 次。
        return Ok(ToolOutcome::Denied);
    }
    emit_ev(app, "engine://state", "tool", &format!("执行中（{}s 超时）", cfg.tool_timeout_sec));
    let result = match execute_powershell(workdir, code) {
        Ok(r) => r,
        Err(e) => format!("[工具错误] {e}"),
    };
    audit_log(&format!(
        "执行命令[{}]（workdir={workdir}）:\n{code}\n--- 结果（前 300 字）---\n{}",
        tool_name,
        result.chars().take(300).collect::<String>()
    ));
    emit_ev(app, "engine://state", "tool", &format!("已执行，回传 {} 字符", result.chars().count()));
    Ok(ToolOutcome::Done(result))
}

// ---------- 自检 ----------

/// 通道自检：对 selectors.json 中**所有组**的**每个候选**执行 querySelectorAll，
/// 返回结构化 JSON：{ groups: { name: { candidates: [{selector, count}], total, healthy } }, summary }
/// healthy = 该组至少有一个候选命中 > 0。total = 所有候选命中数之和（去重前）。
pub(crate) fn run_selfcheck(app: &tauri::AppHandle) -> Result<String, String> {
    let wv = app.get_webview("deepseek").ok_or("deepseek webview not found")?;
    // 【S2】运行时读取选择器配置
    let v: serde_json::Value = serde_json::from_str(&config::selectors_json()).unwrap_or_default();
    let groups = v.get("groups").and_then(|g| g.as_object());
    let mut probe: Vec<serde_json::Value> = Vec::new();
    if let Some(gs) = groups {
        for (name, group) in gs {
            if let Some(cands) = group.get("candidates").and_then(|c| c.as_array()) {
                for c in cands {
                    if let Some(sel) = c.get("selector").and_then(|s| s.as_str()) {
                        probe.push(serde_json::json!({ "g": name, "s": sel }));
                    }
                }
            }
        }
    }
    let probe_json = serde_json::to_string(&probe).unwrap_or_else(|_| "[]".into());
    let js = format!(
        "JSON.stringify((function(){{var C={probe_json};var out={{}};C.forEach(function(c){{if(!out[c.g])out[c.g]={{candidates:[],total:0}};var n=-1;try{{n=document.querySelectorAll(c.s).length}}catch(e){{n=-2}}out[c.g].candidates.push({{selector:c.s,count:n}});out[c.g].total+=Math.max(0,n)}});var unhealthy=[];for(var k in out){{out[k].healthy=out[k].candidates.some(function(x){{return x.count>0}});if(!out[k].healthy)unhealthy.push(k)}}return {{groups:out,unhealthy:unhealthy,allHealthy:unhealthy.length===0}}}})())"
    );
    cdp_eval(&wv, &js)
}
