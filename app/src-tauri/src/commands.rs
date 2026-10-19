//! Tauri IPC 命令与会话初始化。
//!
//! 拆分自原单文件 `lib.rs`（可扩展性分析报告 v2 · S1）。
//! `use super::*` 继承根模块的导入与共享项，避免重复声明。

use super::*;
// `creation_flags` 来自 Windows 的 CommandExt（打开 explorer 时不弹控制台）
#[cfg(windows)]
use std::os::windows::process::CommandExt;

// ---------- 命令 ----------

#[tauri::command]
pub(crate) fn set_browser_rect(app: tauri::AppHandle, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    let Some(wv) = app.get_webview("deepseek") else {
        return Err("deepseek webview not found".into());
    };
    // 归零矩形 = 折叠态：直接隐藏原生视图（不再依赖矩形缩到 1×1）
    if width < 2.0 || height < 2.0 {
        let _ = wv.hide();
        return Ok(());
    }
    let _ = wv.show();
    wv.set_position(tauri::Position::Logical(LogicalPosition::new(x, y)))
        .map_err(|e| e.to_string())?;
    wv.set_size(tauri::Size::Logical(LogicalSize::new(width, height)))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn test_roundtrip(app: tauri::AppHandle, message: String) -> Result<String, String> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<EngineState>();
        let _guard = state.turn_lock.try_lock().map_err(|_| "已有回合在进行中，请稍候".to_string())?;
        Ok(chat::send_turn(&handle, &message)?.text)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// DS 侧开启新对话。返回方式标记："click"（按钮）/ "navigate"（URL 回退）/ "none"（都失败）。
/// 窄栏（480px）下 DS 侧边栏可能压成图标模式、按钮无文本 → 回退到导航根路径（DS 根即新对话入口）。
pub(crate) fn ds_new_conversation(wv: &tauri::webview::Webview) -> Result<&'static str, String> {
    let js = r#"JSON.stringify((function(){var els=document.querySelectorAll('div,button,span,a');var t=null;for(var i=0;i<els.length;i++){var x=els[i];var s=(x.textContent||'').trim();if(s==='开启新对话'||s==='新对话'||s==='新建对话'){var r=x.getBoundingClientRect();if(r.width>0&&r.height>0){t=x;}}}if(!t)return {ok:false};t.click();return {ok:true,label:t.textContent.trim()};})())"#;
    if let Ok(r) = cdp_eval(wv, js) {
        if r.contains("\"ok\":true") {
            return Ok("click");
        }
    }
    // 回退：导航到根路径（新对话入口），等待 textarea 就绪。
    // 注意：不再用 assistant 选择器判断"页面是否清空"——该选择器可能因 DeepSeek 改版而失效，
    // 失效时 0===0 恒为 true，会 falsely 确认就绪。改为仅检测 textarea 存在性 + 非登录页。
    let params = serde_json::json!({ "url": "https://chat.deepseek.com/" }).to_string();
    cdp_call(wv, "Page.navigate", &params)?;
    let ready_js = r#"JSON.stringify({ta:!!document.querySelector('textarea'),signedIn:!window.location.href.includes('sign_in')})"#;
    let mut streak = 0;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(350));
        let ready = cdp_eval(wv, ready_js).unwrap_or_default();
        if ready.contains("\"ta\":true") && ready.contains("\"signedIn\":true") {
            streak += 1;
            if streak >= 2 {
                return Ok("navigate"); // 连续两次就绪 → 页面稳定，避免 SPA 恢复会话中途误判
            }
        } else {
            streak = 0;
        }
    }
    Ok("none")
}

/// 会话初始化：新对话 → 注入系统提示词。调用方持回合锁。
///
/// Phase 2：按 `provider.json` 分流。
/// - `deepseek-web`：沿用原行为（DS 新对话 + 首轮消息即系统提示词）；
/// - `ollama`：把提示词设为 system 消息，不发送用户消息（返回初始化确认）。
pub(crate) fn start_session_inner(app: &tauri::AppHandle, workdir: &str) -> Result<String, String> {
    {
        let state = app.state::<EngineState>();
        {
            let mut wd = state.workdir.lock().map_err(|e| e.to_string())?;
            *wd = workdir.to_string();
        }
        let mut s = state.session_started.lock().map_err(|e| e.to_string())?;
        *s = false;
    }
    let provider_id = chat::active_id();
    let is_web = provider_id != "ollama";
    let caps = chat::current_caps();
    // 每次注入 = 一个新的数据库会话（历史页可回看）。
    if let Err(e) = memory::new_session(app, &provider_id, workdir) {
        chain_log(&format!("memory: 创建会话失败（不影响对话）：{e}"));
    }

    if is_web {
        // 先开 DS 新对话：避免旧上下文（普通聊天痕迹）污染 Agent 人设
        match app.get_webview("deepseek") {
            Some(wv) => match ds_new_conversation(&wv) {
                Ok("click") => {
                    emit_ev(app, "engine://state", "injecting", "已开启 DS 新对话（按钮）");
                    std::thread::sleep(Duration::from_millis(1200));
                }
                Ok("navigate") => {
                    emit_ev(app, "engine://state", "injecting", "已开启 DS 新对话（导航到根）");
                    std::thread::sleep(Duration::from_millis(800));
                }
                Ok(_) => chain_log("ds_new_conversation: 未找到入口，沿用当前会话（上下文可能被污染）"),
                Err(e) => chain_log(&format!("ds_new_conversation error: {e}")),
            },
            None => return Err("deepseek webview not found".into()),
        }
    }
    std::fs::create_dir_all(tmp_dir()).ok();
    // 【技能可用化】把技能库写进临时目录，并注入提示词的 {{SKILLS}} 占位符 ——
    // 否则技能只停留在 UI，模型根本不知道它们存在。
    let skills_block = skills::prompt_block();
    let skills_file = format!(r"{}\skills.md", tmp_dir());
    let _ = std::fs::write(&skills_file, format!("# 可用技能清单\n\n{skills_block}"));
    chain_log(&format!(
        "start_session: 技能 {} 项已写入 {}",
        skills::load().len(),
        skills_file
    ));
    // 【S2】提示词模板运行时读取（改完重启会话即生效，无需重新编译）
    let default_wd = config::default_workdir();
    let effective_wd = if workdir.is_empty() { default_wd.as_str() } else { workdir };
    let mut prompt = config::prompt_template()
        .replace("{{WORKDIR}}", effective_wd)
        .replace("{{TEMP_WORKDIR}}", &tmp_dir())
        .replace("{{SKILLS}}", &skills_block);
    // 【M-01】插件工具说明：**只有在有插件工具时才追加** ——
    // 没有插件工具时提示词与升级前逐字一致，现有回路零影响。
    let extra_tools = tools::extra_prompt_block();
    if !extra_tools.is_empty() {
        chain_log(&format!(
            "start_session: 追加 {} 个插件工具的说明（共 {} 个工具）",
            tools::plugin_tools().len(),
            tools::count()
        ));
        prompt.push_str(&extra_tools);
    }
    // 【策划案 2.0 · P1.3】扩展工具清单（内置工具 + 通用 JSON 调用格式）。
    // 由 config/agent.json 的 extendedTools 控制（默认 true）；设为 false 即回到"只有 PowerShell 协议"。
    let mut tool_blocks = String::new();
    if config::agent_cfg().extended_tools {
        let block = tools::tool_list_prompt_block();
        chain_log(&format!(
            "start_session: 扩展工具清单 {} 字符（内置 {} 个 + 插件 {} 个）",
            block.chars().count(),
            builtin::all().len(),
            tools::plugin_tools().len()
        ));
        tool_blocks.push_str(&block);
    } else {
        chain_log("start_session: extendedTools=false，不注入扩展工具清单");
    }
    // 【阶段 C ④】MCP 工具清单（未配置 server 时为空串 → 提示词与之前逐字一致）
    let mcp_block = mcp_tool_prompt_block(app);
    if !mcp_block.is_empty() {
        chain_log(&format!("start_session: MCP 工具清单 {} 字符", mcp_block.chars().count()));
        tool_blocks.push_str(&mcp_block);
    }
    // 【Phase 2.4】按通道上限裁剪；超限会明确标注"已省略"。
    let tool_blocks = tools::trim_prompt_block(&tool_blocks, caps.max_tool_list_chars);
    chain_log(&format!(
        "start_session: 工具清单最终 {} 字符（上限 {}，通道 {}）",
        tool_blocks.chars().count(),
        caps.max_tool_list_chars,
        provider_id
    ));
    prompt.push_str(&tool_blocks);
    // 【Phase 3.3】长期记忆（为空时提示词与之前逐字一致）
    let mem_block = memory::memory_prompt_block(app, 800);
    if !mem_block.is_empty() {
        chain_log(&format!("start_session: 注入长期记忆 {} 字符", mem_block.chars().count()));
        prompt.push_str(&mem_block);
    }
    chain_log(&format!(
        "start_session: provider={provider_id}, workdir={workdir}, prompt {} 字符",
        prompt.chars().count()
    ));

    let res = if is_web {
        run_agent_loop(app, &prompt)
    } else {
        // Ollama：system 消息 + 空会话，不消耗一轮本地推理。
        chat::set_system_prompt(app, &prompt)?;
        Ok(format!(
            "（会话已初始化：{}；系统提示词 {} 字符已加载）",
            chat::active_label(),
            prompt.chars().count()
        ))
    };
    // 提示词已送进会话（哪怕后续超时）即视为已启动；inject/send 硬失败不算
    let started = match &res {
        Ok(_) => true,
        Err(e) => !(e.starts_with("inject failed") || e.starts_with("send failed")),
    };
    if started {
        if let Ok(mut s) = app.state::<EngineState>().session_started.lock() {
            *s = true;
        }
    }
    res
}

