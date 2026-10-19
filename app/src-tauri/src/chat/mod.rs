//! 对话层：可切换的 `ChatProvider`（策划案 2.0 · Phase 2）。
//!
//! 设计要点（ADR-0004）：
//! - `send_turn` 是**一轮完整语义**（发送 + 等待 + 读回），不暴露半成品状态；
//! - Provider 实例由 `EngineState.chat` 持有，按配置键（通道 + 模型 + 地址）重建；
//! - 工具循环仍留在 `agent.rs`，Provider 不解析工具标记；
//! - 任何 Provider 失败都必须返回明确错误，不静默降级到另一通道。

pub mod ollama;
pub mod website;

use std::time::Duration;

use tauri::Manager;

use crate::config;
use crate::state::EngineState;
use crate::tools::JsonCall;

/// 通道能力声明。工具清单裁剪、原生 tool_calls 均由它决定。
#[derive(Clone, Debug)]
pub struct ProviderCaps {
    /// 是否支持工具调用（当前网页通道走文本协议，Ollama 后期走原生 tool_calls）
    pub supports_tools: bool,
    /// 是否流式
    pub streaming: bool,
    /// 注入提示词的工具清单上限（字符数）
    pub max_tool_list_chars: usize,
}

/// 一轮回复。`tool_calls` 非空表示模型走了原生工具调用（目前仅 Ollama）。
#[derive(Clone, Debug, Default)]
pub struct TurnReply {
    pub text: String,
    pub tool_calls: Vec<JsonCall>,
}

/// 可切换的对话通道。
pub trait ChatProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn caps(&self) -> ProviderCaps;
    /// 清空会话历史（切换通道/开新会话时调用）。
    fn reset(&self) -> Result<(), String>;
    /// 设置系统提示词。网页通道没有独立 system 消息，默认 no-op。
    fn set_system_prompt(&self, _prompt: &str) -> Result<(), String> {
        Ok(())
    }
    /// 阻塞直到本轮完整回复；内部自行处理完成判定/超时。
    fn send_turn(&self, message: &str) -> Result<TurnReply, String>;
    /// 尽力取消当前回合（阻塞式 HTTP 首版只能标记；网页通道由急停/审批拒绝处理）。
    fn cancel(&self) -> Result<(), String> {
        Ok(())
    }
}

/// 当前持有的 Provider 实例 + 构造键。
pub(crate) struct ActiveProvider {
    pub(crate) key: String,
    pub(crate) provider: Box<dyn ChatProvider>,
}

/// 当前配置对应的实例键。配置变化（切通道/换模型/改地址）→ 下一回合自动重建。
pub(crate) fn provider_key(cfg: &config::ProviderCfg) -> String {
    match cfg.active.as_str() {
        "ollama" => format!("ollama|{}|{}", cfg.ollama_base_url, cfg.ollama_model),
        "deepseek-web" => "deepseek-web".to_string(),
        other => format!("{other}|fallback-web"),
    }
}

/// 当前通道的能力声明（不建立连接，纯配置推导）。
pub fn current_caps() -> ProviderCaps {
    let cfg = config::provider_cfg();
    match cfg.active.as_str() {
        "ollama" => ProviderCaps {
            supports_tools: true,
            streaming: false,
            max_tool_list_chars: cfg.ollama_max_tool_list_chars,
        },
        _ => ProviderCaps {
            supports_tools: true,
            streaming: true,
            max_tool_list_chars: cfg.deepseek_max_tool_list_chars,
        },
    }
}

/// 构造一个新 Provider。未知通道回退网页通道并记日志（不让应用因配置写错而不可用）。
fn build_provider(app: &tauri::AppHandle, cfg: &config::ProviderCfg) -> Box<dyn ChatProvider> {
    match cfg.active.as_str() {
        "ollama" => Box::new(ollama::OllamaProvider::new(cfg)),
        "deepseek-web" => Box::new(website::WebsiteProvider::new(app.clone())),
        other => {
            crate::log::chain_log(&format!(
                "chat: 未知通道「{other}」，回退 deepseek-web（请检查 provider.json）"
            ));
            Box::new(website::WebsiteProvider::new(app.clone()))
        }
    }
}

