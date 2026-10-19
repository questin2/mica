//! DeepSeek 网页通道（现役逻辑收编）。
//!
//! 本模块**只做转发**：发送/完成判定/读回全部沿用 `session::send_and_wait_full`，
//! 不修改选择器、网络层、审批门。收编的目标是行为逐字一致，不是顺手重构。

use tauri::{AppHandle, Manager};

use super::{ChatProvider, ProviderCaps, TurnReply};
use crate::session;

pub struct WebsiteProvider {
    app: AppHandle,
}

impl WebsiteProvider {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ChatProvider for WebsiteProvider {
    fn id(&self) -> &'static str {
        "deepseek-web"
    }

    fn label(&self) -> &'static str {
        "DeepSeek 网页（免费）"
    }

    fn caps(&self) -> ProviderCaps {
        ProviderCaps {
            supports_tools: true,
            streaming: true,
            max_tool_list_chars: crate::config::provider_cfg().deepseek_max_tool_list_chars,
        }
    }

    fn reset(&self) -> Result<(), String> {
        // 网页通道的"新对话"由 start_session_inner 负责（需要 WebView 就绪）。
        // 这里只清掉会话标记，下一次 agent_turn 会重新初始化。
        let st = self.app.state::<crate::state::EngineState>();
        let mut s = st.session_started.lock().map_err(|e| e.to_string())?;
        *s = false;
        Ok(())
    }

    fn send_turn(&self, message: &str) -> Result<TurnReply, String> {
        let text = session::send_and_wait_full(&self.app, message)?;
        Ok(TurnReply { text, tool_calls: Vec::new() })
    }
}