#[tauri::command]
pub(crate) async fn start_agent_session(app: tauri::AppHandle, workdir: String) -> Result<String, String> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<EngineState>();
        let _guard = state.turn_lock.try_lock().map_err(|_| "已有回合在进行中，请稍候".to_string())?;
        start_session_inner(&handle, &workdir)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------- 对话通道（Phase 2.3） ----------

#[tauri::command]
pub(crate) async fn list_providers() -> Result<chat::ProvidersPayload, String> {
    tauri::async_runtime::spawn_blocking(chat::list_providers)
        .await
        .map_err(|e| format!("读取通道列表失败：{e}"))
}

/// 切换通道（可选同时切换 Ollama 模型）。
///
/// 切换即视为"新会话"：清空通道历史、重置会话标记；前端会清空本地消息。
/// 不在这里主动开 DS 新对话 —— 下一次 `start_agent_session` / `agent_turn` 会做。
#[tauri::command]
pub(crate) fn set_provider(
    app: tauri::AppHandle,
    id: String,
    model: Option<String>,
) -> Result<String, String> {
    let id = id.trim();
    if id != "deepseek-web" && id != "ollama" {
        return Err(format!("未知通道「{id}」"));
    }
    // 回合/流程进行中禁止切换：Provider 实例正在被 send_turn 使用。
    let state = app.state::<EngineState>();
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "已有回合或流程在进行中，暂不能切换通道".to_string())?;
    let mut v = config::provider_raw_json();
    if !v.is_object() {
        v = serde_json::json!({});
    }
    let obj = v.as_object_mut().ok_or("provider.json 顶层不是对象")?;
    obj.insert("active".into(), serde_json::Value::String(id.to_string()));
    if let Some(m) = model {
        let m = m.trim();
        if !m.is_empty() {
            let ollama = obj
                .entry("ollama")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(o) = ollama.as_object_mut() {
                o.insert("model".into(), serde_json::Value::String(m.to_string()));
            }
        }
    }
    config::save_provider_json(&v)?;
    // 安全快照冻结了 provider.json；UI 写回后必须显式刷新，否则切换不生效。
    config::refresh_provider_snapshot();

    // 运行时重置：丢弃旧 Provider 实例 + 会话标记 + 当前数据库会话。
    let _ = chat::reset(&app);
    if let Ok(mut s) = app.state::<EngineState>().session_started.lock() {
        *s = false;
    }
    memory::set_current_session(&app, None);
    let label = chat::active_label();
    chain_log(&format!("chat: 用户切换通道 → {id}（{label}）"));
    Ok(format!("已切换到 {label}；下次发送将开启新会话"))
}

#[tauri::command]
pub(crate) async fn agent_turn(app: tauri::AppHandle, workdir: String, message: String) -> Result<String, String> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<EngineState>();
        let _guard = state.turn_lock.try_lock().map_err(|_| "已有回合在进行中，请稍候".to_string())?;
        let started = *state.session_started.lock().map_err(|e| e.to_string())?;
        if !started {
            emit_ev(&handle, "engine://state", "injecting", "未开会话 → 自动先初始化（新对话 + 注入提示词）");
            start_session_inner(&handle, &workdir)?;
        }
        let provider_id = chat::active_id();
        // 【Phase 3.4】知识库检索上下文：只随用户消息发送，不进入系统提示词。
        let kb = memory::kb_context_for(&handle, &message, 2400);
        let send_message = if kb.is_empty() {
            message.clone()
        } else {
            chain_log(&format!("kb: 注入检索上下文 {} 字符", kb.chars().count()));
            format!("{kb}\n[用户问题]\n{message}")
        };
        let turn = run_agent_loop_traced(&handle, &send_message)?;
        // 【Phase 3.2】回合落库（失败不影响对话，只记日志）。
        if let Err(e) = memory::persist_turn(
            &handle,
            &provider_id,
            &workdir,
            &message,
            &turn.reply,
            &turn.tool_calls,
        ) {
            chain_log(&format!("memory: 回合落库失败：{e}"));
        }
        Ok(turn.reply)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) fn tool_decision(app: tauri::AppHandle, id: u64, allow: bool) -> Result<(), String> {
    let state = app.state::<EngineState>();
    let m = state.approvals.lock().map_err(|e| e.to_string())?;
    if let Some(entry) = m.get(&id) {
        let _ = entry.tx.send(allow);
        // 可信审计：不依赖前端回报，记录审批对象哈希与是否强制人工。
        let age = crate::now_ms().saturating_sub(entry.created_at_ms);
        audit_log(&format!(
            "approval #{id}: allow={allow} forced={} age={age}ms code_hash={:016x}",
            entry.forced, entry.code_hash
        ));
        Ok(())
    } else {
        Err(format!("approval {id} not found"))
    }
}

