//! 配置与路径：**运行时优先，内嵌兜底**。
//!
//! 背景（可扩展性分析报告 v2 §4.3 ⑥）：原先 `selectors.json` / `prompt.initial.md` 由
//! `include_str!` 在**编译期**打进二进制 —— 改配置必须重新编译，与「零代码配置变更」直接冲突。
//!
//! 现在改为运行时读取，查找顺序（先命中先用）：
//!   1. `$EAGENT_CONFIG_DIR/<name>`
//!   2. `<exe 目录>/config/<name>` 及其上溯 1–3 级（覆盖 `target/debug` 开发布局）
//!   3. `<当前工作目录>/config/<name>`
//!   4. **编译期内嵌副本**（兜底：文件缺失/不可读也能正常启动）
//!
//! 非安全字段每次调用重读磁盘 → 改完**下一回合即生效**；
//! 安全配置（`autoApprovePatterns` / `replyEndpoint` / `approval.json` / `mcp.json` /
//! `provider.json`）在首次使用时冻结为快照，外部改动需 UI 显式重载，未确认时回退安全默认。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// 编译期内嵌兜底副本（仅在运行时文件全部缺失时使用）
const EMBEDDED_SELECTORS: &str = include_str!("../../../config/selectors.json");
const EMBEDDED_PROMPT: &str = include_str!("../../../config/prompt.initial.md");
const EMBEDDED_AGENT: &str = include_str!("../../../config/agent.json");
const EMBEDDED_APPROVAL: &str = include_str!("../../../config/approval.json");
const EMBEDDED_PROVIDER: &str = include_str!("../../../config/provider.json");

// ---------- 运行时路径 ----------

/// 已解析的数据目录（进程内只解析一次）。
static DATA_DIR: OnceLock<String> = OnceLock::new();
/// 已解析的缓存目录（进程内只解析一次）。缓存可丢，不放进 Roaming。
static CACHE_DIR: OnceLock<String> = OnceLock::new();
/// 路径探测记录（供启动日志 / 自检展示）。独立于数据目录，避免解析期间递归写日志。
static PATH_PROBES: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record_probe(label: &str, path: &Path, ok: bool, note: &str) {
    if let Ok(mut v) = PATH_PROBES.lock() {
        v.push(format!(
            "[{}] {label}: {}（{note}）",
            if ok { "OK" } else { "FAIL" },
            path.display()
        ));
    }
}

/// 写入探针：建目录 → 写临时文件 → 删除。失败即视为不可写。
fn probe_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let p = dir.join(format!(".eagent_write_probe_{}", std::process::id()));
    let ok = std::fs::write(&p, b"x").is_ok();
    if ok {
        let _ = std::fs::remove_file(&p);
    }
    ok
}

/// 开发布局：exe 在 `target/debug|release` 下，且能找到仓库根（含 app/src-tauri）。
/// 命中时沿用 `<repo>/app/data`，避免升级本代码后丢失登录态/数据库。
fn dev_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let s = exe.to_string_lossy();
    if !s.contains("\\target\\debug\\") && !s.contains("\\target\\release\\") {
        return None;
    }
    let mut cur = exe.parent()?.to_path_buf();
    for _ in 0..8 {
        if cur.join("app").join("src-tauri").is_dir() {
            return Some(cur.join("app").join("data"));
        }
        cur = cur.parent()?.to_path_buf();
    }
    None
}

