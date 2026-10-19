//! 共享状态：网络事件缓冲、回合锁、审批通道。
//!
//! 拆分自原单文件 `lib.rs`（可扩展性分析报告 v2 · S1）。
//! `use super::*` 继承根模块的导入与共享项，避免重复声明。

use super::*;

// ---------- 网络层捕获（CDP Network 事件，替代 DOM 类名抓取的回复流信源） ----------

/// 一次 CDP 网络事件。method 形如 "Network.webSocketFrameReceived"；params 为事件参数 JSON。
#[derive(Clone)]
pub(crate) struct NetEvent {
    pub(crate) ts_ms: u64,
    pub(crate) method: String,
    pub(crate) params: String,
}

/// 网络事件环形缓冲：由 WebView2 事件回调写入，回合轮询线程读取。
/// 上限 4096 条，超限丢弃最旧（防长会话无限增长）。
#[derive(Default)]
pub(crate) struct NetCapture {
    pub(crate) events: Mutex<VecDeque<NetEvent>>,
    pub(crate) enabled: AtomicBool,
}

// ---------- 共享状态 ----------

/// 一条待审批工具调用。
///
/// 除决策通道外，额外保存命令哈希与“强制人工确认”标记：
/// 前者用于 `tool_decision` 写可信审计（不依赖前端回报），
/// 后者为后续原生确认框/命令绑定留接口。
pub(crate) struct ApprovalEntry {
    pub(crate) tx: mpsc::Sender<bool>,
    pub(crate) code_hash: u64,
    pub(crate) forced: bool,
    pub(crate) created_at_ms: u64,
}

#[derive(Default)]
pub(crate) struct EngineState {
    // 回合锁：同一时间只允许一个回合（防并发注入把页面搅乱）
    pub(crate) turn_lock: Mutex<()>,
    // 工作目录（注入提示词时设定）
    pub(crate) workdir: Mutex<String>,
    // 是否已开会话（注入过系统提示词）；②在未开会话时自动先初始化
    pub(crate) session_started: Mutex<bool>,
    // 待审批工具：id -> 决策通道 + 命令哈希/强制标记
    pub(crate) approvals: Mutex<HashMap<u64, ApprovalEntry>>,
    // 网络层捕获（回复流信源）
    pub(crate) net: Arc<NetCapture>,
    // MCP 桥接（懒建；未配置 server 时始终为 None，功能不生效）
    pub(crate) mcp: Mutex<Option<Arc<crate::mcp_bridge::Bridge>>>,
    // 对话通道实例（按 provider 配置键懒惰重建）
    pub(crate) chat: Mutex<Option<crate::chat::ActiveProvider>>,
    // 当前会话 id（SQLite 多会话；未初始化时为 None）
    pub(crate) session_id: Mutex<Option<String>>,
    // 记忆层数据库（懒建）
    pub(crate) db: Mutex<Option<Arc<crate::memory::Db>>>,
    // 正在运行的流程 run id → 取消标记
    pub(crate) workflow_cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct EngineEvent {
    pub(crate) seq: u64,
    pub(crate) ts_ms: u64,
    pub(crate) phase: String,
    pub(crate) detail: String,
}

pub(crate) fn emit_ev(app: &tauri::AppHandle, topic: &str, phase: &str, detail: &str) {
    let ev = EngineEvent {
        seq: SEQ.fetch_add(1, Ordering::Relaxed),
        ts_ms: now_ms(),
        phase: phase.to_string(),
        detail: detail.to_string(),
    };
    let _ = app.emit(topic, ev);
}

// ---------- MCP 桥接（阶段 C） ----------

/// 生产用桥接观察者：把 MCP 事件打进引擎事件流与磁盘审计。
///
/// **同步**实现（`Observer` 契约要求），因此可以直接在桥接线程里调用。
pub(crate) struct TauriObserver(pub(crate) tauri::AppHandle);

impl crate::mcp_bridge::Observer for TauriObserver {
    fn phase(&self, detail: &str) {
        emit_ev(&self.0, "engine://state", "mcp", detail);
    }
    fn audit(&self, ev: &crate::mcp_bridge::AuditEvent) {
        audit_log(&format!(
            "MCP[{}] tool={} {}ms result={} killed={} {}",
            ev.server, ev.tool, ev.elapsed_ms, ev.result, ev.killed, ev.reason
        ));
    }
}

/// 丢弃缓存的桥接实例（用于"连接已死"时强制重建）。
pub(crate) fn reset_mcp_bridge(app: &tauri::AppHandle) {
    let st = app.state::<EngineState>();
    if let Ok(mut g) = st.mcp.lock() {
        if g.take().is_some() {
            chain_log("mcp: 丢弃失效的桥接实例");
        }
    }
}

/// 取（或懒建）全局桥接实例。
///
/// 未配置任何 server → 返回 `None`，调用方应把 MCP 分支整体跳过
/// （与不启用 MCP 时行为逐字一致）。
pub(crate) fn mcp_bridge(app: &tauri::AppHandle) -> Option<Arc<crate::mcp_bridge::Bridge>> {
    let servers = config::mcp_servers();
    if servers.is_empty() {
        return None;
    }
    // 文件工具的根目录必须存在；建不出来就整体禁用（fail-closed）。
    if let Err(e) = config::workspace_dir() {
        chain_log(&format!("mcp: 创建 workspace 失败，MCP 不启用：{e}"));
        return None;
    }
    let st = app.state::<EngineState>();
    let mut guard = match st.mcp.lock() {
        Ok(g) => g,
        Err(e) => {
            chain_log(&format!("mcp: 状态锁失败 {e}"));
            return None;
        }
    };
    if let Some(b) = guard.as_ref() {
        return Some(b.clone());
    }
    let cfgs: Vec<(String, crate::mcp_bridge::ServerCfg)> = servers
        .into_iter()
        .map(|s| {
            let mut c = crate::mcp_bridge::ServerCfg::new(s.command, s.args);
            c.env = s.env;
            // 发布环境默认把 npm 缓存收进数据目录：不依赖 %LOCALAPPDATA% 是否可写，
            // 也避免把缓存散落到用户目录。显式配置了 npm_config_cache 时尊重配置。
            if !c
                .env
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("npm_config_cache"))
            {
                c.env.push((
                    "npm_config_cache".to_string(),
                    format!(r"{}\npm-cache", config::cache_dir()),
                ));
            }
            c.cwd = s.cwd;
            c.call_deadline = Duration::from_secs(s.call_timeout_sec);
            c.handshake_deadline = Duration::from_secs(s.handshake_timeout_sec);
            (s.name, c)
        })
        .collect();
    match crate::mcp_bridge::Bridge::new(cfgs, Arc::new(TauriObserver(app.clone()))) {
        Ok(b) => {
            let b = Arc::new(b);
            *guard = Some(b.clone());
            chain_log("mcp: 桥接实例已建立");
            Some(b)
        }
        Err(e) => {
            chain_log(&format!("mcp: 桥接建立失败 {e}"));
            None
        }
    }
}