#[tauri::command]
pub(crate) fn selfcheck(app: tauri::AppHandle) -> Result<String, String> {
    // 【S2】附带配置来源信息：一眼看出 selectors / prompt / agent.json
    // 到底读的是运行时文件还是内嵌兜底副本（排障「改了配置怎么没生效」）。
    let probe = run_selfcheck(&app)?;
    let sources = config::config_source_report();
    chain_log(&format!("selfcheck: config sources → {sources}"));
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&probe) else {
        return Ok(probe);
    };
    if let Some(obj) = v.as_object_mut() {
        obj.insert("configSources".to_string(), serde_json::Value::String(sources));
        let c = config::agent_cfg();
        obj.insert(
            "agentCfg".to_string(),
            serde_json::json!({
                "toolTimeoutSec": c.tool_timeout_sec,
                "toolOutputCap": c.tool_output_cap,
                "maxToolIterations": c.max_tool_iter,
                "replyEndpoint": c.reply_endpoint,
            }),
        );
        obj.insert(
            "dataDir".to_string(),
            serde_json::Value::String(config::data_dir()),
        );
        obj.insert(
            "pathProbe".to_string(),
            serde_json::Value::String(config::path_probe_report()),
        );
        obj.insert(
            "securityConfig".to_string(),
            serde_json::to_value(config::security_config_status())
                .unwrap_or(serde_json::Value::Null),
        );
    }
    Ok(v.to_string())
}

/// 网络层探针：把累积的 CDP Network 事件（含 WebSocket 帧原文）落盘并返回摘要。
/// 用途：确认 DeepSeek 网页版的真实流式协议，为「网络层读回」提供依据。
#[tauri::command]
pub(crate) fn net_probe(app: tauri::AppHandle) -> Result<String, String> {
    let json = net_dump(&app);
    let path = format!(r"{}\net_probe.json", data_dir());
    let _ = std::fs::write(&path, &json);
    let summary = serde_json::json!({
        "wsFrameReceived": net_count(&app, "Network.webSocketFrameReceived"),
        "wsFrameSent": net_count(&app, "Network.webSocketFrameSent"),
        "wsCreated": net_count(&app, "Network.webSocketCreated"),
        "wsClosed": net_count(&app, "Network.webSocketClosed"),
        "responseReceived": net_count(&app, "Network.responseReceived"),
        "loadingFinished": net_count(&app, "Network.loadingFinished"),
        "dataReceived": net_count(&app, "Network.dataReceived"),
        "bytes": json.len(),
        "path": path,
    });
    Ok(summary.to_string())
}

// ---------- 技能库（数据驱动，可增删） ----------

#[tauri::command]
pub(crate) fn list_skills() -> Result<Vec<skills::Skill>, String> {
    // 完整视图：本地技能库 + 插件贡献（插件项带 source 标注、不可从本地库删除）
    Ok(skills::load_all())
}

/// 新增技能并返回新列表（前端直接用返回值刷新，避免二次往返）。
#[tauri::command]
pub(crate) fn add_skill(name: String, desc: String, cat: String) -> Result<Vec<skills::Skill>, String> {
    let list = skills::add(&name, &desc, &cat)?;
    chain_log(&format!("skills: 新增「{}」（{}）", name.trim(), cat.trim()));
    Ok(list)
}

#[tauri::command]
pub(crate) fn remove_skill(name: String) -> Result<Vec<skills::Skill>, String> {
    let list = skills::remove(&name)?;
    chain_log(&format!("skills: 删除「{}」", name.trim()));
    Ok(list)
}

/// 恢复内置技能（保留用户自建项）。
#[tauri::command]
pub(crate) fn restore_builtin_skills() -> Result<Vec<skills::Skill>, String> {
    let list = skills::restore_builtin()?;
    chain_log("skills: 已恢复内置清单");
    Ok(list)
}

// ---------- 会话持久化（S-02） ----------

/// 保存会话快照（原子写 + 备份上一版）。前端传 JSON 字符串，后端只做校验与落盘。
#[tauri::command]
pub(crate) fn save_session(state: String) -> Result<(), String> {
    store::save(&state)
}

/// 读取会话快照；无存档返回空串（前端据此走初始状态）。
#[tauri::command]
pub(crate) fn load_session() -> Result<String, String> {
    Ok(store::load())
}

/// 清空会话存档（用于「开新会话」）。
#[tauri::command]
pub(crate) fn clear_session() -> Result<(), String> {
    let r = store::clear();
    chain_log("session: 已清空存档");
    r
}

/// 开新对话：清空前端快照 + 重置引擎会话标记/通道历史 + 取消当前数据库会话指针。
///
/// 只调 `clear_session` 是不够的：引擎的 `session_started` 仍为 true，
/// 下一条消息会继续旧对话（尤其网页通道）。本命令把三者一起复位。
#[tauri::command]
pub(crate) fn new_chat(app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<EngineState>();
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "已有回合或流程在进行中，暂不能开新对话".to_string())?;
    chat::reset(&app)?;
    if let Ok(mut s) = app.state::<EngineState>().session_started.lock() {
        *s = false;
    }
    memory::set_current_session(&app, None);
    store::clear()?;
    chain_log("session: 已开新对话（前端快照 + 引擎会话 + 数据库指针已复位）");
    Ok(())
}

/// 会话存档路径（诊断用，让用户知道东西存在哪）。
#[tauri::command]
pub(crate) fn session_path_info() -> Result<String, String> {
    Ok(format!("存档：{} ｜ 备份：{}", store::session_path(), store::bak_path()))
}

/// 运行时路径（发布环境不得依赖开发机硬编码路径）。
#[derive(serde::Serialize)]
pub(crate) struct AppPaths {
    pub data_dir: String,
    pub cache_dir: String,
    pub webview_dir: String,
    pub default_workdir: String,
    pub path_probe: String,
}

#[tauri::command]
pub(crate) fn app_paths() -> AppPaths {
    AppPaths {
        data_dir: config::data_dir(),
        cache_dir: config::cache_dir(),
        webview_dir: config::webview_dir(),
        default_workdir: config::default_workdir(),
        path_probe: config::path_probe_report(),
    }
}

// ---------- 多会话历史（Phase 3.5） ----------

/// 会话列表（按最近更新倒序，最多 200 条）。
#[tauri::command]
pub(crate) fn list_sessions(app: tauri::AppHandle) -> Result<Vec<memory::db::SessionRow>, String> {
    memory::db(&app)?.list_sessions()
}

#[derive(serde::Serialize)]
pub(crate) struct SessionDetail {
    pub session: memory::db::SessionRow,
    pub messages: Vec<memory::db::MessageRow>,
}