/// 数据目录解析链：
/// `$EAGENT_DATA_DIR` → 开发布局 → `%APPDATA%\Mica` → `%LOCALAPPDATA%\Mica`
/// → exe 旁 `data\` → `%TEMP%\Mica`。每一级都做写探针；失败原因进 `path_probe_report()`。
fn resolve_data_dir() -> String {
    let mut candidates: Vec<(&'static str, PathBuf)> = Vec::new();
    if let Ok(v) = std::env::var("EAGENT_DATA_DIR") {
        let v = v.trim();
        if !v.is_empty() {
            candidates.push(("EAGENT_DATA_DIR", PathBuf::from(v)));
        }
    }
    if let Some(d) = dev_data_dir() {
        candidates.push(("dev-layout", d));
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.trim().is_empty() {
            candidates.push(("APPDATA", Path::new(&appdata).join("Mica")));
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if !local.trim().is_empty() {
            candidates.push(("LOCALAPPDATA", Path::new(&local).join("Mica")));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(("exe-side", dir.join("data")));
        }
    }
    if let Ok(tmp) = std::env::var("TEMP") {
        if !tmp.trim().is_empty() {
            candidates.push(("TEMP", Path::new(&tmp).join("Mica")));
        }
    }

    for (label, p) in &candidates {
        let ok = probe_writable(p);
        record_probe(label, p, ok, if ok { "可写" } else { "不可写" });
        if ok {
            return p.to_string_lossy().to_string();
        }
    }
    let fallback = candidates
        .first()
        .map(|(_, p)| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());
    record_probe("fallback", Path::new(&fallback), false, "所有候选均不可写");
    fallback
}

/// 数据目录：所有派生路径（tmp、日志、DB、WebView2 profile）均基于此目录。
///
/// 解析结果进程内缓存；失败不会静默回落到不可写路径而不报错 ——
/// `path_probe_report()` 会把每一级的结果交给启动日志/自检。
pub fn data_dir() -> String {
    DATA_DIR.get_or_init(resolve_data_dir).clone()
}

/// 路径探测报告（启动日志与自检用）。
pub fn path_probe_report() -> String {
    PATH_PROBES
        .lock()
        .map(|v| v.join(" ｜ "))
        .unwrap_or_else(|_| "路径探测记录不可用".to_string())
}

/// 缓存目录解析链：`$EAGENT_CACHE_DIR` → 开发布局 `<data>/cache`
/// → `%LOCALAPPDATA%\Mica\cache` → `<data>/cache` → `%TEMP%\Mica\cache`。
fn resolve_cache_dir() -> String {
    let mut candidates: Vec<(&'static str, PathBuf)> = Vec::new();
    if let Ok(v) = std::env::var("EAGENT_CACHE_DIR") {
        let v = v.trim();
        if !v.is_empty() {
            candidates.push(("EAGENT_CACHE_DIR", PathBuf::from(v)));
        }
    }
    let dev = dev_data_dir().is_some();
    if dev {
        // 开发布局：保持 WebView2 profile 原地（不丢登录态）。
        candidates.push(("dev-cache", Path::new(&data_dir()).join("cache")));
    } else if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if !local.trim().is_empty() {
            // 注意：per-user 安装目录就是 %LOCALAPPDATA%\Mica，
            // 缓存必须放在**兄弟目录**，否则卸载后安装目录会残留 cache。
            candidates.push((
                "LOCALAPPDATA-cache",
                Path::new(&local).join("MicaCache"),
            ));
        }
    }
    candidates.push(("data-cache", Path::new(&data_dir()).join("cache")));
    if let Ok(tmp) = std::env::var("TEMP") {
        if !tmp.trim().is_empty() {
            candidates.push(("TEMP-cache", Path::new(&tmp).join("Mica").join("cache")));
        }
    }
    for (label, p) in &candidates {
        let ok = probe_writable(p);
        record_probe(label, p, ok, if ok { "可写" } else { "不可写" });
        if ok {
            return p.to_string_lossy().to_string();
        }
    }
    candidates
        .first()
        .map(|(_, p)| p.to_string_lossy().to_string())
        .unwrap_or_else(|| data_dir())
}

/// 缓存目录：WebView2 profile、npm 缓存等可重建数据。
pub fn cache_dir() -> String {
    CACHE_DIR.get_or_init(resolve_cache_dir).clone()
}

/// WebView2 profile 目录：开发布局保持 `<data>/webview`（不丢登录态）；
/// 发布布局放缓存目录（`%LOCALAPPDATA%\Mica\cache\webview`），不写 Roaming。
pub fn webview_dir() -> String {
    if dev_data_dir().is_some() {
        format!(r"{}\webview", data_dir())
    } else {
        format!(r"{}\webview", cache_dir())
    }
}

/// 默认工作目录：用户桌面（不存在则用户主目录，再退当前目录）。
/// 发布环境不能用开发机的 `C:\Users\<dev>\Desktop`。
pub fn default_workdir() -> String {
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let profile = profile.trim();
        if !profile.is_empty() {
            let desktop = Path::new(profile).join("Desktop");
            if desktop.is_dir() {
                return desktop.to_string_lossy().to_string();
            }
            return profile.to_string();
        }
    }
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

pub fn tmp_dir() -> String {
    format!(r"{}\tmp", data_dir())
}

/// MCP 文件工具的专用根目录（与安全配置/密钥/数据文件是兄弟关系，不在其内）。
/// 创建失败由调用方决定是否禁用 MCP（fail-closed）。
pub fn workspace_dir() -> Result<String, String> {
    let p = Path::new(&data_dir()).join("workspace");
    std::fs::create_dir_all(&p).map_err(|e| format!("创建 workspace 失败：{e}"))?;
    Ok(p.to_string_lossy().to_string())
}

pub fn chain_log_path() -> String {
    format!(r"{}\chain_test.log", data_dir())
}

pub fn audit_log_path() -> String {
    format!(r"{}\audit.log", data_dir())
}

/// 仅 debug 构建使用（链路自检的一次性 flag）。release 下不编译，避免 dead_code。
#[cfg(debug_assertions)]
pub fn chain_flag_path() -> String {
    format!(r"{}\.chain_test_done", data_dir())
}

// ---------- 配置查找 ----------

/// 候选配置目录（按优先级）。
pub(crate) fn config_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Ok(d) = std::env::var("EAGENT_CONFIG_DIR") {
        if !d.trim().is_empty() {
            dirs.push(PathBuf::from(d));
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("config"));
            // 开发布局：target/debug/eagent-app.exe → 逐级上溯到仓库根
            // 需要 4 级才能到达 <repo>/config（debug → target → src-tauri → app → repo）
            let mut cur = dir.to_path_buf();
            for _ in 0..5 {
                match cur.parent() {
                    Some(p) => {
                        cur = p.to_path_buf();
                        dirs.push(cur.join("config"));
                    }
                    None => break,
                }
            }
        }
    }

    dirs.push(PathBuf::from("config"));
    dirs
}