/// 取当前 Provider；配置键变化时重建实例。
fn with_provider<R>(
    app: &tauri::AppHandle,
    f: impl FnOnce(&dyn ChatProvider) -> Result<R, String>,
) -> Result<R, String> {
    let cfg = config::provider_cfg();
    let key = provider_key(&cfg);
    let st = app.state::<EngineState>();
    let mut guard = st.chat.lock().map_err(|e| format!("对话状态锁失败：{e}"))?;
    let need_rebuild = guard.as_ref().map(|a| a.key != key).unwrap_or(true);
    if need_rebuild {
        if let Some(old) = guard.as_ref() {
            let _ = old.provider.cancel();
            let _ = old.provider.reset();
            crate::log::chain_log(&format!("chat: 切换通道 {} → {}（重建实例）", old.key, key));
        }
        *guard = Some(ActiveProvider {
            key,
            provider: build_provider(app, &cfg),
        });
    }
    let active = guard.as_mut().ok_or("对话状态初始化失败")?;
    f(active.provider.as_ref())
}

/// 发送一轮消息（当前通道）。
pub(crate) fn send_turn(app: &tauri::AppHandle, message: &str) -> Result<TurnReply, String> {
    with_provider(app, |p| {
        crate::log::chain_log(&format!(
            "chat: provider={}（{}）发送 {} 字符",
            p.id(),
            p.label(),
            message.chars().count()
        ));
        p.send_turn(message)
    })
}

/// 设置当前通道的系统提示词。
pub(crate) fn set_system_prompt(app: &tauri::AppHandle, prompt: &str) -> Result<(), String> {
    with_provider(app, |p| p.set_system_prompt(prompt))
}

/// 清空当前通道会话（切换/开新对话）。
pub(crate) fn reset(app: &tauri::AppHandle) -> Result<(), String> {
    with_provider(app, |p| p.reset())
}

/// 当前通道 id（不建实例）。
pub fn active_id() -> String {
    config::provider_cfg().active
}

/// 当前通道展示名（不建实例）。
pub fn active_label() -> String {
    match active_id().as_str() {
        "ollama" => {
            let c = config::provider_cfg();
            format!("Ollama: {}", c.ollama_model)
        }
        _ => "DeepSeek 网页（免费）".to_string(),
    }
}

/// 给 UI 的通道列表（Ollama 可用性探测带短超时）。
#[derive(serde::Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub active: bool,
    pub available: bool,
    pub streaming: bool,
    pub model: Option<String>,
    pub models: Vec<String>,
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ProvidersPayload {
    pub active: String,
    pub providers: Vec<ProviderInfo>,
}

/// 列出通道。Ollama 未启动不报错，`available=false` + `error` 交给 UI 展示。
pub fn list_providers() -> ProvidersPayload {
    let cfg = config::provider_cfg();
    let active = cfg.active.clone();
    let mut providers = vec![ProviderInfo {
        id: "deepseek-web".to_string(),
        label: "DeepSeek 网页（免费）".to_string(),
        kind: "website".to_string(),
        active: active == "deepseek-web",
        available: true,
        streaming: true,
        model: None,
        models: Vec::new(),
        error: None,
    }];

    let (models, mut error) = match ollama::list_models(
        &cfg.ollama_base_url,
        Duration::from_secs(cfg.ollama_connect_timeout_sec.min(3)),
    ) {
        Ok(m) => (m, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    // API 可达但一个模型都没有 → 仍然不可用；把原因说清楚，别让 UI 显示"已连接"。
    if error.is_none() && models.is_empty() {
        error = Some("Ollama 已运行，但本地没有任何模型；请先执行 `ollama pull <模型名>`".to_string());
    }
    let available = error.is_none();
    let mut ollama_models = models.clone();
    if !ollama_models.iter().any(|m| m == &cfg.ollama_model) {
        // 把配置里的模型也列出来（即便当前不在本地），UI 才能显示"已选但不可用"
        ollama_models.insert(0, cfg.ollama_model.clone());
    }
    providers.push(ProviderInfo {
        id: "ollama".to_string(),
        label: format!("Ollama: {}", cfg.ollama_model),
        kind: "ollama".to_string(),
        active: active == "ollama",
        available,
        streaming: false,
        model: Some(cfg.ollama_model.clone()),
        models: ollama_models,
        error,
    });

    ProvidersPayload { active, providers }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_key_changes_with_model_and_base_url() {
        let mut c = config::ProviderCfg::default();
        assert_eq!(provider_key(&c), "deepseek-web");
        c.active = "ollama".into();
        c.ollama_model = "a".into();
        let k1 = provider_key(&c);
        c.ollama_model = "b".into();
        let k2 = provider_key(&c);
        assert_ne!(k1, k2, "换模型必须重建 Provider");
        c.ollama_base_url = "http://127.0.0.1:9999".into();
        assert_ne!(k2, provider_key(&c), "换地址必须重建 Provider");
    }

    #[test]
    fn caps_follow_configured_provider() {
        let caps = current_caps();
        assert!(caps.max_tool_list_chars > 0);
        assert!(caps.supports_tools);
    }
}