/// 打开一个历史会话：返回元数据 + 消息，并设它为当前会话（后续回合追加到同一记录）。
#[tauri::command]
pub(crate) fn open_session(app: tauri::AppHandle, id: String) -> Result<SessionDetail, String> {
    let d = memory::db(&app)?;
    let session = d
        .list_sessions()?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("会话「{id}」不存在"))?;
    let messages = d.load_messages(&id)?;
    memory::set_current_session(&app, Some(id.clone()));
    chain_log(&format!("memory: 打开历史会话 {id}（{} 条消息）", messages.len()));
    Ok(SessionDetail { session, messages })
}

#[tauri::command]
pub(crate) fn delete_session(app: tauri::AppHandle, id: String) -> Result<(), String> {
    memory::db(&app)?.delete_session(&id)?;
    if memory::current_session(&app).as_deref() == Some(id.as_str()) {
        memory::set_current_session(&app, None);
    }
    chain_log(&format!("memory: 删除历史会话 {id}"));
    Ok(())
}

// ---------- 长期记忆（Phase 3.3） ----------

#[tauri::command]
pub(crate) fn memory_list(app: tauri::AppHandle) -> Result<Vec<memory::db::MemoryRow>, String> {
    memory::db(&app)?.memory_list()
}

#[tauri::command]
pub(crate) fn memory_set(app: tauri::AppHandle, key: String, value: String) -> Result<(), String> {
    memory::db(&app)?.memory_set(&key, &value)?;
    chain_log(&format!("memory: 写入长期记忆「{}」", key.trim()));
    Ok(())
}

#[tauri::command]
pub(crate) fn memory_delete(app: tauri::AppHandle, key: String) -> Result<(), String> {
    memory::db(&app)?.memory_delete(&key)?;
    chain_log(&format!("memory: 删除长期记忆「{key}」"));
    Ok(())
}

// ---------- 流程编排（Phase 4.1） ----------

#[tauri::command]
pub(crate) fn list_workflows() -> Vec<workflows::Workflow> {
    workflows::builtin_workflows()
}

#[tauri::command]
pub(crate) fn run_workflow(
    app: tauri::AppHandle,
    id: String,
    input: String,
) -> Result<String, String> {
    workflows::start_workflow(app, &id, &input)
}

#[tauri::command]
pub(crate) fn cancel_workflow(app: tauri::AppHandle, run_id: String) -> Result<(), String> {
    workflows::cancel_workflow(&app, &run_id)
}

#[tauri::command]
pub(crate) fn list_workflow_runs(app: tauri::AppHandle) -> Result<Vec<memory::db::WorkflowRun>, String> {
    memory::db(&app)?.list_workflow_runs()
}

// ---------- 知识库（Phase 3.4） ----------

#[tauri::command]
pub(crate) async fn kb_import(
    app: tauri::AppHandle,
    path: String,
) -> Result<memory::KbImportResult, String> {
    tauri::async_runtime::spawn_blocking(move || memory::kb_import_path(&app, &path))
        .await
        .map_err(|e| format!("导入任务失败：{e}"))?
}

#[tauri::command]
pub(crate) fn kb_list(app: tauri::AppHandle) -> Result<Vec<memory::db::KbDoc>, String> {
    memory::db(&app)?.kb_list_docs()
}

#[tauri::command]
pub(crate) fn kb_delete(app: tauri::AppHandle, doc_id: i64) -> Result<(), String> {
    memory::db(&app)?.kb_delete_doc(doc_id)?;
    chain_log(&format!("kb: 删除文档 #{doc_id}"));
    Ok(())
}

#[tauri::command]
pub(crate) async fn kb_search(
    app: tauri::AppHandle,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<memory::kb::KbHit>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        memory::kb_search(&app, &query, limit.unwrap_or(5))
    })
    .await
    .map_err(|e| format!("检索任务失败：{e}"))?
}

#[derive(serde::Serialize)]
pub(crate) struct KbStatus {
    pub docs: i64,
    pub chunks: i64,
}

#[tauri::command]
pub(crate) fn kb_status(app: tauri::AppHandle) -> Result<KbStatus, String> {
    let (docs, chunks) = memory::db(&app)?.kb_status()?;
    Ok(KbStatus { docs, chunks })
}

/// 审批风险规则（前端 `classify` 用）。返回 `config/approval.json` 原文，
/// 由前端解析并编译正则；非法项前端会跳过并报出。
#[tauri::command]
pub(crate) fn approval_rules() -> Result<String, String> {
    Ok(config::approval_json())
}

/// **持久自动放行规则**（`config/agent.json` → `autoApprovePatterns`）。
///
/// 默认返回空数组 = 不生效。注意：这里只负责"把用户写的规则交给前端编译"，
/// 安全边界（危险类永不自动放行、逐条审计）在放行路径上强制，不依赖本返回值。
#[tauri::command]
pub(crate) fn auto_approve_rules() -> Result<Vec<String>, String> {
    Ok(config::agent_cfg().auto_approve_patterns)
}

/// 安全配置快照状态：`changed=true` 表示磁盘上的配置与已确认快照不一致，
/// 当前进程仍在使用安全默认/旧快照，等待用户显式重载。
#[tauri::command]
pub(crate) fn security_config_status() -> Result<config::SecurityConfigStatus, String> {
    Ok(config::security_config_status())
}

/// UI 显式确认后采用磁盘上的安全配置；同时重建 MCP 桥接（旧子进程按旧配置运行）。
#[tauri::command]
pub(crate) fn reload_security_config(
    app: tauri::AppHandle,
) -> Result<config::SecurityConfigStatus, String> {
    let status = config::reload_security_config();
    reset_mcp_bridge(&app);
    chain_log(&format!(
        "security: 用户确认重载安全配置（hash={}）",
        status.loaded_hash
    ));
    Ok(status)
}

/// 让前端把**审批来源**写进磁盘审计。
///
/// 存在的理由：会话信任的自动放行发生在前端（引擎只看到"被批准了"），
/// 若前端不回报，`audit.log` 就**无法区分「人工点击允许」与「会话信任自动放行」** ——
/// 那正是审计链最需要留痕的一处。本命令只追加文本，不改变任何放行决策。
#[tauri::command]
pub(crate) fn log_audit(text: String) -> Result<(), String> {
    audit_log(&prepare_audit_text(&text)?);
    Ok(())
}

/// 读取磁盘审计日志尾部，供「审计」页展示**审批来源留痕**
/// （人工允许 / 会话信任自动放行 / 信任集变更）。
///
/// 只读操作，不改动任何状态。窗口上限写死，避免日志增长后把大文件读进内存。
#[tauri::command]
pub(crate) fn read_audit_log() -> Result<String, String> {
    const MAX_BYTES: usize = 64 * 1024;
    const MAX_LINES: usize = 400;
    Ok(log::read_audit_tail(MAX_BYTES, MAX_LINES))
}

/// 审计日志路径与大小（让用户知道留痕存在哪、有多大 —— 该文件**不会自动清理**）。
#[tauri::command]
pub(crate) fn audit_log_path_info() -> Result<String, String> {
    Ok(format!(
        "{} · 当前 {} · 无自动清理",
        audit_log_path(),
        log::audit_log_size_human()
    ))
}

// ---------- 最小插件系统（L-01 第一步） ----------

