//! **MCP 桥接层**（策划案 2.0 · P1.1 · 阶段 B）
//!
//! 职责：把**同步的回合循环** 与 **异步的 MCP 客户端** 隔开。
//! 设计文档：`docs/15-mcp-bridge-design.md`（本文所有 §x.y 均指该文档）。
//!
//! # 并发契约（务必阅读）
//!
//! **桥接线程一次只处理一个请求。**
//! 多会话 / 多 server 并发调用时，会话 A 的 MCP 调用会**串行阻塞**会话 B 的 MCP 调用 ——
//! 后者的 `recv_timeout` 会被白白吃掉，直到前一个返回或超时。
//!
//! 当前可接受：回合循环本身串行，P1.1 阶段通常单会话。
//! **多窗口并发时这是第一个会暴露的问题。**
//!
//! 演进路径（上层零改动）：
//! 1. 桥接线程内部把请求**按 server 分发**到各自 client（接口已按 `Job { server, .. }` 设计）；
//! 2. 再进一步：每 server 一个线程 / 换 `multi_thread` runtime。
//! 两种演进都只改本文件内部，`Bridge::call()` 签名与语义不变。
//!
//! # 线程模型
//!
//! ```text
//! 同步侧（可能是 tokio worker，无所谓：本模块不碰任何 async 类型）
//!   Bridge::call() ── mpsc ──▶ 桥接线程（current_thread runtime）
//!        ▲                            │
//!        └──────── mpsc reply ◀───────┘
//! ```
//!
//! - 同步侧只有 `Sender` + `recv_timeout`，**不调用 `block_on`**（§1 C1：会 panic）；
//! - 桥接线程独占一个 `current_thread` runtime 与 server 表。
//!
//! # 安全语义
//!
//! - 每个失败都必须变成**一条明确错误 + 一条审计**，绝不静默；
//! - `cancelled` flag 只能阻止**尚未开始执行**的请求（§4.5）：
//!   外层超时的语义是「桥接子系统失效，无法保证」，**不是**「调用被撤销」；
//! - 清理（`taskkill`）**必须发生在 `reply_tx.send` 之前**（§ B-2）。

// 本模块在**阶段 C**（接入 tools 路由）之前没有调用方，因此 lib 构建会报 dead_code。
// 接入后请**移除**下面这行 —— 它只是过渡期的静音，不是长期豁免。
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Map, Value};

/// 外层 `recv_timeout` 相对内层 deadline 的余量（§4.1）。
pub const OUTER_SLACK: Duration = Duration::from_secs(5);

/// 审计里 `reason` 字段的字符上限（与 `log::prepare_audit_text` 的 1000 对齐）。
pub const REASON_CAP: usize = 1000;

/// stderr 只保留前 64KB 用于审计，但**仍然持续读**（§ 实现细节：不能读满就停）。
pub const STDERR_KEEP: usize = 64 * 1024;

// ---------- 对外通知接口 ----------

/// 桥接层的对外通知（抽成 trait 是为了**单测不需要 Tauri `AppHandle`** ——
/// 单测环境里造不出 AppHandle，而 8 行单测矩阵是硬要求）。
/// **实现必须同步，不得写成 async** —— 桥接线程直接调用它；
/// 引入 async 会把同步边界重新撕开（正是本模块要消除的东西）。
pub trait Observer: Send + Sync + 'static {
    /// 阶段事件（对应 `emit_ev(app, "engine://state", ...)`）
    fn phase(&self, detail: &str);
    /// 审计（五字段 + `killed`）。生产实现会写入 `audit.log`。
    fn audit(&self, ev: &AuditEvent);
}

/// 一条 MCP 审计记录（§5.1 五字段 + 清理结果）。
#[derive(Clone)]
pub struct AuditEvent {
    pub server: String,
    pub tool: String,
    pub elapsed_ms: u128,
    /// `ok` / `error` / `timeout` / `crashed` / `cancelled` / `failed`
    pub result: &'static str,
    pub reason: String,
    pub killed: bool,
}

// ---------- 配置 ----------

/// 一个 MCP server 的启动配置。
#[derive(Clone, Debug)]
pub struct ServerCfg {
    /// 可执行名（如 `npx`）；用 `which_command` 解析（Windows 上能处理 `.cmd` shim）
    pub command: String,
    /// 传给**包装层**（npx）的参数，如 `-y @scope/pkg`
    pub args: Vec<String>,
    /// 环境变量；如 `npm_config_cache` —— A′ 实测本机默认 npm 缓存不可写（EPERM）
    pub env: Vec<(String, String)>,
    /// 可选工作目录
    pub cwd: Option<PathBuf>,
    /// `tools/call` 的默认预算（caller 可覆盖）
    pub call_deadline: Duration,
    /// 握手预算（initialize + tools/list）
    pub handshake_deadline: Duration,
}

impl ServerCfg {
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            command: command.into(),
            args,
            env: Vec::new(),
            cwd: None,
            call_deadline: Duration::from_secs(30),
            handshake_deadline: Duration::from_secs(30),
        }
    }
}

// ---------- 请求 / 回复 ----------

/// 一个请求要做什么（阶段 C 的 ④ 需要"只列工具"这种轻量请求）
#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    /// 调用一个工具
    Call,
    /// 只列工具（用于提示词注入）
    ListTools,
}

struct Job {
    kind: JobKind,
    server: String,
    tool: String,
    /// ★ 定成 `Map` 而非 `Value`，直接喂给 `with_arguments`，避免内部 `as_object().unwrap()`（§ 实现细节）
    args: Map<String, Value>,
    call_deadline: Duration,
    handshake_deadline: Duration,
    /// ★ 外层超时后由同步侧置位；桥接线程在 **dispatch 之前** 检查（§4.5）
    cancelled: Arc<AtomicBool>,
    reply_tx: Sender<Reply>,
}

