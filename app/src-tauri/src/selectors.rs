//! 选择器配置加载（运行时优先，见 config.rs）。
//!
//! 拆分自原单文件 `lib.rs`（可扩展性分析报告 v2 · S1）。
//! `use super::*` 继承根模块的导入与共享项，避免重复声明。

use super::*;

// ---------- selectors ----------

pub(crate) fn sel_list(v: &serde_json::Value, group: &str) -> Vec<String> {
    v.get("groups")
        .and_then(|g| g.get(group))
        .and_then(|g| g.get("candidates"))
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("selector").and_then(|s| s.as_str()).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) struct SelCfg {
    pub(crate) assistant: Vec<String>,
    pub(crate) send_btn: Vec<String>,
    pub(crate) arrow_prefix: String,
    pub(crate) timeout_sec: u64,
    pub(crate) poll_ms: u64,
    pub(crate) fallback_stable: u32,
    pub(crate) cooldown_ticks: u32,
}

pub(crate) fn load_sel() -> SelCfg {
    // 【S2】运行时读取（改完下一回合生效；文件缺失自动回退内嵌副本）
    let v: serde_json::Value = serde_json::from_str(&config::selectors_json()).unwrap_or_default();
    let comp = v.get("completion");
    let get_u64 = |p: &serde_json::Value, k: &str, d: u64| p.get(k).and_then(|x| x.as_u64()).unwrap_or(d);
    SelCfg {
        assistant: sel_list(&v, "assistant"),
        send_btn: sel_list(&v, "sendBtn"),
        arrow_prefix: v
            .pointer("/sendStates/svgFallback/arrowPrefix")
            .and_then(|x| x.as_str())
            .unwrap_or("M8.3125 0.980206")
            .to_string(),
        timeout_sec: comp.map(|c| get_u64(c, "timeoutSec", 180)).unwrap_or(180),
        poll_ms: comp.map(|c| get_u64(c, "pollMs", 400)).unwrap_or(400),
        fallback_stable: comp.map(|c| get_u64(c, "fallbackStableTicks", 7) as u32).unwrap_or(7),
        cooldown_ticks: comp.map(|c| get_u64(c, "cooldownTicks", 2) as u32).unwrap_or(2),
    }
}