/// 扫描插件目录并返回插件清单（含解析错误，便于界面显示"为什么没加载"）。
#[tauri::command]
pub(crate) fn list_plugins() -> Result<Vec<plugins::Plugin>, String> {
    let _ = plugins::ensure_dir();
    Ok(plugins::scan().plugins)
}

/// 插件目录路径（首次调用会创建，方便用户直接把插件文件夹丢进去）。
#[tauri::command]
pub(crate) fn plugin_dir_info() -> Result<String, String> {
    plugins::ensure_dir()
}

/// 插件详情（含每个插件贡献的技能与工具明细），供「插件」页展示。
#[tauri::command]
pub(crate) fn plugin_details() -> Result<Vec<plugins::PluginDetail>, String> {
    let _ = plugins::ensure_dir();
    Ok(plugins::details())
}

/// 新建插件（可选附带开箱即用的示例技能 / 示例工具）。
#[tauri::command]
pub(crate) fn create_plugin(
    name: String,
    version: String,
    description: String,
    with_skill: bool,
    with_tool: bool,
) -> Result<String, String> {
    plugins::ensure_dir()?;
    let dir = plugins::create(&name, &version, &description, with_skill, with_tool)?;
    chain_log(&format!(
        "plugins: 新建「{}」→ {dir}（示例技能={with_skill} 示例工具={with_tool}）",
        name.trim()
    ));
    Ok(dir)
}

// ---------- 插件预置库（策划案 2.0 · P1.6，与 MCP 能力库同一套交互）----------

#[derive(serde::Serialize)]
pub(crate) struct PluginCatalogEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub tool_count: usize,
    pub installed: bool,
    pub enabled: bool,
}

struct PluginPreset {
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    cat: &'static str,
    /// (name, title, description, command)
    tools: &'static [(&'static str, &'static str, &'static str, &'static str)],
}

const PLUGIN_PRESETS: &[PluginPreset] = &[
    PluginPreset {
        id: "preset-sysinfo", name: "系统信息", cat: "系统",
        desc: "磁盘占用、内存、占用最高的进程",
        tools: &[
            ("disk-usage", "磁盘占用", "列出各盘剩余空间", "Get-PSDrive -PSProvider FileSystem | Select-Object Name,@{n='UsedGB';e={[math]::Round($_.Used/1GB,1)}},@{n='FreeGB';e={[math]::Round($_.Free/1GB,1)}} | Format-Table -AutoSize"),
            ("mem-usage", "内存占用", "总内存与可用内存", "(Get-CimInstance Win32_OperatingSystem) | Select-Object @{n='TotalGB';e={[math]::Round($_.TotalVisibleMemorySize/1MB,1)}},@{n='FreeGB';e={[math]::Round($_.FreePhysicalMemory/1MB,1)}} | Format-List"),
            ("top-proc", "最占资源的进程", "按 CPU 时间取前 N 个进程；参数为数量", "Get-Process | Sort-Object CPU -Descending | Select-Object -First {{arg}} Name,Id,@{n='CPUsec';e={[math]::Round($_.CPU,1)}} | Format-Table -AutoSize"),
        ],
    },
    PluginPreset {
        id: "preset-netdiag", name: "网络诊断", cat: "系统",
        desc: "连通性、端口、DNS 解析",
        tools: &[
            ("ping-host", "Ping", "测试到某个主机的连通性；参数为主机名或 IP", "Test-Connection -TargetName '{{arg}}' -Count 4 | Select-Object Address,ResponseTime | Format-Table -AutoSize"),
            ("port-check", "端口探测", "测试本机某端口是否在听；参数为端口号", "Get-NetTCPConnection -State Listen -LocalPort {{arg}} -ErrorAction SilentlyContinue | Select-Object LocalAddress,LocalPort,OwningProcess | Format-Table -AutoSize"),
            ("dns-lookup", "DNS 解析", "解析域名；参数为域名", "Resolve-DnsName '{{arg}}' | Select-Object Name,Type,IPAddress | Format-Table -AutoSize"),
        ],
    },
    PluginPreset {
        id: "preset-files", name: "文件整理", cat: "文件",
        desc: "目录大小、最大的文件、临时文件统计",
        tools: &[
            ("dir-size", "目录大小", "统计某目录总大小；参数为目录路径", "$p='{{arg}}'; if(Test-Path $p){ $s=(Get-ChildItem -LiteralPath $p -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum); \\\"{0}  文件数={1}  大小={2:N1} MB\\\" -f $p,$s.Count,($s.Sum/1MB) } else { 'path not found' }"),
            ("biggest-files", "最大的文件", "列出某目录下最大的 N 个文件；参数为 路径|数量", "$a='{{arg}}' -split '\\|'; $p=$a[0]; $n=if($a.Count -gt 1){[int]$a[1]}else{10}; Get-ChildItem -LiteralPath $p -Recurse -File -ErrorAction SilentlyContinue | Sort-Object Length -Descending | Select-Object -First $n @{n='MB';e={[math]::Round($_.Length/1MB,2)}},FullName | Format-Table -AutoSize"),
            ("temp-usage", "临时文件占用", "统计用户临时目录占用（只统计，不删除）", "$p=$env:TEMP; $s=(Get-ChildItem -LiteralPath $p -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum); \\\"TEMP={0}  文件数={1}  大小={2:N1} MB\\\" -f $p,$s.Count,($s.Sum/1MB)"),
        ],
    },
    PluginPreset {
        id: "preset-text", name: "文本处理", cat: "文本",
        desc: "行数/字数统计、编码检测、内容预览",
        tools: &[
            ("line-count", "行数与字数", "统计文本文件的行数与字符数；参数为文件路径", "$p='{{arg}}'; if(Test-Path $p){ $c=Get-Content -LiteralPath $p; \\\"行数={0}  字符数={1}\\\" -f $c.Count,(Get-Content -LiteralPath $p -Raw).Length } else { 'file not found' }"),
            ("text-head", "预览前 N 行", "预览文件开头；参数为 路径|行数", "$a='{{arg}}' -split '\\|'; $n=if($a.Count -gt 1){[int]$a[1]}else{20}; Get-Content -LiteralPath $a[0] -TotalCount $n"),
            ("encoding-detect", "编码探测", "读取文件前若干字节判断编码；参数为文件路径", "$p='{{arg}}'; $b=[IO.File]::ReadAllBytes($p)[0..3]; ($b | ForEach-Object { $_.ToString('X2') }) -join ' '"),
        ],
    },
    PluginPreset {
        id: "preset-archive", name: "压缩解压", cat: "文件",
        desc: "打包目录、解压压缩包",
        tools: &[
            ("zip-dir", "打包目录", "把目录打成 zip；参数为 源目录|目标zip", "$a='{{arg}}' -split '\\|'; Compress-Archive -LiteralPath $a[0] -DestinationPath $a[1] -Force; \\\"已生成 $($a[1])\\\""),
            ("unzip", "解压", "解压 zip；参数为 压缩包|目标目录", "$a='{{arg}}' -split '\\|'; Expand-Archive -LiteralPath $a[0] -DestinationPath $a[1] -Force; \\\"已解压到 $($a[1])\\\""),
        ],
    },
];