enum Reply {
    Ok(String),
    Err(String),
}

// ---------- 桥接实例 ----------

/// 同步侧持有的桥接句柄。
///
/// **不持 `AppHandle`**（§ 实现细节）：AppHandle 只由桥接线程持有，
/// 同步侧只持 `Sender` 与共享状态，避免生命周期问题。
pub struct Bridge {
    tx: Option<Sender<Job>>,
    worker: Option<std::thread::JoinHandle<()>>,
    failed: Arc<AtomicBool>,
    /// ★ 存活子进程登记簿：桥接线程是唯一写者，`Drop` 是唯一读者。
    ///
    /// 它不是"共享 server 表"（没有分发逻辑），只是一份**只增只减的 pid 列表**，
    /// 用来兜住"桥接线程卡死、join 超时"时的子进程残留：
    /// Windows 上父进程死了子进程**不会**自动死，所以"进程退出统一清理"在崩溃时失效。
    ///
    /// 锁持有时间必须**极短**（push/retain 一个 pid），**不得在其中做 IO**，
    /// 否则 `Drop` 会卡在等锁上。
    live_pids: Arc<Mutex<Vec<u32>>>,
}

impl Bridge {
    /// 创建一个**独立**的桥接实例（自己的线程 / channel / server 表）。
    ///
    /// 测试里每个用例各建一个，互不干扰 —— 否则 `hang` 用例会串行阻塞其他用例（§3.5）。
    pub fn new(servers: Vec<(String, ServerCfg)>, obs: Arc<dyn Observer>) -> Result<Self, String> {
        Self::new_with_hook(servers, obs, None)
    }

    fn new_with_hook(
        servers: Vec<(String, ServerCfg)>,
        obs: Arc<dyn Observer>,
        #[cfg_attr(not(test), allow(unused_variables))] hook: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<Job>();
        let failed = Arc::new(AtomicBool::new(false));
        let failed2 = failed.clone();
        let live_pids: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
        let live2 = live_pids.clone();

        let worker = std::thread::Builder::new()
            .name("mcp-bridge".into())
            .spawn(move || worker_main(rx, servers, obs, failed2, live2, hook))
            .map_err(|e| format!("启动桥接线程失败：{e}"))?;

        Ok(Self {
            tx: Some(tx),
            worker: Some(worker),
            failed,
            live_pids,
        })
    }

    /// 当前登记在册的存活子进程 pid（**仅供测试**）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn live_pids(&self) -> Vec<u32> {
        self.live_pids.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// 桥接是否已失效（线程崩溃过）。失效后 `call` 立即返回错误，不排队。
    pub fn is_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// 同步调用一个 MCP 工具。
    ///
    /// 等待上限 = `call_deadline + handshake_deadline + OUTER_SLACK`
    /// （重拉时先握手再执行，两段各有预算，见 §4.6）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn call(
        &self,
        server: &str,
        tool: &str,
        args: Map<String, Value>,
    ) -> Result<String, String> {
        self.call_with_deadlines(server, tool, args, None, None)
    }

    /// 取某个 server 的工具清单（渲染成提示词用的 bullet 文本）。
    ///
    /// 与 `call` 走同一条线程与超时机制，只是 `kind = ListTools`。
    pub fn tool_list_text(&self, server: &str, deadline: Duration) -> Result<String, String> {
        if self.is_failed() {
            return Err("MCP 桥接已失效，请重启应用".into());
        }
        let Some(tx) = self.tx.as_ref() else {
            return Err("MCP 桥接已关闭".into());
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let (reply_tx, reply_rx) = mpsc::channel::<Reply>();
        let job = Job {
            kind: JobKind::ListTools,
            server: server.to_string(),
            tool: String::new(),
            args: Map::new(),
            call_deadline: deadline,
            handshake_deadline: deadline,
            cancelled: cancelled.clone(),
            reply_tx,
        };
        if tx.send(job).is_err() {
            return Err("MCP 桥接线程已退出".into());
        }
        match reply_rx.recv_timeout(deadline + OUTER_SLACK) {
            Ok(Reply::Ok(v)) => Ok(v),
            Ok(Reply::Err(e)) => Err(e),
            Err(RecvTimeoutError::Timeout) => {
                cancelled.store(true, Ordering::Release);
                Err("列工具超时".into())
            }
            Err(RecvTimeoutError::Disconnected) => Err("MCP 桥接线程已崩溃".into()),
        }
    }

    pub fn call_with_deadlines(
        &self,
        server: &str,
        tool: &str,
        args: Map<String, Value>,
        call_deadline: Option<Duration>,
        handshake_deadline: Option<Duration>,
    ) -> Result<String, String> {
        if self.is_failed() {
            return Err("MCP 桥接已失效，请重启应用".into());
        }
        let Some(tx) = self.tx.as_ref() else {
            return Err("MCP 桥接已关闭".into());
        };

        let cancelled = Arc::new(AtomicBool::new(false));
        let (reply_tx, reply_rx) = mpsc::channel::<Reply>();

        let (cd, hd) = {
            // 未显式给出时用保守默认；caller 正常路径应显式传（B-7：bridge 不查配置表）
            (call_deadline.unwrap_or(Duration::from_secs(30)),
             handshake_deadline.unwrap_or(Duration::from_secs(30)))
        };

        let job = Job {
            kind: JobKind::Call,
            server: server.to_string(),
            tool: tool.to_string(),
            args,
            call_deadline: cd,
            handshake_deadline: hd,
            cancelled: cancelled.clone(),
            reply_tx,
        };
        if tx.send(job).is_err() {
            return Err("MCP 桥接线程已退出".into());
        }

        // ★ 外层兜底：内层超时的余量。内层负责真正的打断，外层负责"桥接线程自己卡死"（§4.1）
        let outer = cd + hd + OUTER_SLACK;
        match reply_rx.recv_timeout(outer) {
            Ok(Reply::Ok(v)) => Ok(v),
            Ok(Reply::Err(e)) => Err(e),
            Err(RecvTimeoutError::Timeout) => {
                // ★ 必须在放弃之前置位取消标记，否则桥接线程恢复后会照常执行（§4.5）
                cancelled.store(true, Ordering::Release);
                Err(format!(
                    "MCP 桥接失效：{}s 内无响应（子系统不可用，无法保证该调用未被执行）",
                    outer.as_secs()
                ))
            }
            Err(RecvTimeoutError::Disconnected) => {
                cancelled.store(true, Ordering::Release);
                Err("MCP 桥接线程已崩溃".into())
            }
        }
    }
}