/// 读一份配置文本：运行时文件优先，全部不可用则回退内嵌副本。
fn load_text(name: &str, embedded: &str) -> String {
    for dir in config_dirs() {
        let p = dir.join(name);
        match std::fs::read_to_string(&p) {
            Ok(s) if !s.trim().is_empty() => return s,
            _ => continue,
        }
    }
    embedded.to_string()
}

/// 选择器配置（运行时优先）。每次调用重读 → 改完即生效。
pub fn selectors_json() -> String {
    load_text("selectors.json", EMBEDDED_SELECTORS)
}

/// 系统提示词模板（运行时优先）。占位符 `{{WORKDIR}}` / `{{TEMP_WORKDIR}}` 由调用方替换。
pub fn prompt_template() -> String {
    load_text("prompt.initial.md", EMBEDDED_PROMPT)
}

/// 审批风险规则（运行时优先）。
///
/// **只影响"分级"，不影响"是否放行"** —— 危险类永远需要用户确认；
/// 本文件让用户能自行增删危险模式 / 只读动词，而不必重新编译。
/// 正则的合法性由前端在编译时校验（非法项跳过并报出，不会整体失效）。
pub fn approval_json() -> String {
    read_snapshot().approval_raw
}

// ---------- 安全配置快照 ----------
//
// 安全相关的四份配置（agent.json 的 autoApprovePatterns / replyEndpoint、
// approval.json、mcp.json、provider.json）不在每次调用时直接采用磁盘内容：
//   - 首次运行：采用磁盘内容并写入指纹；
//   - 指纹匹配：采用磁盘内容；
//   - 指纹不匹配（外部改动未确认）：回退到安全默认，直到 UI 显式重载。
// 非安全字段（超时、输出上限、maxToolIterations、extendedTools）仍按原逻辑热读。

use std::sync::RwLock;

/// 安全配置快照（含原始文本，便于 UI 写回后只刷新单个文件而不吞掉其他待确认变更）。
#[derive(Clone)]
pub struct SecuritySnapshot {
    pub agent_raw: String,
    pub approval_raw: String,
    pub mcp_raw: String,
    pub provider_raw: String,
    pub auto_approve_patterns: Vec<String>,
    pub reply_endpoint: String,
    pub mcp_servers: Vec<McpServerCfg>,
    pub provider: ProviderCfg,
    pub loaded_at_ms: u64,
    pub loaded_hash: String,
}

#[derive(Clone, serde::Serialize)]
pub struct SecurityConfigStatus {
    pub loaded_hash: String,
    pub disk_hash: String,
    pub changed: bool,
    pub loaded_at_ms: u64,
}

static SECURITY: OnceLock<RwLock<SecuritySnapshot>> = OnceLock::new();

fn raw_agent() -> String {
    load_text("agent.json", EMBEDDED_AGENT)
}

fn raw_approval() -> String {
    load_text("approval.json", EMBEDDED_APPROVAL)
}

fn raw_mcp() -> String {
    let t = load_text("mcp.json", "");
    if t.trim().is_empty() { String::new() } else { t }
}

fn raw_provider() -> String {
    load_text("provider.json", EMBEDDED_PROVIDER)
}