fn preset_installed(id: &str) -> Option<PluginDetailLite> {
    let dir = std::path::Path::new(&plugins::plugins_dir()).join(id);
    let raw = std::fs::read_to_string(dir.join("plugin.json")).ok()?;
    let m: plugins::PluginManifest = serde_json::from_str(&raw).ok()?;
    Some(PluginDetailLite { installed: true, enabled: m.enabled })
}

struct PluginDetailLite {
    installed: bool,
    enabled: bool,
}

/// 列出插件预置库（含是否已安装/已启用）。
#[tauri::command]
pub(crate) fn plugin_catalog() -> Result<Vec<PluginCatalogEntry>, String> {
    let _ = plugins::ensure_dir();
    Ok(PLUGIN_PRESETS
        .iter()
        .map(|p| {
            let st = preset_installed(p.id);
            PluginCatalogEntry {
                id: p.id.to_string(),
                name: p.name.to_string(),
                description: p.desc.to_string(),
                category: p.cat.to_string(),
                tool_count: p.tools.len(),
                installed: st.is_some(),
                enabled: st.map(|s| s.installed && s.enabled).unwrap_or(false),
            }
        })
        .collect())
}

/// 勾选/取消勾选一个插件预置。
/// 勾选 → 写入 `app/data/plugins/<id>/plugin.json`；取消 → 只把 enabled 改 false（**不删目录**）。
#[tauri::command]
pub(crate) fn plugin_set_catalog(id: String, enabled: bool) -> Result<String, String> {
    let Some(p) = PLUGIN_PRESETS.iter().find(|p| p.id == id) else {
        return Err(format!("预置库里没有「{id}」"));
    };
    let _ = plugins::ensure_dir()?;
    let dir = std::path::Path::new(&plugins::plugins_dir()).join(&id);
    let manifest_path = dir.join("plugin.json");

    if manifest_path.exists() {
        // 已安装：只改 enabled，保留用户可能的自定义
        let raw = std::fs::read_to_string(&manifest_path).map_err(|e| format!("读取失败：{e}"))?;
        let mut v: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("解析失败：{e}"))?;
        if let Some(o) = v.as_object_mut() {
            o.insert("enabled".into(), serde_json::Value::Bool(enabled));
        }
        let text = serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败：{e}"))?;
        std::fs::write(&manifest_path, text).map_err(|e| format!("写入失败：{e}"))?;
    } else {
        if !enabled {
            return Ok(format!("「{}」本来就没安装", p.name));
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败：{e}"))?;
        let tools: Vec<serde_json::Value> = p
            .tools
            .iter()
            .map(|(n, t, d, c)| serde_json::json!({ "name": n, "title": t, "description": d, "command": c }))
            .collect();
        let m = serde_json::json!({
            "enabled": true,
            "name": p.id,
            "version": "1.0.0",
            "description": p.desc,
            "skills": [],
            "tools": tools,
        });
        std::fs::write(&manifest_path, serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?)
            .map_err(|e| format!("写入失败：{e}"))?;
    }
    chain_log(&format!("plugins: 预置「{}」→ enabled={enabled}", p.name));
    Ok(format!("「{}」已{}", p.name, if enabled { "启用" } else { "停用" }))
}
/// 删除插件（后端强制：必须是插件根目录的直接子目录且含 plugin.json）。
#[tauri::command]
pub(crate) fn delete_plugin(dir: String) -> Result<(), String> {
    plugins::delete(&dir)?;
    chain_log(&format!("plugins: 已删除 {dir}"));
    Ok(())
}

// ---------- 附件读取 ----------
//
// 设计取舍：**不把二进制塞进对话**，只把"能当文本读的"内容交给模型。
// 二进制文件只报元信息（名字/大小），由模型决定是否用工具去处理。

#[derive(serde::Serialize)]
pub(crate) struct AttachedFile {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    /// 文本内容；二进制或不支持的类型为 `None`
    pub text: Option<String>,
    /// 是否因体积被截断
    pub truncated: bool,
}

/// 单个附件最多读取的字符数（防止一次塞爆提示词）
const ATTACH_CHAR_CAP: usize = 60_000;
/// 单次最多字符数（所有附件合计）
const ATTACH_TOTAL_CAP: usize = 200_000;

/// 读取用户选择的附件（供 Composer 使用）。
#[tauri::command]
pub(crate) fn read_attachments(paths: Vec<String>) -> Result<Vec<AttachedFile>, String> {
    if paths.len() > 20 {
        return Err("一次最多 20 个附件".into());
    }
    let mut out = Vec::new();
    let mut used = 0usize;
    for raw in paths {
        let path = std::path::Path::new(&raw);
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| raw.clone());
        let meta = std::fs::metadata(path).map_err(|e| format!("{name}: {e}"))?;
        if !meta.is_file() {
            continue;
        }
        let bytes = meta.len();
        // 二进制判定：含 NUL 或超过 8MB 一律只报元信息
        let mut was_truncated = false;
        let text = if bytes > 8 * 1024 * 1024 {
            None
        } else {
            match std::fs::read(path) {
                Ok(b) => {
                    if b.iter().take(4096).any(|c| *c == 0) {
                        None
                    } else {
                        let s = decode_bytes(&b);
                        let room = ATTACH_CHAR_CAP.min(ATTACH_TOTAL_CAP.saturating_sub(used));
                        was_truncated = s.chars().count() > room;
                        let clipped: String = s.chars().take(room).collect();
                        used += clipped.chars().count();
                        Some(clipped)
                    }
                }
                Err(_) => None,
            }
        };
        out.push(AttachedFile {
            name,
            path: raw,
            bytes,
            truncated: was_truncated,
            text,
        });
    }
    chain_log(&format!("attach: 读取 {} 个附件", out.len()));
    Ok(out)
}
// ---------- MCP 工具市场（策划案 2.0 · P1.6）----------

#[derive(serde::Serialize)]
pub(crate) struct McpToolInfo {
    pub name: String,
    pub description: String,
}

#[derive(serde::Serialize)]
pub(crate) struct McpServerStatus {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub tools: Vec<McpToolInfo>,
    /// 连接/列工具失败原因；`None` 表示正常
    pub error: Option<String>,
}

// ---------- MCP 预置库（策划案 2.0 · P1.6，对标豆包桌面端的"勾选即用"）----------

#[derive(serde::Serialize)]
pub(crate) struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    /// 组装后的完整命令（已把 {{DIR}} 换成实际目录），供界面展示
    pub command: String,
    /// 依赖的外部条件（如"需要 GITHUB_TOKEN"）；空 = 开箱即用
    pub needs: Vec<String>,
    /// 界面上额外的说明
    pub note: String,
    pub installed: bool,
    pub enabled: bool,
}

/// 预置库定义（内置，无需联网；也可在界面里勾选后自行改 config）。
struct Preset {
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    cat: &'static str,
    /// 命令与参数；`{{DIR}}` 会被替换成当前工作目录
    args: &'static [&'static str],
    /// 需要用户额外准备的东西（空 = 开箱即用）
    needs: &'static [&'static str],
    note: &'static str,
}