/// 生成「MCP 工具」提示词段（阶段 C ④）。
///
/// 未配置 server / 桥接建不起来 / 所有 server 都失败 → 返回空串
/// （**空串时提示词与不启用 MCP 时逐字一致**，零风险增量）。
pub(crate) fn mcp_tool_prompt_block(app: &tauri::AppHandle) -> String {
    let Some(bridge) = mcp_bridge(app) else {
        return String::new();
    };
    let servers = config::mcp_servers();
    let mut body = String::new();
    let mut healed = false;
    for s in &servers {
        let deadline = Duration::from_secs(s.handshake_timeout_sec);
        let mut attempt = bridge.tool_list_text(&s.name, deadline);
        // 自愈：连接可能在空闲期间已死（实测出现过 Transport closed）。
        // 第一次失败就丢弃桥接、重建、重试一次 —— 否则会静默给出空清单，
        // 模型完全不知道有 MCP 工具（那是"看起来正常但其实没生效"的最坏情形）。
        if attempt.is_err() && !healed {
            healed = true;
            chain_log("mcp: 首次列工具失败，重建桥接并重试一次");
            reset_mcp_bridge(app);
            if let Some(b2) = mcp_bridge(app) {
                attempt = b2.tool_list_text(&s.name, deadline);
            }
        }
        match attempt {
            Ok(text) if !text.trim().is_empty() => {
                body.push_str(&text);
                body.push('\n');
            }
            Ok(_) => chain_log(&format!("mcp: server「{}」没有工具", s.name)),
            Err(e) => chain_log(&format!("mcp: server「{}」列工具失败（已跳过）：{e}", s.name)),
        }
    }
    if body.trim().is_empty() {
        return String::new();
    }
    format!(
        "\n\n---\n\n## MCP 工具（外部 server 提供）\n\n\
         这些工具来自用户配置的 MCP server。调用方式与内置工具**完全相同**，\
         工具名形如 `mcp:<server>.<tool>`，参数放在 `arguments` 里：\n\n\
         ```\n{}\n{{\"name\":\"mcp:服务器名.工具名\",\"arguments\":{{ ... }}}}\n{}\n```\n\n\
         所有 MCP 调用都会**经用户审批**，被拒绝时不要重试。\n\n{}",
        crate::tools::JSON_HEAD,
        crate::tools::JSON_TAIL,
        body
    )
}