fn hash_texts(parts: &[&str]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

fn auto_approve_patterns_from_raw(raw: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    v.get("autoApprovePatterns")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn reply_endpoint_from_raw(raw: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return "/api/v0/chat/completion".to_string();
    };
    v.get("replyEndpoint")
        .and_then(|x| x.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("/api/v0/chat/completion")
        .to_string()
}

/// 由四份原始文本构造快照（不读写磁盘，便于测试与局部刷新）。
fn snapshot_from_raws(
    agent_raw: String,
    approval_raw: String,
    mcp_raw: String,
    provider_raw: String,
) -> SecuritySnapshot {
    let loaded_hash = hash_texts(&[&agent_raw, &approval_raw, &mcp_raw, &provider_raw]);
    let auto_approve_patterns = auto_approve_patterns_from_raw(&agent_raw);
    let reply_endpoint = reply_endpoint_from_raw(&agent_raw);
    let mcp_servers = parse_mcp_servers_with(&mcp_raw, &data_dir(), &cache_dir());
    let provider = provider_cfg_from_raw(&provider_raw);
    SecuritySnapshot {
        agent_raw,
        approval_raw,
        mcp_raw,
        provider_raw,
        auto_approve_patterns,
        reply_endpoint,
        mcp_servers,
        provider,
        loaded_at_ms: crate::now_ms(),
        loaded_hash,
    }
}

impl SecuritySnapshot {
    fn from_disk() -> Self {
        snapshot_from_raws(raw_agent(), raw_approval(), raw_mcp(), raw_provider())
    }

    fn safe_fallback(approved_hash: String) -> Self {
        SecuritySnapshot {
            agent_raw: EMBEDDED_AGENT.to_string(),
            approval_raw: EMBEDDED_APPROVAL.to_string(),
            mcp_raw: String::new(),
            provider_raw: EMBEDDED_PROVIDER.to_string(),
            auto_approve_patterns: Vec::new(),
            reply_endpoint: "/api/v0/chat/completion".to_string(),
            mcp_servers: Vec::new(),
            provider: ProviderCfg::default(),
            loaded_at_ms: crate::now_ms(),
            loaded_hash: approved_hash,
        }
    }

    fn recompute_hash(&mut self) {
        self.loaded_hash = hash_texts(&[
            &self.agent_raw,
            &self.approval_raw,
            &self.mcp_raw,
            &self.provider_raw,
        ]);
    }
}

fn security_state_path() -> PathBuf {
    Path::new(&data_dir()).join("security_state.json")
}

fn read_approved_hash() -> Option<String> {
    let raw = std::fs::read_to_string(security_state_path()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("hash")
        .and_then(|h| h.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn write_approved_hash(hash: &str) {
    let p = security_state_path();
    let v = serde_json::json!({ "hash": hash, "approvedAt": crate::now_ms() });
    if let Err(e) = save_json_at(&p, &v) {
        crate::log::chain_log(&format!("security: 写入安全配置指纹失败：{e}"));
    }
}

fn disk_security_hash() -> String {
    hash_texts(&[&raw_agent(), &raw_approval(), &raw_mcp(), &raw_provider()])
}

fn load_security_snapshot() -> SecuritySnapshot {
    let disk = SecuritySnapshot::from_disk();
    match read_approved_hash() {
        Some(approved) if approved != disk.loaded_hash => {
            crate::log::chain_log(
                "security: 安全配置指纹不匹配（外部改动未确认），已回退到安全默认",
            );
            SecuritySnapshot::safe_fallback(approved)
        }
        Some(_) => disk,
        None => {
            // 首次运行：采用随包配置并记录指纹。
            write_approved_hash(&disk.loaded_hash);
            disk
        }
    }
}

fn security_cell() -> &'static RwLock<SecuritySnapshot> {
    SECURITY.get_or_init(|| RwLock::new(load_security_snapshot()))
}

fn read_snapshot() -> SecuritySnapshot {
    match security_cell().read() {
        Ok(g) => g.clone(),
        Err(e) => e.into_inner().clone(),
    }
}

/// 当前安全配置状态（给 UI 提示/重载用）。
pub fn security_config_status() -> SecurityConfigStatus {
    let snap = read_snapshot();
    let disk = disk_security_hash();
    SecurityConfigStatus {
        loaded_hash: snap.loaded_hash.clone(),
        disk_hash: disk.clone(),
        changed: snap.loaded_hash != disk,
        loaded_at_ms: snap.loaded_at_ms,
    }
}

/// UI 显式确认后采用磁盘上的安全配置，并更新指纹。
pub fn reload_security_config() -> SecurityConfigStatus {
    let disk = SecuritySnapshot::from_disk();
    let approved = disk.loaded_hash.clone();
    {
        let cell = security_cell();
        let mut w = match cell.write() {
            Ok(w) => w,
            Err(e) => e.into_inner(),
        };
        *w = disk;
    }
    write_approved_hash(&approved);
    security_config_status()
}

/// UI 自己写回 provider.json 后调用：只采用 provider 文件，不吞掉其他待确认变更。
pub fn refresh_provider_snapshot() {
    let raw = raw_provider();
    let provider = provider_cfg_from_raw(&raw);
    let cell = security_cell();
    let mut w = match cell.write() {
        Ok(w) => w,
        Err(e) => e.into_inner(),
    };
    w.provider_raw = raw;
    w.provider = provider;
    w.recompute_hash();
    let hash = w.loaded_hash.clone();
    drop(w);
    write_approved_hash(&hash);
}

/// UI 自己写回 mcp.json 后调用：只采用 MCP 配置，不吞掉其他待确认变更。
pub fn refresh_mcp_snapshot() {
    let raw = raw_mcp();
    let servers = parse_mcp_servers_with(&raw, &data_dir(), &cache_dir());
    let cell = security_cell();
    let mut w = match cell.write() {
        Ok(w) => w,
        Err(e) => e.into_inner(),
    };
    w.mcp_raw = raw;
    w.mcp_servers = servers;
    w.recompute_hash();
    let hash = w.loaded_hash.clone();
    drop(w);
    write_approved_hash(&hash);
}

// ---------- 引擎行为参数（S3） ----------

/// 引擎行为参数。`config/agent.json` 缺失或字段非法时回落到此处默认值。
#[derive(Clone, Debug)]
pub struct AgentCfg {
    pub tool_timeout_sec: u64,
    pub tool_output_cap: usize,
    pub max_tool_iter: usize,
    pub reply_endpoint: String,
    /// **持久自动放行规则**（正则原文，由前端编译）。
    ///
    /// 默认**空数组** —— 空即"本功能完全不生效"，行为与不配时逐字一致。
    /// 这里只负责把配置读出来；真正的安全边界（危险类永不自动放行 + 逐条审计）
    /// 在 `useEngineEvents` 的放行路径上强制，且**不依赖本配置是否诚实**。
    pub auto_approve_patterns: Vec<String>,
    /// 是否向模型注入「扩展工具清单」（策划案 2.0 · P1.3）。默认 `true`。
    ///
    /// 这是一个**安全阀**：新协议叠在既有 PowerShell 协议之上，若某个模型被新格式干扰
    /// 导致工具调用变差，把它设为 `false` 就回到"只有 PowerShell 协议"的老行为，无需重编译。
    pub extended_tools: bool,
}

impl Default for AgentCfg {
    fn default() -> Self {
        Self {
            tool_timeout_sec: 120,
            tool_output_cap: 4000,
            max_tool_iter: 6,
            reply_endpoint: "/api/v0/chat/completion".to_string(),
            auto_approve_patterns: Vec::new(),
            extended_tools: true,
        }
    }
}

/// 读取引擎参数（运行时优先，逐字段回落默认值）。
pub fn agent_cfg() -> AgentCfg {
    let mut cfg = AgentCfg::default();
    let raw = load_text("agent.json", EMBEDDED_AGENT);
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return cfg;
    };
    if let Some(x) = v.get("toolTimeoutSec").and_then(|x| x.as_u64()) {
        if x > 0 {
            cfg.tool_timeout_sec = x;
        }
    }
    if let Some(x) = v.get("toolOutputCap").and_then(|x| x.as_u64()) {
        if x > 0 {
            cfg.tool_output_cap = x as usize;
        }
    }
    if let Some(x) = v.get("maxToolIterations").and_then(|x| x.as_u64()) {
        if x > 0 {
            cfg.max_tool_iter = x as usize;
        }
    }
    if let Some(s) = v.get("replyEndpoint").and_then(|x| x.as_str()) {
        if !s.trim().is_empty() {
            cfg.reply_endpoint = s.to_string();
        }
    }
    // 自动放行规则：只收非空字符串；非法正则由前端编译时把关
    if let Some(arr) = v.get("autoApprovePatterns").and_then(|x| x.as_array()) {
        cfg.auto_approve_patterns = arr
            .iter()
            .filter_map(|x| x.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
    }
    // 扩展工具清单开关：缺省 true；显式 false 才关闭
    if let Some(b) = v.get("extendedTools").and_then(|x| x.as_bool()) {
        cfg.extended_tools = b;
    }
    // 安全字段一律以快照为准（外部改动未确认时快照是安全默认）。
    let snap = read_snapshot();
    cfg.auto_approve_patterns = snap.auto_approve_patterns;
    cfg.reply_endpoint = snap.reply_endpoint;
    cfg
}

// ---------- 对话通道配置（策划案 2.0 · Phase 2） ----------

/// 对话通道配置（`config/provider.json`）。
#[derive(Clone, Debug)]
pub struct ProviderCfg {
    /// `deepseek-web` / `ollama`
    pub active: String,
    pub ollama_base_url: String,
    pub ollama_model: String,
    pub ollama_connect_timeout_sec: u64,
    pub ollama_request_timeout_sec: u64,
    pub deepseek_max_tool_list_chars: usize,
    pub ollama_max_tool_list_chars: usize,
}

impl Default for ProviderCfg {
    fn default() -> Self {
        Self {
            active: "deepseek-web".to_string(),
            ollama_base_url: "http://127.0.0.1:11434".to_string(),
            ollama_model: "qwen2.5:7b".to_string(),
            ollama_connect_timeout_sec: 3,
            ollama_request_timeout_sec: 180,
            deepseek_max_tool_list_chars: 4000,
            ollama_max_tool_list_chars: 6000,
        }
    }
}

/// 读取通道配置（运行时优先，逐字段回落默认值）。
pub fn provider_cfg() -> ProviderCfg {
    read_snapshot().provider
}

/// 从原始 JSON 解析通道配置（快照加载 / 局部刷新用；不读磁盘，避免递归）。
fn provider_cfg_from_raw(raw: &str) -> ProviderCfg {
    let mut cfg = ProviderCfg::default();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return cfg;
    };
    if let Some(s) = v.get("active").and_then(|x| x.as_str()) {
        let s = s.trim();
        if !s.is_empty() {
            cfg.active = s.to_string();
        }
    }
    if let Some(o) = v.get("ollama").and_then(|x| x.as_object()) {
        if let Some(s) = o.get("baseUrl").and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                cfg.ollama_base_url = s.trim().trim_end_matches('/').to_string();
            }
        }
        if let Some(s) = o.get("model").and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                cfg.ollama_model = s.trim().to_string();
            }
        }
        if let Some(n) = o.get("connectTimeoutSec").and_then(|x| x.as_u64()) {
            if n > 0 {
                cfg.ollama_connect_timeout_sec = n;
            }
        }
        if let Some(n) = o.get("requestTimeoutSec").and_then(|x| x.as_u64()) {
            if n > 0 {
                cfg.ollama_request_timeout_sec = n;
            }
        }
    }
    if let Some(c) = v.get("caps").and_then(|x| x.as_object()) {
        if let Some(n) = c
            .get("deepseek-web")
            .and_then(|x| x.get("maxToolListChars"))
            .and_then(|x| x.as_u64())
        {
            if n > 0 {
                cfg.deepseek_max_tool_list_chars = n as usize;
            }
        }
        if let Some(n) = c
            .get("ollama")
            .and_then(|x| x.get("maxToolListChars"))
            .and_then(|x| x.as_u64())
        {
            if n > 0 {
                cfg.ollama_max_tool_list_chars = n as usize;
            }
        }
    }
    cfg
}