const PRESETS: &[Preset] = &[
    Preset { id: "memory", name: "知识记忆", cat: "记忆",
        desc: "知识图谱式长期记忆，可跨会话记住信息", args: &["-y","@modelcontextprotocol/server-memory@1.0.0"],
        needs: &[], note: "数据存在 server 自己的内存/文件里（cwd 由 config 指定）" },
    Preset { id: "sequential-thinking", name: "结构化思考", cat: "推理",
        desc: "把复杂问题拆成可回溯的思考步骤", args: &["-y","@modelcontextprotocol/server-sequential-thinking@1.0.0"],
        needs: &[], note: "" },
    Preset { id: "github", name: "GitHub", cat: "代码",
        desc: "仓库、Issue、PR 等 GitHub 操作", args: &["-y","@modelcontextprotocol/server-github@2025.4.8"],
        needs: &["GITHUB_PERSONAL_ACCESS_TOKEN"], note: "勾选后请到 config\\mcp.json 的 env 里填 token" },
    Preset { id: "brave-search", name: "Brave 搜索", cat: "搜索",
        desc: "通过 Brave Search API 联网检索", args: &["-y","@modelcontextprotocol/server-brave-search@0.6.2"],
        needs: &["BRAVE_API_KEY"], note: "勾选后请到 config\\mcp.json 的 env 里填 API Key" },
];

fn current_dir_for_preset(app: &tauri::AppHandle) -> String {
    app.state::<EngineState>()
        .workdir
        .lock()
        .ok()
        .map(|w| {
            let s = w.trim().to_string();
            if s.is_empty() { config::default_workdir() } else { s }
        })
        .unwrap_or_else(|| config::default_workdir())
}

/// 列出预置库（含"是否已安装/已启用"）。
#[tauri::command]
pub(crate) fn mcp_catalog(app: tauri::AppHandle) -> Result<Vec<CatalogEntry>, String> {
    let dir = current_dir_for_preset(&app);
    let cfg = config::mcp_raw_json();
    let servers = cfg.get("servers").and_then(|s| s.as_object());
    Ok(PRESETS
        .iter()
        .map(|p| {
            let args: Vec<String> = p.args.iter().map(|a| a.replace("{{DIR}}", &dir)).collect();
            let existing = servers.and_then(|m| m.get(p.id));
            let installed = existing.is_some();
            let enabled = existing
                .and_then(|e| e.get("enabled"))
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            CatalogEntry {
                id: p.id.to_string(),
                name: p.name.to_string(),
                description: p.desc.to_string(),
                category: p.cat.to_string(),
                command: format!("npx {}", args.join(" ")),
                needs: p.needs.iter().map(|s| s.to_string()).collect(),
                note: p.note.to_string(),
                installed,
                enabled,
            }
        })
        .collect())
}

/// 勾选/取消勾选一个预置 server。
///
/// 勾选 → 写入 config/mcp.json（enabled=true）；取消 → 只把 enabled 改成 false，
/// **不删除**该条（保留用户可能填过的 token 等）。
#[tauri::command]
pub(crate) fn mcp_set_catalog(app: tauri::AppHandle, id: String, enabled: bool) -> Result<String, String> {
    let Some(p) = PRESETS.iter().find(|p| p.id == id) else {
        return Err(format!("预置库里没有「{id}」"));
    };
    let dir = current_dir_for_preset(&app);
    let mut v = config::mcp_raw_json();
    let servers = v
        .as_object_mut()
        .ok_or("mcp.json 顶层不是对象")?
        .entry("servers")
        .or_insert_with(|| serde_json::json!({}));
    let map = servers.as_object_mut().ok_or("servers 不是对象")?;

    match map.get_mut(&id) {
        Some(existing) => {
            // 已存在：只改 enabled，保留用户填的 env / args；
            // 但启用前必须再查一次 command 白名单（防止旧配置/外部写入塞进任意可执行文件）。
            let cmd = existing
                .get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            let allowed = cmd.eq_ignore_ascii_case("npx") || cmd.eq_ignore_ascii_case("node");
            if enabled && !allowed {
                return Err(format!(
                    "「{}」的 command「{cmd}」不在 npx/node 白名单，不能从预置库启用",
                    p.name
                ));
            }
            if let Some(o) = existing.as_object_mut() {
                o.insert("enabled".into(), serde_json::Value::Bool(enabled));
            }
        }
        None => {
            if !enabled {
                return Ok(format!("「{}」本来就没启用", p.name));
            }
            let args: Vec<String> = p.args.iter().map(|a| a.replace("{{DIR}}", &dir)).collect();
            map.insert(
                id.clone(),
                serde_json::json!({
                    "enabled": true,
                    "command": "npx",
                    "args": args,
                    "env": {},
                    "cwd": "{{DATA_DIR}}\\workspace",
                }),
            );
            if !p.needs.is_empty() {
                chain_log(&format!(
                    "mcp: 已启用「{}」，但它需要 {} —— 请到 config/mcp.json 的 env 里填",
                    p.name,
                    p.needs.join(", ")
                ));
            }
        }
    }
    config::save_mcp_json(&v)?;
    // 显式 UI 写回 → 采用新 MCP 配置并重建桥接（旧子进程按旧配置运行，必须丢弃）。
    config::refresh_mcp_snapshot();
    reset_mcp_bridge(&app);
    chain_log(&format!("mcp: 预置「{}」→ enabled={enabled}", p.name));
    Ok(format!("「{}」已{}", p.name, if enabled { "启用" } else { "停用" }))
}
/// 添加或更新一个 MCP server（写入 `config/mcp.json`）。
///
/// 写入后立即刷新安全快照并重建 MCP 桥接；提示词里的工具清单需要重新注入（新会话）才会更新。
#[tauri::command]
pub(crate) fn mcp_save_server(
    app: tauri::AppHandle,
    name: String,
    command: String,
    args: Vec<String>,
    env: std::collections::HashMap<String, String>,
    enabled: bool,
) -> Result<String, String> {
    let name = name.trim();
    let command = command.trim();
    if name.is_empty() || command.is_empty() {
        return Err("server 名与 command 都不能为空".into());
    }
    // 最小启动面：UI 只允许 npx/node；其他命令强制停用，需要手动改 config 并确认安全配置。
    let allowed = command.eq_ignore_ascii_case("npx") || command.eq_ignore_ascii_case("node");
    let effective_enabled = enabled && allowed;

    let mut v = config::mcp_raw_json();
    let servers = v
        .as_object_mut()
        .ok_or("mcp.json 顶层不是对象")?
        .entry("servers")
        .or_insert_with(|| serde_json::json!({}));
    let map = servers.as_object_mut().ok_or("servers 不是对象")?;
    let env_obj: serde_json::Map<String, serde_json::Value> = env
        .into_iter()
        .map(|(k, val)| (k, serde_json::Value::String(val)))
        .collect();
    // 保留已有的 cwd / callTimeoutSec / handshakeTimeoutSec 等字段。
    let mut entry = map
        .get(name)
        .cloned()
        .unwrap_or_else(|| serde_json::json!({ "cwd": "{{DATA_DIR}}\\workspace" }));
    if let Some(o) = entry.as_object_mut() {
        o.insert("enabled".into(), serde_json::Value::Bool(effective_enabled));
        o.insert("command".into(), serde_json::Value::String(command.to_string()));
        o.insert("args".into(), serde_json::json!(args));
        o.insert("env".into(), serde_json::Value::Object(env_obj));
    }
    map.insert(name.to_string(), entry);
    config::save_mcp_json(&v)?;
    config::refresh_mcp_snapshot();
    reset_mcp_bridge(&app);
    chain_log(&format!(
        "mcp: 已保存 server「{name}」（enabled={effective_enabled}，command_allowed={allowed}）"
    ));
    let note = if allowed {
        ""
    } else {
        "；command 不是 npx/node，已强制停用（需手动改 config 并确认安全配置后启用）"
    };
    Ok(format!("已保存「{name}」{note}"))
}