impl Drop for Bridge {
    /// shutdown 顺序（§3.5）：
    /// 1. drop `tx` → 桥接线程 `rx.recv()` 返回 `Disconnected` → 退出 loop；
    /// 2. `join` 带超时（线程可能卡在 `rt.block_on` 里）；
    /// 3. 子进程由**桥接线程自己**在退出路径上清理（它持有 server 表）。
    ///
    /// 若 join 超时（线程卡死），子进程可能残留 —— 这是**已知缺口**（§ 已知缺口 1），
    /// 由进程退出时统一清理。刻意**不**为此引入"pid 共享表"。
    fn drop(&mut self) {
        self.tx.take(); // 断开
        if let Some(h) = self.worker.take() {
            let (done_tx, done_rx) = mpsc::channel::<()>();
            std::thread::spawn(move || {
                let _ = h.join();
                let _ = done_tx.send(());
            });
            if done_rx.recv_timeout(Duration::from_secs(5)).is_err() {
                crate::log::chain_log("mcp-bridge: Drop 等待线程超时，改用登记簿兜底清理子进程");
                // ★ 先把 pid 取出来**释放锁**，再去做 IO（taskkill），避免持锁
                let survivors: Vec<u32> = self.live_pids.lock().map(|v| v.clone()).unwrap_or_default();
                if let Ok(mut g) = self.live_pids.lock() {
                    g.clear();
                }
                for pid in survivors {
                    match crate::exec::kill_process_tree(pid) {
                        Ok(()) => crate::log::chain_log(&format!("mcp-bridge: 兜底已终止 pid={pid}")),
                        Err(e) => crate::log::chain_log(&format!("mcp-bridge: 兜底终止 pid={pid} 失败：{e}")),
                    }
                }
            }
        }
    }
}

// ---------- 桥接线程 ----------

fn worker_main(
    rx: mpsc::Receiver<Job>,
    servers: Vec<(String, ServerCfg)>,
    obs: Arc<dyn Observer>,
    failed: Arc<AtomicBool>,
    live_pids: Arc<Mutex<Vec<u32>>>,
    #[cfg_attr(not(test), allow(unused_variables))] hook: Option<Arc<dyn Fn() + Send + Sync>>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            crate::log::chain_log(&format!("mcp-bridge: 建 runtime 失败 {e}"));
            failed.store(true, Ordering::Release);
            return;
        }
    };

    let mut table: HashMap<String, ServerEntry> = HashMap::new();
    for (name, cfg) in servers {
        table.insert(name, ServerEntry::new(cfg, live_pids.clone()));
    }

    while let Ok(job) = rx.recv() {
        // ★ cancellation flag：dispatch 之前检查（§4.5）—— 阻止"未报告的成功"
        if job.cancelled.load(Ordering::Acquire) {
            obs.audit(&AuditEvent {
                server: job.server.clone(),
                tool: job.tool.clone(),
                elapsed_ms: 0,
                result: "cancelled",
                reason: "cancelled before dispatch（调用方已放弃）".into(),
                killed: false,
            });
            // send 失败是正常路径（§ B-2）：静默忽略，继续下一个
            let _ = job.reply_tx.send(Reply::Err("请求已取消（未执行）".into()));
            continue;
        }

        // ★ catch_unwind 包在 rt.block_on 的**外层**（§ 实现细节）
        let hook_ref = hook.clone();
        let res = catch_unwind(AssertUnwindSafe(|| {
            if let Some(h) = hook_ref {
                h(); // 测试注入：模拟"桥接线程在 dispatch 前卡死"
            }
            rt.block_on(dispatch(&job, &mut table, &obs))
        }));

        match res {
            Ok(text) => {
                // 清理已在 dispatch 内部、send 之前完成
                let _ = job.reply_tx.send(text);
            }
            Err(_) => {
                // runtime 已被 panic 破坏 → 标记失效并**退出 loop**，不尝试恢复（§3.4）
                crate::log::chain_log("mcp-bridge: 桥接线程 panic，标记失效并退出");
                obs.audit(&AuditEvent {
                    server: job.server.clone(),
                    tool: job.tool.clone(),
                    elapsed_ms: 0,
                    result: "failed",
                    reason: "bridge thread panicked".into(),
                    killed: false,
                });
                failed.store(true, Ordering::Release);
                let _ = job.reply_tx.send(Reply::Err("MCP 桥接线程异常".into()));
                break;
            }
        }
    }

    // 退出前清理所有子进程（这里才有 server 表）
    for (_, e) in table.iter_mut() {
        e.shutdown();
    }
}

struct ServerEntry {
    cfg: ServerCfg,
    client: Option<RunningService<RoleClient, ()>>,
    pid: Option<u32>,
    /// 只保留前 `STDERR_KEEP`，但 drain 任务会**持续读**（§ 实现细节）
    stderr_buf: Arc<std::sync::Mutex<Vec<u8>>>,
    /// 是否处于"需要重新握手"的状态
    needs_handshake: bool,
    /// 是否曾经连接成功过（用于区分「首次握手」与「崩溃后重拉」的审计措辞）
    ever_connected: bool,
    /// 存活 pid 登记簿（与 `Bridge` 共享；**只做 push/retain，不做 IO**）
    live_pids: Arc<Mutex<Vec<u32>>>,
}