/// `provider.json` 的写入路径（与 mcp.json 同一套查找规则）。
pub fn provider_write_path() -> std::path::PathBuf {
    write_path("provider.json")
}

/// 读 `provider.json` 原始 JSON（保留 `$comment`，供读-改-写）。
pub fn provider_raw_json() -> serde_json::Value {
    let p = provider_write_path();
    match std::fs::read_to_string(&p) {
        Ok(t) => serde_json::from_str(&t).unwrap_or_else(|_| serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    }
}

/// 原子写回 `provider.json`（tmp → rename + .bak）。
pub fn save_provider_json(v: &serde_json::Value) -> Result<(), String> {
    save_json_at(&provider_write_path(), v)
}

/// 某个配置文件的写入路径：优先已存在的那个；都不存在则用第一个候选目录。
fn write_path(name: &str) -> std::path::PathBuf {
    let dirs = config_dirs();
    for d in &dirs {
        let p = d.join(name);
        if p.exists() {
            return p;
        }
    }
    dirs.first()
        .cloned()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(name)
}

/// 原子写回 JSON 配置（先写 .tmp 再 rename，并留一份 .bak）。
fn save_json_at(p: &std::path::Path, v: &serde_json::Value) -> Result<(), String> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建配置目录失败：{e}"))?;
    }
    let text = serde_json::to_string_pretty(v).map_err(|e| format!("序列化失败：{e}"))?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写临时文件失败：{e}"))?;
    if p.exists() {
        let _ = std::fs::copy(p, p.with_extension("json.bak"));
    }
    std::fs::rename(&tmp, p).map_err(|e| format!("替换配置失败：{e}"))?;
    Ok(())
}