/// 删除一个 MCP server。
#[tauri::command]
pub(crate) fn mcp_remove_server(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let mut v = config::mcp_raw_json();
    let Some(map) = v.get_mut("servers").and_then(|s| s.as_object_mut()) else {
        return Err("mcp.json 里没有 servers".into());
    };
    if map.remove(&name).is_none() {
        return Err(format!("没有名为「{name}」的 server"));
    }
    config::save_mcp_json(&v)?;
    config::refresh_mcp_snapshot();
    reset_mcp_bridge(&app);
    chain_log(&format!("mcp: 已删除 server「{name}」"));
    Ok(format!("已删除「{name}」"))
}

/// 从外界导入：接受 Claude Desktop 风格 `{"mcpServers":{...}}`
/// 或本项目的 `{"servers":{...}}`，也接受单个 `{"name":..,"command":..}`。
///
/// 返回导入的 server 名列表。**只合并，不清空已有配置。**
#[tauri::command]
pub(crate) fn mcp_import(app: tauri::AppHandle, text: String) -> Result<Vec<String>, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(text.trim()).map_err(|e| format!("不是合法 JSON：{e}"))?;

    // 三种形态：{mcpServers}、{servers}、单个 server 对象
    let incoming: serde_json::Map<String, serde_json::Value> =
        if let Some(m) = parsed.get("mcpServers").and_then(|x| x.as_object()) {
            m.clone()
        } else if let Some(m) = parsed.get("servers").and_then(|x| x.as_object()) {
            m.clone()
        } else if parsed.get("command").is_some() {
            let n = parsed
                .get("name")
                .and_then(|x| x.as_str())
                .ok_or("单个 server 对象必须是 {\"name\":..,\"command\":..}")?;
            let mut m = serde_json::Map::new();
            m.insert(n.to_string(), parsed.clone());
            m
        } else {
            return Err("无法识别的 JSON：需要 {\"mcpServers\":{…}}、{\"servers\":{…}} 或单个 server 对象".into());
        };

    let mut v = config::mcp_raw_json();
    let servers = v
        .as_object_mut()
        .ok_or("mcp.json 顶层不是对象")?
        .entry("servers")
        .or_insert_with(|| serde_json::json!({}));
    let map = servers.as_object_mut().ok_or("servers 不是对象")?;

    let mut names = Vec::new();
    for (name, cfg) in incoming {
        // 缺 command 的条目跳过（不猜）
        if cfg.get("command").and_then(|c| c.as_str()).unwrap_or("").trim().is_empty() {
            chain_log(&format!("mcp: 导入时跳过「{name}」（缺 command）"));
            continue;
        }
        let mut obj = cfg.clone();
        let cmd = cfg.get("command").and_then(|c| c.as_str()).unwrap_or("");
        let allowed = cmd.eq_ignore_ascii_case("npx") || cmd.eq_ignore_ascii_case("node");
        // 导入的条目默认启用（用户显式导入的意图就是"要用"）；
        // 非 npx/node 的命令强制停用，避免一次导入就启动任意可执行文件。
        if let Some(o) = obj.as_object_mut() {
            if allowed {
                o.entry("enabled").or_insert(serde_json::Value::Bool(true));
            } else {
                o.insert("enabled".into(), serde_json::Value::Bool(false));
                chain_log(&format!(
                    "mcp: 导入「{name}」的 command「{cmd}」不在 npx/node 白名单，已强制停用"
                ));
            }
        }
        map.insert(name.clone(), obj);
        names.push(name);
    }
    if names.is_empty() {
        return Err("没有可导入的条目（都缺 command）".into());
    }
    config::save_mcp_json(&v)?;
    config::refresh_mcp_snapshot();
    reset_mcp_bridge(&app);
    chain_log(&format!("mcp: 导入 {} 个 server：{}", names.len(), names.join(", ")));
    Ok(names)
}
/// 列出已配置的 MCP server 及其工具（供「MCP」页展示）。
///
/// 复用 `tool_list_text`（含连接自愈），再把渲染好的 bullet 文本解析回结构。
/// 未配置任何 server → 返回空数组（前端展示空态）。
#[tauri::command]
pub(crate) fn mcp_status(app: tauri::AppHandle) -> Result<Vec<McpServerStatus>, String> {
    let servers = config::mcp_servers();
    if servers.is_empty() {
        return Ok(Vec::new());
    }
    if mcp_bridge(&app).is_none() {
        return Err("MCP 桥接建立失败（详见 chain_test.log）".into());
    }

    let mut out = Vec::new();
    for s in servers {
        let deadline = Duration::from_secs(s.handshake_timeout_sec.min(20));
        let mut attempt = match mcp_bridge(&app) {
            Some(b) => b.tool_list_text(&s.name, deadline),
            None => Err("桥接不可用".to_string()),
        };
        if attempt.is_err() {
            // 与提示词注入同一套自愈策略：连接可能已死，重建后重试一次
            reset_mcp_bridge(&app);
            if let Some(b2) = mcp_bridge(&app) {
                attempt = b2.tool_list_text(&s.name, deadline);
            }
        }
        let mut tools = Vec::new();
        let mut error = None;
        match attempt {
            Ok(text) => {
                for line in text.lines() {
                    let line = line.trim();
                    let Some(rest) = line.strip_prefix("- `") else { continue };
                    let Some(end) = rest.find('`') else { continue };
                    let full = &rest[..end]; // mcp:<server>.<tool>
                    let tool = full.rsplit('.').next().unwrap_or(full).to_string();
                    let desc = rest[end + 1..]
                        .trim_start_matches('：')
                        .trim()
                        .to_string();
                    tools.push(McpToolInfo { name: tool, description: desc });
                }
            }
            Err(e) => error = Some(e),
        }
        out.push(McpServerStatus {
            name: s.name,
            command: s.command,
            args: s.args,
            tools,
            error,
        });
    }
    Ok(out)
}
/// 用系统文件管理器打开插件目录（方便用户放自制插件）。
#[tauri::command]
pub(crate) fn open_plugins_dir() -> Result<(), String> {
    let dir = plugins::ensure_dir()?;
    std::process::Command::new("explorer.exe")
        .arg(&dir)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW：不弹控制台
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开目录失败：{e}"))
}