impl ServerEntry {
    fn new(cfg: ServerCfg, live_pids: Arc<Mutex<Vec<u32>>>) -> Self {
        Self {
            cfg,
            client: None,
            pid: None,
            stderr_buf: Arc::new(std::sync::Mutex::new(Vec::new())),
            needs_handshake: true,
            ever_connected: false,
            live_pids,
        }
    }

    /// 登记一个存活 pid（锁内只做 push，微秒级）。
    fn register(&self, pid: u32) {
        if let Ok(mut g) = self.live_pids.lock() {
            if !g.contains(&pid) {
                g.push(pid);
            }
        }
    }

    /// 注销一个 pid（锁内只做 retain，微秒级）。
    fn unregister(&self, pid: u32) {
        if let Ok(mut g) = self.live_pids.lock() {
            g.retain(|p| *p != pid);
        }
    }

    /// 清理子进程。`pid` 缺失时**不按名字杀**，只 drop client 靠 stdio EOF（§5.4）。
    fn shutdown(&mut self) -> bool {
        self.client = None; // drop → 关闭 stdin/stdout
        let Some(pid) = self.pid.take() else {
            return false; // killed = false
        };
        let ok = crate::exec::kill_process_tree(pid).is_ok();
        // 无论 taskkill 是否报成功，进程都已不该再被当作存活者；从登记簿移除
        self.unregister(pid);
        ok
    }

    fn stderr_text(&self) -> String {
        let b = self.stderr_buf.lock().map(|v| v.clone()).unwrap_or_default();
        String::from_utf8_lossy(&b).to_string()
    }
}