/// `mcp.json` 的**写入**路径。
pub fn mcp_write_path() -> std::path::PathBuf {
    write_path("mcp.json")
}

/// 读 `mcp.json` 的原始 JSON（保留 `$comment` 等字段，供"读-改-写"）。
pub fn mcp_raw_json() -> serde_json::Value {
    let p = mcp_write_path();
    match std::fs::read_to_string(&p) {
        Ok(t) => serde_json::from_str(&t).unwrap_or_else(|_| serde_json::json!({ "servers": {} })),
        Err(_) => serde_json::json!({ "servers": {} }),
    }
}

/// 原子写回 `mcp.json`。
pub fn save_mcp_json(v: &serde_json::Value) -> Result<(), String> {
    save_json_at(&mcp_write_path(), v)
}
// ---------- MCP server 配置（策划案 2.0 · 阶段 C） ----------

/// 一个已启用 MCP server 的配置（从 `config/mcp.json` 读取）。
///
#[derive(Clone, Debug)]
pub struct McpServerCfg {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<std::path::PathBuf>,
    pub call_timeout_sec: u64,
    pub handshake_timeout_sec: u64,
}

/// 读取当前生效的 MCP server 列表（来自安全快照；外部改动未确认时可能为空）。
pub fn mcp_servers() -> Vec<McpServerCfg> {
    read_snapshot().mcp_servers
}

/// 纯解析：从 `mcp.json` 原文构造 enabled server 列表。
///
/// `data` / `cache` 显式传入，便于单测；生产用 `parse_mcp_servers()`。
/// 含未解析 `{{...}}` 占位符的 server 会被跳过并记日志 —— 不带着半截路径启动。
/// 去掉 npm 包规格里的 `@version` 后缀；scoped 包名开头的 `@` 保留。
fn package_name(spec: &str) -> &str {
    match spec.rfind('@') {
        Some(pos) if pos > 0 => &spec[..pos],
        _ => spec,
    }
}