/// 一次 job 的完整处理：按需握手 → 执行 → 清理 → 组装回复。
///
/// **清理必须在返回前完成**（send 在 caller 侧，失败无所谓）—— § B-2。
async fn dispatch(job: &Job, table: &mut HashMap<String, ServerEntry>, obs: &Arc<dyn Observer>) -> Reply {
    let started = Instant::now();
    let audit = |result: &'static str, reason: String, killed: bool| AuditEvent {
        server: job.server.clone(),
        tool: job.tool.clone(),
        elapsed_ms: started.elapsed().as_millis(),
        result,
        reason,
        killed,
    };

    let Some(entry) = table.get_mut(&job.server) else {
        obs.audit(&audit("error", format!("未注册的 MCP server：{}", job.server), false));
        return Reply::Err(format!("未注册的 MCP server：{}", job.server));
    };

    // ---------- 1. 需要时握手（懒启动 / 崩溃后重拉）----------
    // ★ 第二处 cancellation 检查（评审建议三处：入队后 / 握手前 / call_tool 前）。
    // 检查是幂等的，多查一次成本为零；握手本身可能耗时几百毫秒，是独立的放弃窗口。
    if job.cancelled.load(Ordering::Acquire) {
        obs.audit(&audit("cancelled", "cancelled before handshake（调用方已放弃）".into(), false));
        return Reply::Err("请求已取消（未执行）".into());
    }
    let mut restart_note = String::new();
    // ★ 必须在 handshake 之前读：handshake() 成功时会把 ever_connected 置 true
    let was_connected = entry.ever_connected;
    if entry.needs_handshake || entry.client.is_none() {
        obs.phase(&format!("MCP 连接 {}（含握手）", job.server));
        match handshake(entry, job.handshake_deadline).await {
            Ok(ms) => {
                // 首次连接与崩溃重拉的措辞要分开，否则审计里追溯不出"这次其实重拉了"
                restart_note = if was_connected {
                    format!("lazy restart: initialize ok ({ms}ms); ")
                } else {
                    format!("initial handshake: ok ({ms}ms); ")
                };
                crate::log::chain_log(&format!(
                    "mcp-bridge: {} 握手完成 {ms}ms pid={:?}",
                    job.server, entry.pid
                ));
            }
            Err(e) => {
                let killed = entry.shutdown();
                let reason = truncate_reason(&format!("{restart_note}handshake failed: {e}\n{}", entry.stderr_text()));
                obs.audit(&audit("error", reason, killed));
                return Reply::Err(format!("MCP server「{}」握手失败：{e}", job.server));
            }
        }
    }

    // ---------- 1.5 只列工具（阶段 C ④：提示词注入用）----------
    if job.kind == JobKind::ListTools {
        let lines = {
            let client = entry.client.as_ref().expect("握手后必有 client");
            match tokio::time::timeout(job.call_deadline, client.list_all_tools()).await {
                Ok(Ok(tools)) => tools
                    .iter()
                    .map(|t| {
                        let d = t.description.clone().unwrap_or_default();
                        let d: String = d.chars().take(160).collect();
                        if d.is_empty() {
                            format!("- `mcp:{}.{}`", job.server, t.name)
                        } else {
                            format!("- `mcp:{}.{}`：{}", job.server, t.name, d.replace(char::from(10), " "))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                Ok(Err(e)) => {
                    let killed = entry.shutdown();
                    entry.needs_handshake = true;
                    obs.audit(&audit("error", format!("tools/list 失败：{e}"), killed));
                    return Reply::Err(format!("列工具失败：{e}"));
                }
                Err(_) => {
                    let killed = entry.shutdown();
                    entry.needs_handshake = true;
                    obs.audit(&audit("timeout", "tools/list timeout".into(), killed));
                    return Reply::Err("列工具超时".into());
                }
            }
        };
        obs.audit(&audit("ok", format!("{restart_note}tools/list"), false));
        return Reply::Ok(lines);
    }

    // ---------- 2. 执行 tool call ----------
    // ★ 这里是 cancellation 的**关键检查点**：握手可能已经花掉几百毫秒，
    // 若调用方在此期间放弃，就必须在这里拦住 —— 否则就是"未报告的成功"（§4.5）。
    if job.cancelled.load(Ordering::Acquire) {
        // ★ 刻意**不** shutdown：取消的是"这次调用"，不是 server。
        // 杀掉连接只会让下次调用再吃一次 30s 冷启动（评审指出的过度清理）。
        // 只有内层 deadline 超时才 taskkill（那时 server 可能已卡死）。
        obs.audit(&audit(
            "cancelled",
            format!("{restart_note}cancelled before tool call（调用方已放弃，连接保留）"),
            false,
        ));
        return Reply::Err("请求已取消（未执行）".into());
    }
    let params = CallToolRequestParams::new(job.tool.clone()).with_arguments(job.args.clone());
    let call = {
        let client = entry.client.as_ref().expect("握手后必有 client");
        tokio::time::timeout(job.call_deadline, client.call_tool(params)).await
    };

    match call {
        Ok(Ok(res)) => {
            let text = render_content(&res);
            obs.audit(&audit("ok", restart_note, false));
            Reply::Ok(text)
        }
        Ok(Err(e)) => {
            // 传输层错误（含 server 崩溃 → Transport closed）→ 标记需重拉并清理
            let killed = entry.shutdown();
            entry.needs_handshake = true;
            let reason = truncate_reason(&format!("{restart_note}call error: {e}\n{}", entry.stderr_text()));
            obs.audit(&audit("crashed", reason, killed));
            Reply::Err(format!("MCP 调用失败（{}）：{e}", job.server))
        }
        Err(_) => {
            // ★ 内层超时：先清理再回复（§ B-2）
            let killed = entry.shutdown();
            entry.needs_handshake = true;
            let reason = truncate_reason(&format!(
                "{restart_note}call timeout after {:?}",
                job.call_deadline
            ));
            obs.audit(&audit("timeout", reason, killed));
            Reply::Err(format!(
                "MCP 调用超时（{}s，server={}，子进程已{}）",
                job.call_deadline.as_secs(),
                job.server,
                if killed { "终止" } else { "尝试关闭" }
            ))
        }
    }
}

/// 建连 + 握手 + 列表（只为确认可用）。返回耗时毫秒。
async fn handshake(entry: &mut ServerEntry, deadline: Duration) -> Result<u128, String> {
    let t0 = Instant::now();
    let fut = async {
        // which_command：Windows 上解析 npx.cmd 这类 shim（阶段 A′3 实测必需）
        let mut cmd = rmcp::transport::which_command(&entry.cfg.command)
            .map_err(|e| format!("找不到可执行文件 {}：{e}", entry.cfg.command))?;

        let args = entry.cfg.args.clone();
        // ★ 不继承父进程完整环境：只回填白名单 + 用户显式配置。
        //   否则任何 ambient token（GITHUB_TOKEN / OPENAI_API_KEY / NODE_OPTIONS）都会进子进程。
        let env = crate::security::filtered_current_env(&entry.cfg.env);
        let cwd = entry.cfg.cwd.clone();
        cmd = cmd.configure(move |c| {
            c.env_clear();
            for a in &args {
                c.arg(a);
            }
            for (k, v) in &env {
                c.env(k, v);
            }
            if let Some(d) = &cwd {
                c.current_dir(d);
            }
            c.kill_on_drop(true);
            // ★ CREATE_NO_WINDOW：否则 npx/node 子进程会弹出**可见的控制台窗口**
            // （与 exec.rs / commands.rs 同一个问题；用户实测遇到过并被手动关掉，
            //  关窗口等于杀掉 MCP server）。
            #[cfg(windows)]
            c.creation_flags(0x0800_0000);
        });

        let (transport, stderr) = TokioChildProcess::builder(cmd)
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn 失败：{e}"))?;

        // ★ 先记 pid，再交给 serve()（serve 会消费 transport）（§ pid 方案）
        entry.pid = transport.id();
        if let Some(pid) = entry.pid {
            entry.register(pid);
            // ★ 发布兜底：把子进程加入 kill-on-close Job。
            // 应用崩溃/强杀时 OS 关闭 Job 句柄，npx 及其 node 子进程一并终止，避免孤儿。
            match crate::win_job::assign_kill_on_close(pid) {
                Ok(()) => crate::log::chain_log(&format!(
                    "mcp: pid {pid} 已加入 kill-on-close Job Object"
                )),
                Err(e) => crate::log::chain_log(&format!(
                    "mcp: pid {pid} 未能加入 Job Object（继续运行）：{e}"
                )),
            }
        }

        // ★ stderr 必须**持续 drain**，只保留前 64KB（不能读满就停，否则重新堵管道）
        if let Some(mut err) = stderr {
            let buf = entry.stderr_buf.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut chunk = [0u8; 8192];
                while let Ok(n) = err.read(&mut chunk).await {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut b) = buf.lock() {
                        let room = STDERR_KEEP.saturating_sub(b.len());
                        if room > 0 {
                            b.extend_from_slice(&chunk[..n.min(room)]);
                        }
                    }
                }
            });
        }

        let client = ().serve(transport).await.map_err(|e| format!("initialize 失败：{e}"))?;
        // tools/list 一次确认可用（也预热了 client 内部状态）
        client.list_all_tools().await.map_err(|e| format!("tools/list 失败：{e}"))?;
        Ok::<_, String>(client)
    };

    match tokio::time::timeout(deadline, fut).await {
        Ok(Ok(client)) => {
            entry.client = Some(client);
            entry.needs_handshake = false;
            entry.ever_connected = true;
            Ok(t0.elapsed().as_millis())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err(format!("握手超时（>{deadline:?}）")),
    }
}

/// 把 `CallToolResult` 的 content 摊平成文本（模型要吃文本）。
fn render_content(res: &rmcp::model::CallToolResult) -> String {
    let mut out = String::new();
    for c in &res.content {
        if let Some(t) = c.as_text() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&t.text);
        }
    }
    if out.is_empty() {
        out = format!("{:?}", res.content);
    }
    if res.is_error == Some(true) {
        out = format!("[工具报告错误] {out}");
    }
    out
}

// ---------- stderr 截断（§5.2）----------

/// 把 stderr 压成审计用的 `reason`：**优先保留关键行**，再补首尾，且明确标注截断。
///
/// 规则（§5.2）：不能简单"取前 1000 字"，否则会丢掉 `EPERM` 那一行。
pub fn truncate_reason(raw: &str) -> String {
    const KEYWORDS: [&str; 5] = ["EPERM", "EACCES", "ENOENT", "error", "Error"];
    let lines: Vec<&str> = raw.lines().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return String::new();
    }

    let mut picked: Vec<&str> = Vec::new();
    // 1. 关键行优先，最多 5 行
    for l in &lines {
        if picked.len() >= 5 {
            break;
        }
        if KEYWORDS.iter().any(|k| l.contains(k)) && !picked.contains(l) {
            picked.push(l);
        }
    }
    // 2. 补首尾
    let mut head = 0usize;
    while picked.len() < REASON_CAP / 80 && head < lines.len() && head < 3 {
        if !picked.contains(&lines[head]) {
            picked.push(lines[head]);
        }
        head += 1;
    }
    let mut tail = lines.len();
    while picked.len() < REASON_CAP / 80 && tail > lines.len().saturating_sub(3) && tail > 0 {
        tail -= 1;
        if !picked.contains(&lines[tail]) {
            picked.push(lines[tail]);
        }
    }

    let mut s = picked.join(" | ");
    if s.chars().count() > REASON_CAP.saturating_sub(40) {
        s = s.chars().take(REASON_CAP.saturating_sub(40)).collect();
    }
    let omitted = lines.len().saturating_sub(picked.len());
    if omitted > 0 {
        s.push_str(&format!(" | ...(truncated {omitted} lines)"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_reason_keeps_key_lines() {
        // A′ 实测的 EPERM 样本（裁剪版）
        let raw = "Starting default (STDIO) server...\nnpm error code EPERM\nnpm error syscall open\nnpm error path C:\\Users\\x\\npm-cache\\_cacache\\tmp\\abc\nnpm error errno EPERM\nnpm error FetchError: Invalid response body\n  at foo.js:1\n  at bar.js:2\n  at baz.js:3\n  at qux.js:4";
        let r = truncate_reason(raw);
        assert!(r.contains("EPERM"), "必须保留 EPERM 关键行：{r}");
        assert!(r.contains("truncated"), "必须标注截断：{r}");
        assert!(r.chars().count() <= REASON_CAP, "不得超过审计上限：{}", r.chars().count());
    }

    #[test]
    fn truncate_reason_empty_and_short() {
        assert_eq!(truncate_reason(""), "");
        assert_eq!(truncate_reason("   \n\n  "), "");
        let short = truncate_reason("boom");
        assert_eq!(short, "boom");
        assert!(!short.contains("truncated"));
    }

    #[test]
    fn truncate_reason_respects_cap_on_huge_input() {
        let raw: String = (0..5000).map(|i| format!("line {i} error here\n")).collect();
        let r = truncate_reason(&raw);
        assert!(r.chars().count() <= REASON_CAP);
        assert!(r.contains("truncated"));
    }

    #[test]
    fn server_cfg_defaults() {
        let c = ServerCfg::new("npx", vec!["-y".into(), "@x/y".into()]);
        assert_eq!(c.call_deadline, Duration::from_secs(30));
        assert_eq!(c.handshake_deadline, Duration::from_secs(30));
        assert!(c.env.is_empty());
    }
}

// ---------- 集成测试（真跑 fake server，验证"桥接层真的能用"）----------

#[cfg(test)]
mod it {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    /// 方案 A：主工程没有 fake_mcp_server 这个 bin，`CARGO_BIN_EXE_*` 无效（会编译期报错）。
    /// 用 `CARGO_MANIFEST_DIR` 相对定位 spike 工程的产物，缺失时按同 profile 构建。
    /// 用 `OnceLock` 缓存，避免每个用例都检查一次（用户建议）。
    fn fake_server_path() -> PathBuf {
        static P: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        P.get_or_init(|| {
            // 主工程 manifest = app/src-tauri；spike 在其**上两级**（差一级就全错）
            let spike = std::path::absolute(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_spike_mcp"),
            )
            .expect("absolute");
            let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
            let bin = spike.join("target").join(profile).join("fake_mcp_server.exe");
            if !bin.exists() {
                let mut c = std::process::Command::new("cargo");
                c.arg("build")
                    .arg("--manifest-path")
                    .arg(spike.join("Cargo.toml"))
                    .args(["--bin", "fake_mcp_server"]);
                if !cfg!(debug_assertions) {
                    c.arg("--release");
                }
                let st = c.status().expect("无法启动 cargo 构建 fake server");
                assert!(
                    st.success(),
                    "构建 fake server 失败；请先执行：cargo build --manifest-path {}",
                    spike.join("Cargo.toml").display()
                );
            }
            assert!(bin.exists(), "fake server 不存在：{}", bin.display());
            bin
        })
        .clone()
    }

    /// 收集审计事件，供断言。
    #[derive(Default)]
    struct Rec(Mutex<Vec<AuditEvent>>);
    impl Observer for Rec {
        fn phase(&self, _detail: &str) {}
        fn audit(&self, ev: &AuditEvent) {
            self.0.lock().unwrap().push(ev.clone());
        }
    }
    impl Rec {
        fn events(&self) -> Vec<AuditEvent> {
            self.0.lock().unwrap().clone()
        }
        fn last_with(&self, result: &str) -> Option<AuditEvent> {
            self.events().into_iter().filter(|e| e.result == result).next_back()
        }
    }

    fn cfg(mode: &str, extra: Vec<String>) -> ServerCfg {
        let mut args = vec!["--mode".to_string(), mode.to_string()];
        args.extend(extra);
        ServerCfg::new(fake_server_path().to_string_lossy().to_string(), args)
    }

    fn args_hi() -> Map<String, Value> {
        serde_json::json!({ "msg": "hi" }).as_object().cloned().unwrap()
    }

    /// 测试临时文件放在 **target/** 下，不用 `%TEMP%`。
    ///
    /// 原因（实测）：本机 fake server 子进程写 `%TEMP%` / `%LOCALAPPDATA%` 会拿到
    /// `拒绝访问 (os error 5)`，而 pwsh 与 cmd 写同样路径都成功 —— 说明存在进程令牌/ACL 层面的差异。
    /// `target/` 已被 .gitignore 覆盖且确定可写，测试不该依赖这个未定论的差异。
    /// 测试临时文件的位置。
    ///
    /// **实测发现（记录，不掩盖）**：本机上 fake server 子进程写
    /// `%TEMP%`、`%LOCALAPPDATA%`、甚至 `app/src-tauri/target/` 都会拿到
    /// `拒绝访问 (os error 5)`，而写 `_spike_mcp/` 目录却正常 —— 同样是子进程、同样方式。
    /// 说明存在进程令牌/ACL 层面的差异，尚未定论。
    /// 因此测试临时文件放到**已验证可写**的 `_spike_mcp/` 下（该目录已被 .gitignore 覆盖）。
    fn tmp(tag: &str) -> PathBuf {
        let spike = std::path::absolute(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_spike_mcp"),
        )
        .expect("absolute");
        spike.join(format!("mcp_it_{}_{}.txt", tag, std::process::id()))
    }

    /// pid 是否还活着（用 tasklist 的 CSV 输出，避免依赖系统语言）。
    fn pid_alive(pid: u32) -> bool {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output();
        let Ok(o) = out else { return true }; // 查询失败 → 保守认为还活着
        String::from_utf8_lossy(&o.stdout).lines().any(|l| {
            let cols: Vec<&str> = l.split(',').collect();
            cols.len() >= 2 && cols[1].trim_matches('"').trim() == pid.to_string()
        })
    }

    /// ① 正常调用：通道打通，审计五字段齐全。
    #[test]
    fn it1_normal_call_succeeds() {
        let obs = Arc::new(Rec::default());
        let b = Bridge::new(vec![("s".into(), cfg("normal", vec![]))], obs.clone()).expect("建桥");
        let out = b.call("s", "echo", args_hi()).expect("应成功");
        assert!(out.contains("echo:hi"), "应回显参数，实际：{out}");

        let ok = obs.last_with("ok").expect("应有 result=ok 的审计");
        assert_eq!(ok.server, "s");
        assert_eq!(ok.tool, "echo");
        assert!(ok.reason.contains("initial handshake"), "首次连接措辞：{}", ok.reason);
        assert!(!ok.killed);
        println!("it1 ✓ 正常调用：{out}");
    }

    /// ⑦ pid 登记簿：连接建立后在册；被杀后出册。
    #[test]
    fn it7_pid_registry_tracks_lifetime() {
        // (a) 连接建立 → 登记簿恰好 1 个 pid，且就是子进程 self-reported pid
        let obs = Arc::new(Rec::default());
        let b = Bridge::new(vec![("s".into(), cfg("normal", vec![]))], obs.clone()).expect("建桥");
        let out = b.call("s", "echo", args_hi()).expect("应成功");
        let pids = b.live_pids();
        assert_eq!(pids.len(), 1, "连接建立后应恰好登记 1 个 pid，实际 {pids:?}");
        let reported: u32 = out
            .split("pid=")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|s| s.parse().ok())
            .expect("应能从回显里取到 pid");
        assert_eq!(pids[0], reported, "登记簿里的 pid 必须就是子进程本身");
        assert!(pid_alive(pids[0]), "此刻应仍存活");
        drop(b); // Drop 正常路径：线程自行清理

        // (b) 内层超时被杀 → 出册
        let obs2 = Arc::new(Rec::default());
        let b2 = Bridge::new(vec![("s".into(), cfg("hang", vec![]))], obs2.clone()).expect("建桥");
        let pid = {
            let _ = b2.call_with_deadlines(
                "s",
                "echo",
                args_hi(),
                Some(Duration::from_millis(1500)),
                Some(Duration::from_secs(10)),
            );
            // 超时后 shutdown 已 taskkill 并注销
            b2.live_pids()
        };
        assert!(pid.is_empty(), "被杀后登记簿应清空，实际 {pid:?}");
        println!("it7 ✓ 登记簿：连接建立后在册、被杀后出册");
    }
    /// ② 崩溃后懒恢复：第一次 crashed，第二次确实**重新握手**（审计含 lazy restart）。
    #[test]
    fn it2_crash_then_lazy_restart() {
        let obs = Arc::new(Rec::default());
        let b = Bridge::new(vec![("s".into(), cfg("crash", vec![]))], obs.clone()).expect("建桥");

        let e1 = b.call("s", "echo", args_hi()).expect_err("crash 模式应失败");
        assert!(e1.contains("MCP 调用失败"), "{e1}");

        // 第二次：应重握手（lazy restart）后再次崩溃 —— 关键是"确实重拉了"
        let _ = b.call("s", "echo", args_hi());
        let ev = obs.events();
        assert!(ev.iter().any(|e| e.result == "crashed"), "应有 crashed 审计");
        assert!(
            ev.iter().any(|e| e.reason.contains("lazy restart")),
            "第二次调用应记录 lazy restart；实际审计：{:?}",
            ev.iter().map(|e| (&e.result, &e.reason)).collect::<Vec<_>>()
        );
        println!("it2 ✓ 崩溃 → 标记需重拉 → 下次 lazy restart");
    }

    /// ③ 内层超时：杀进程，且审计 killed=true、result=timeout。
    #[test]
    fn it3_hang_timeout_kills_child() {
        let mark = tmp("hang_pid");
        let _ = std::fs::remove_file(&mark);
        let obs = Arc::new(Rec::default());
        let b = Bridge::new(
            vec![(
                "s".into(),
                cfg(
                    "hang",
                    vec!["--mark-file".into(), mark.to_string_lossy().to_string()],
                ),
            )],
            obs.clone(),
        )
        .expect("建桥");

        let e = b
            .call_with_deadlines(
                "s",
                "echo",
                args_hi(),
                Some(Duration::from_millis(1500)),
                Some(Duration::from_secs(10)),
            )
            .expect_err("hang 应超时");
        assert!(e.contains("超时"), "{e}");

        // mark 文件里是子进程 pid
        let pid: u32 = std::fs::read_to_string(&mark)
            .expect("mark 文件应存在（证明调用确实到达了 server）")
            .trim()
            .parse()
            .expect("应含 pid");
        std::thread::sleep(Duration::from_millis(400));
        assert!(!pid_alive(pid), "内层超时后子进程 pid={pid} 应已被终止");

        let t = obs.last_with("timeout").expect("应有 result=timeout 审计");
        assert!(t.killed, "应记录 killed=true");
        assert!(t.reason.contains("call timeout"), "reason={}", t.reason);
        let _ = std::fs::remove_file(&mark);
        println!("it3 ✓ 内层超时 → 杀 pid {pid}，审计 timeout/killed");
    }

    /// ④ stderr 洪泛 + 崩溃 → 不卡死，且 reason 被截断并标注。
    #[test]
    fn it4_stderr_flood_reason_truncated() {
        let obs = Arc::new(Rec::default());
        let b = Bridge::new(
            vec![(
                "s".into(),
                cfg(
                    "crash",
                    vec!["--flood-bytes".into(), "100000".to_string()],
                ),
            )],
            obs.clone(),
        )
        .expect("建桥");

        let _ = b.call("s", "echo", args_hi());
        let c = obs.last_with("crashed").expect("应有 crashed 审计");
        assert!(
            c.reason.chars().count() <= REASON_CAP,
            "reason 超上限：{} 字符",
            c.reason.chars().count()
        );
        assert!(c.reason.contains("truncated"), "应标注截断：{}", c.reason);
        println!("it4 ✓ stderr 截断生效（{} 字符）", c.reason.chars().count());
    }

    /// ⑤ 外层超时：桥接线程在 dispatch 前卡死 → 外层兜底返回"桥接失效"。
    #[test]
    fn it5_outer_timeout_reports_bridge_failure() {
        let obs = Arc::new(Rec::default());
        let hook: Arc<dyn Fn() + Send + Sync> =
            Arc::new(|| std::thread::sleep(Duration::from_secs(6)));
        let b = Bridge::new_with_hook(
            vec![("s".into(), cfg("normal", vec![]))],
            obs.clone(),
            Some(hook),
        )
        .expect("建桥");

        let t = Instant::now();
        let e = b
            .call_with_deadlines(
                "s",
                "echo",
                args_hi(),
                Some(Duration::from_millis(100)),
                Some(Duration::from_millis(100)),
            )
            .expect_err("外层应超时");
        assert!(e.contains("桥接失效"), "{e}");
        assert!(
            t.elapsed() >= OUTER_SLACK,
            "外层应等满 call+handshake+slack，实际 {:?}",
            t.elapsed()
        );
        println!("it5 ✓ 外层超时 {:?} → {}", t.elapsed(), e);
    }

    /// ⑥ cancellation：外层超时后桥接线程恢复，必须**跳过执行**（不产生未报告的成功）。
    #[test]
    fn it6_cancelled_before_dispatch_is_not_executed() {
        let mark = tmp("cancel_mark");
        let _ = std::fs::remove_file(&mark);
        let obs = Arc::new(Rec::default());
        let hook: Arc<dyn Fn() + Send + Sync> =
            Arc::new(|| std::thread::sleep(Duration::from_secs(6)));
        let b = Bridge::new_with_hook(
            vec![(
                "s".into(),
                cfg(
                    "normal",
                    vec!["--mark-file".into(), mark.to_string_lossy().to_string()],
                ),
            )],
            obs.clone(),
            Some(hook),
        )
        .expect("建桥");

        let e = b
            .call_with_deadlines(
                "s",
                "echo",
                args_hi(),
                Some(Duration::from_millis(100)),
                Some(Duration::from_millis(100)),
            )
            .expect_err("外层应超时");
        assert!(e.contains("桥接失效"), "{e}");

        // 等 hook 睡醒（6s），给桥接线程足够时间"如果它要执行的话"
        std::thread::sleep(Duration::from_millis(2500));

        assert!(
            !mark.exists(),
            "被取消的请求**不应**真的调用 server（否则就是未报告的成功）"
        );
        assert!(
            obs.events().iter().any(|e| e.result == "cancelled"),
            "应记录 cancelled 审计"
        );
        println!("it6 ✓ 取消后跳过执行（mark 文件未产生）");
    }
}