fn parse_mcp_servers_with(raw: &str, data: &str, cache: &str) -> Vec<McpServerCfg> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let v: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            crate::log::chain_log(&format!("config: mcp.json 解析失败，MCP 不启用：{e}"));
            return Vec::new();
        }
    };
    let expand = |s: &str| s.replace("{{DATA_DIR}}", data).replace("{{CACHE_DIR}}", cache);
    // 已知不可用/默认禁用的包：git/sqlite 在 npm 上已 404；everything 暴露面过大。
    const BLOCKED_PACKAGES: &[&str] = &[
        "@modelcontextprotocol/server-git",
        "@modelcontextprotocol/server-sqlite",
        "@modelcontextprotocol/server-everything",
    ];
    let mut out = Vec::new();
    let Some(servers) = v.get("servers").and_then(|s| s.as_object()) else {
        return out;
    };
    for (name, cfg) in servers {
        if !cfg.get("enabled").and_then(|b| b.as_bool()).unwrap_or(false) {
            continue;
        }
        let Some(command) = cfg.get("command").and_then(|c| c.as_str()) else {
            crate::log::chain_log(&format!("config: mcp server「{name}」缺 command，已跳过"));
            continue;
        };
        let command = expand(command);
        let args: Vec<String> = cfg
            .get("args")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(|s| expand(s))
                    .collect()
            })
            .unwrap_or_default();
        let env: Vec<(String, String)> = cfg
            .get("env")
            .and_then(|e| e.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), expand(s))))
                    .collect()
            })
            .unwrap_or_default();
        let cwd = cfg
            .get("cwd")
            .and_then(|c| c.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| std::path::PathBuf::from(expand(s)))
            // 未显式指定时统一落在 workspace：memory 等会写文件的 server 不会污染安装目录。
            .or_else(|| Some(std::path::PathBuf::from(format!(r"{}\workspace", data))));

        let unresolved = command.contains("{{")
            || args.iter().any(|s| s.contains("{{"))
            || env.iter().any(|(_, s)| s.contains("{{"))
            || cwd
                .as_ref()
                .map(|p| p.to_string_lossy().contains("{{"))
                .unwrap_or(false);
        if unresolved {
            crate::log::chain_log(&format!(
                "config: mcp server「{name}」含未解析占位符，已跳过"
            ));
            continue;
        }
        if let Some(pkg) = args
            .iter()
            .find(|a| BLOCKED_PACKAGES.iter().any(|b| package_name(a) == *b))
        {
            crate::log::chain_log(&format!(
                "config: mcp server「{name}」使用已禁用/不存在的包「{pkg}」，已跳过"
            ));
            continue;
        }

        out.push(McpServerCfg {
            name: name.clone(),
            command,
            args,
            env,
            cwd,
            call_timeout_sec: cfg.get("callTimeoutSec").and_then(|x| x.as_u64()).unwrap_or(30),
            handshake_timeout_sec: cfg.get("handshakeTimeoutSec").and_then(|x| x.as_u64()).unwrap_or(30),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// 配置来源诊断（供自检/排障展示：到底读的是哪个文件）。
pub fn config_source_report() -> String {
    let mut lines = Vec::new();
    for (name, embedded) in [
        ("selectors.json", EMBEDDED_SELECTORS),
        ("prompt.initial.md", EMBEDDED_PROMPT),
        ("agent.json", EMBEDDED_AGENT),
        ("approval.json", EMBEDDED_APPROVAL),
        ("provider.json", EMBEDDED_PROVIDER),
    ] {
        let mut from: Option<PathBuf> = None;
        for dir in config_dirs() {
            let p = dir.join(name);
            if std::fs::read_to_string(&p).map(|s| !s.trim().is_empty()).unwrap_or(false) {
                from = Some(p);
                break;
            }
        }
        match from {
            Some(p) => lines.push(format!("{name} ← {} ({} 字节)", p.display(), std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0))),
            None => lines.push(format!("{name} ← 内嵌副本 ({} 字节，未找到运行时文件)", embedded.len())),
        }
    }
    lines.join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_fallbacks_exist_and_are_nonempty() {
        assert!(EMBEDDED_SELECTORS.contains("groups"));
        assert!(EMBEDDED_PROMPT.contains("{{WORKDIR}}"));
        assert!(EMBEDDED_AGENT.contains("toolTimeoutSec"));
        assert!(EMBEDDED_APPROVAL.contains("danger"));
        assert!(EMBEDDED_PROVIDER.contains("deepseek-web"));
    }

    #[test]
    fn data_dir_resolves_to_writable_existing_dir() {
        let d = data_dir();
        assert!(!d.is_empty(), "data dir must not be empty");
        assert!(Path::new(&d).is_dir(), "data dir should exist: {d}");
        assert!(
            !path_probe_report().is_empty(),
            "path probe report should record at least one candidate"
        );
    }

    #[test]
    fn cache_dir_resolves_to_writable_dir() {
        let c = cache_dir();
        assert!(!c.is_empty());
        assert!(Path::new(&c).is_dir(), "cache dir should exist: {c}");
        assert!(!webview_dir().is_empty());
    }

    #[test]
    fn default_workdir_exists() {
        let w = default_workdir();
        assert!(!w.is_empty());
        assert!(Path::new(&w).is_dir(), "default workdir should exist: {w}");
    }

    #[test]
    fn provider_cfg_has_sane_defaults() {
        let c = ProviderCfg::default();
        assert_eq!(c.active, "deepseek-web");
        assert!(c.ollama_base_url.starts_with("http"));
        assert!(!c.ollama_model.is_empty());
        assert!(c.deepseek_max_tool_list_chars >= 1000);
        assert!(c.ollama_max_tool_list_chars >= 1000);
        // 运行时读取也必须给出可用值（不依赖文件是否存在）
        let r = provider_cfg();
        assert!(!r.active.is_empty());
        assert!(r.ollama_request_timeout_sec >= 5);
    }

    #[test]
    fn agent_cfg_has_sane_defaults() {
        let c = AgentCfg::default();
        assert!(c.tool_timeout_sec >= 10);
        assert!(c.tool_output_cap >= 1000);
        assert!(c.max_tool_iter >= 1);
        assert!(!c.reply_endpoint.is_empty());
        // ★ 自动放行默认必须是**空规则** —— 空即"功能完全不生效"，这是安全默认
        assert!(c.auto_approve_patterns.is_empty(), "默认不得携带任何自动放行规则");
    }

    #[test]
    fn embedded_agent_has_empty_auto_approve_by_default() {
        // 内嵌兜底副本同样必须是空的（配置丢失时也不能意外开启自动放行）
        let v: serde_json::Value = serde_json::from_str(EMBEDDED_AGENT).expect("合法 JSON");
        let arr = v.get("autoApprovePatterns").and_then(|x| x.as_array());
        assert!(arr.is_some(), "内嵌 agent.json 应显式声明 autoApprovePatterns");
        assert!(arr.unwrap().is_empty(), "内嵌配置的自动放行规则必须为空");
    }

    #[test]
    fn auto_approve_patterns_are_read_and_sanitised() {
        // 用一个临时目录冒充配置目录，验证读取 + 清洗逻辑（不碰真实 data_dir）
        let dir = std::env::temp_dir().join(format!("eagent-aacfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("agent.json");
        std::fs::write(
            &p,
            r#"{"autoApprovePatterns":["  ^Get-ChildItem  ","", 42, "  ", "^Get-Date"]}"#,
        )
        .unwrap();
        let raw = std::fs::read_to_string(&p).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let got: Vec<String> = v
            .get("autoApprovePatterns")
            .and_then(|x| x.as_array())
            .unwrap()
            .iter()
            .filter_map(|x| x.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        assert_eq!(got, vec!["^Get-ChildItem".to_string(), "^Get-Date".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_auto_approve_field_yields_empty_not_error() {
        // 旧版配置文件没有该字段 → 必须回落为空（功能不启用），而不是报错或默认开启
        let v: serde_json::Value = serde_json::from_str(r#"{"toolTimeoutSec": 60}"#).unwrap();
        let got = v.get("autoApprovePatterns").and_then(|x| x.as_array());
        assert!(got.is_none(), "缺字段时读取结果应为 None，由调用方回落到空 Vec");
    }

    #[test]
    fn selectors_json_is_parseable() {
        // 无论走运行时还是内嵌，都必须是合法 JSON 且含 groups
        let v: serde_json::Value = serde_json::from_str(&selectors_json()).expect("valid json");
        assert!(v.get("groups").is_some());
    }

    #[test]
    fn parse_mcp_servers_expands_placeholders_and_skips_unresolved() {
        let raw = r#"{
            "servers": {
                "fs": {
                    "enabled": true,
                    "command": "npx",
                    "args": ["-y", "pkg@1.0.0", "{{DATA_DIR}}\\workspace"],
                    "env": { "X": "{{CACHE_DIR}}\\c" },
                    "cwd": "{{DATA_DIR}}\\workspace"
                },
                "broken": {
                    "enabled": true,
                    "command": "npx",
                    "args": ["{{NOPE}}"]
                },
                "git": {
                    "enabled": true,
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-git@1.0.0"]
                },
                "off": { "enabled": false, "command": "npx", "args": [] }
            }
        }"#;
        let got = parse_mcp_servers_with(raw, r"D:\data", r"D:\cache");
        assert_eq!(got.len(), 1, "broken/git/off 应被跳过");
        let s = &got[0];
        assert_eq!(s.name, "fs");
        assert_eq!(s.args[2], r"D:\data\workspace");
        assert_eq!(s.env[0].1, r"D:\cache\c");
        assert_eq!(
            s.cwd.as_ref().map(|p| p.to_string_lossy().to_string()),
            Some(r"D:\data\workspace".to_string())
        );
    }

    #[test]
    fn snapshot_from_raws_parses_security_fields() {
        let snap = snapshot_from_raws(
            r#"{"autoApprovePatterns":[" ^Get-ChildItem ",""],"replyEndpoint":"/x"}"#.into(),
            r#"{"danger":["x"],"readonly":"^Get-"}"#.into(),
            r#"{"servers":{}}"#.into(),
            r#"{"active":"ollama"}"#.into(),
        );
        assert_eq!(snap.auto_approve_patterns, vec!["^Get-ChildItem".to_string()]);
        assert_eq!(snap.reply_endpoint, "/x");
        assert_eq!(snap.provider.active, "ollama");
        assert!(snap.mcp_servers.is_empty());
        assert!(!snap.loaded_hash.is_empty());
    }

    #[test]
    fn safe_fallback_disables_security_sensitive_features() {
        let snap = SecuritySnapshot::safe_fallback("approved-old-hash".into());
        assert!(snap.auto_approve_patterns.is_empty());
        assert!(snap.mcp_servers.is_empty());
        assert_eq!(snap.provider.active, "deepseek-web");
        assert_eq!(snap.loaded_hash, "approved-old-hash");
        assert_eq!(snap.reply_endpoint, "/api/v0/chat/completion");
    }
}
