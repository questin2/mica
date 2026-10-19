//! Ollama 本地通道（策划案 2.0 · Phase 2 / P4.2）。
//!
//! 走 Ollama 原生 `/api/chat`（非流式首版）：
//! - `GET /api/tags` 列出本地模型；
//! - `POST /api/chat` 完成一轮；支持原生 `tools`（模型不支持时自动回退纯文本协议）；
//! - 会话历史保存在 Provider 实例内，切换通道/换模型时重建。

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{ChatProvider, ProviderCaps, TurnReply};
use crate::config::ProviderCfg;
use crate::tools::JsonCall;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OllamaMessage {
    pub role: String,
    pub content: String,
}

pub struct OllamaProvider {
    base_url: String,
    model: String,
    request_timeout: Duration,
    connect_timeout: Duration,
    messages: Mutex<Vec<OllamaMessage>>,
    /// 首次原生 tools 请求被模型拒绝后置 true，后续不再附带 tools。
    native_tools_unsupported: Mutex<bool>,
}

impl OllamaProvider {
    pub fn new(cfg: &ProviderCfg) -> Self {
        Self {
            base_url: cfg.ollama_base_url.trim_end_matches('/').to_string(),
            model: cfg.ollama_model.clone(),
            request_timeout: Duration::from_secs(cfg.ollama_request_timeout_sec.max(5)),
            connect_timeout: Duration::from_secs(cfg.ollama_connect_timeout_sec.max(1)),
            messages: Mutex::new(Vec::new()),
            native_tools_unsupported: Mutex::new(false),
        }
    }

    fn client(&self) -> Result<reqwest::blocking::Client, String> {
        reqwest::blocking::Client::builder()
            .connect_timeout(self.connect_timeout)
            .timeout(self.request_timeout)
            .build()
            .map_err(|e| format!("创建 Ollama HTTP 客户端失败：{e}"))
    }

    fn post_chat(
        &self,
        messages: &[OllamaMessage],
        with_tools: bool,
    ) -> Result<ChatResponse, String> {
        let client = self.client()?;
        let url = format!("{}/api/chat", self.base_url);
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": false,
        });
        if with_tools {
            let defs = crate::tools::native_tool_defs();
            if !defs.is_empty() {
                body["tools"] = serde_json::Value::Array(defs);
            }
        }
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .map_err(|e| map_reqwest_error(&self.base_url, e))?;

        let status = resp.status();
        let text = resp
            .text()
            .map_err(|e| format!("读取 Ollama 响应失败：{e}"))?;
        if !status.is_success() {
            // 模型不支持 tools → 回退纯文本协议再试一次（只发生一次）。
            if with_tools && text.contains("does not support tools") {
                return Err("OLLAMA_TOOLS_UNSUPPORTED".to_string());
            }
            // 404 model not found 是最常见的首次使用错误：给出可直接执行的修复命令。
            if status.as_u16() == 404 && text.to_ascii_lowercase().contains("not found") {
                return Err(format!(
                    "模型「{}」未安装。请先在终端运行 `ollama pull {}`，\
                     或在对话页顶部切换到已安装的模型 / DeepSeek 网页。",
                    self.model, self.model
                ));
            }
            return Err(format!(
                "Ollama 返回 HTTP {}：{}",
                status.as_u16(),
                truncate(&text, 500)
            ));
        }
        let parsed: ChatResponse = serde_json::from_str(&text)
            .map_err(|e| format!("Ollama 响应不是合法 JSON：{e}；原文：{}", truncate(&text, 300)))?;
        if let Some(err) = parsed.error.as_deref() {
            return Err(format!("Ollama 错误：{err}"));
        }
        Ok(parsed)
    }

    fn send_with_fallback(
        &self,
        messages: &[OllamaMessage],
    ) -> Result<ChatResponse, String> {
        let unsupported = *self
            .native_tools_unsupported
            .lock()
            .map_err(|e| e.to_string())?;
        let with_tools = !unsupported && self.caps().supports_tools;
        match self.post_chat(messages, with_tools) {
            Ok(r) => Ok(r),
            Err(e) if e == "OLLAMA_TOOLS_UNSUPPORTED" => {
                if let Ok(mut u) = self.native_tools_unsupported.lock() {
                    *u = true;
                }
                crate::log::chain_log(&format!(
                    "ollama: 模型「{}」不支持原生 tools，回退文本协议",
                    self.model
                ));
                self.post_chat(messages, false)
            }
            Err(e) => Err(e),
        }
    }
}

impl ChatProvider for OllamaProvider {
    fn id(&self) -> &'static str {
        "ollama"
    }

    fn label(&self) -> &'static str {
        "Ollama 本地模型"
    }

    fn caps(&self) -> ProviderCaps {
        ProviderCaps {
            supports_tools: true,
            streaming: false,
            max_tool_list_chars: crate::config::provider_cfg().ollama_max_tool_list_chars,
        }
    }

    fn reset(&self) -> Result<(), String> {
        self.messages
            .lock()
            .map_err(|e| e.to_string())?
            .clear();
        Ok(())
    }

    fn set_system_prompt(&self, prompt: &str) -> Result<(), String> {
        let mut m = self.messages.lock().map_err(|e| e.to_string())?;
        m.clear();
        if !prompt.trim().is_empty() {
            m.push(OllamaMessage {
                role: "system".to_string(),
                content: prompt.to_string(),
            });
        }
        Ok(())
    }

    fn send_turn(&self, message: &str) -> Result<TurnReply, String> {
        let mut messages = self
            .messages
            .lock()
            .map_err(|e| format!("Ollama 会话锁失败：{e}"))?;
        messages.push(OllamaMessage {
            role: "user".to_string(),
            content: message.to_string(),
        });

        let resp = match self.send_with_fallback(&messages) {
            Ok(r) => r,
            Err(e) => {
                // 失败不污染历史：撤掉刚加入的用户消息，允许用户重试。
                messages.pop();
                return Err(e);
            }
        };

        let reply = resp.message.unwrap_or_default();
        let content = reply.content.unwrap_or_default();
        let tool_calls = reply
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tc| tc.into_json_call())
            .collect::<Vec<_>>();

        // 工具调用轮次的 content 可能为空：空 assistant 消息会干扰后续对话，
        // 因此只在有正文时入历史；工具结果由下一轮 user 消息带回。
        if !content.trim().is_empty() {
            messages.push(OllamaMessage {
                role: "assistant".to_string(),
                content: content.clone(),
            });
        }

        if tool_calls.is_empty() && content.trim().is_empty() {
            return Err("Ollama 返回了空回复（模型可能未正确加载）".to_string());
        }
        Ok(TurnReply { text: content, tool_calls })
    }
}

// ---------- JSON 结构 ----------

#[derive(Deserialize)]
struct ChatResponse {
    message: Option<ReplyMessage>,
    error: Option<String>,
}

#[derive(Deserialize, Default)]
struct ReplyMessage {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Deserialize)]
struct ToolCall {
    function: ToolFunction,
}

#[derive(Deserialize)]
struct ToolFunction {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

impl ToolCall {
    fn into_json_call(self) -> Option<JsonCall> {
        let name = self.function.name.trim().to_string();
        if name.is_empty() {
            return None;
        }
        // Ollama 不同版本可能把 arguments 给成 JSON 字符串或对象。
        let arguments = match self.function.arguments {
            serde_json::Value::Object(_) => self.function.arguments,
            serde_json::Value::String(s) => serde_json::from_str(&s).ok()?,
            serde_json::Value::Null => serde_json::json!({}),
            _ => return None,
        };
        if !arguments.is_object() {
            return None;
        }
        Some(JsonCall { name, arguments })
    }
}

// ---------- 模型列表 ----------

#[derive(Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    #[serde(default)]
    name: String,
    #[serde(default)]
    model: String,
}

/// 列出本地模型；连接失败返回可读错误（UI 用它显示"Ollama 未启动"）。
pub fn list_models(base_url: &str, timeout: Duration) -> Result<Vec<String>, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(timeout.min(Duration::from_secs(3)))
        .timeout(timeout)
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| map_reqwest_error(base_url, e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "Ollama /api/tags 返回 HTTP {}",
            resp.status().as_u16()
        ));
    }
    let parsed: TagsResponse = resp
        .json()
        .map_err(|e| format!("Ollama /api/tags 响应解析失败：{e}"))?;
    let mut names: Vec<String> = parsed
        .models
        .into_iter()
        .map(|m| if m.name.trim().is_empty() { m.model } else { m.name })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

fn map_reqwest_error(base_url: &str, e: reqwest::Error) -> String {
    if e.is_timeout() {
        format!("连接 Ollama 超时（{base_url}）")
    } else if e.is_connect() {
        format!("连接不上 Ollama（{base_url}）；请先运行 `ollama serve` 并确认端口 11434")
    } else {
        format!("Ollama 请求失败：{e}")
    }
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// 起一个只服务一次请求的本地 HTTP 服务，返回 base_url。
    fn serve_once(response_json: &'static str, capture: Option<std::sync::mpsc::Sender<String>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf: Vec<u8> = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let n = s.read(&mut tmp).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4) else {
                        continue;
                    };
                    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
                    let cl = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    while buf.len() < header_end + cl {
                        let n = s.read(&mut tmp).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    break;
                }
                if let Some(tx) = capture {
                    let _ = tx.send(String::from_utf8_lossy(&buf).to_string());
                }
                let body = response_json.as_bytes();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes());
                let _ = s.write_all(body);
            }
        });
        format!("http://{addr}")
    }

    fn test_cfg(base_url: String) -> ProviderCfg {
        ProviderCfg {
            active: "ollama".to_string(),
            ollama_base_url: base_url,
            ollama_model: "test-model".to_string(),
            ollama_connect_timeout_sec: 2,
            ollama_request_timeout_sec: 5,
            deepseek_max_tool_list_chars: 4000,
            ollama_max_tool_list_chars: 6000,
        }
    }

    #[test]
    fn list_models_parses_tags_endpoint() {
        let base = serve_once(r#"{"models":[{"name":"m1"},{"model":"m2"}]}"#, None);
        let got = list_models(&base, Duration::from_secs(2)).unwrap();
        assert_eq!(got, vec!["m1".to_string(), "m2".to_string()]);
    }

    #[test]
    fn send_turn_posts_system_and_user_and_parses_native_tool_call() {
        let (tx, rx) = std::sync::mpsc::channel();
        let base = serve_once(
            r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"calc","arguments":{"expr":"1+2"}}}]}}"#,
            Some(tx),
        );
        let p = OllamaProvider::new(&test_cfg(base));
        p.set_system_prompt("SYS").unwrap();
        let reply = p.send_turn("hi").unwrap();
        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].name, "calc");
        assert_eq!(reply.tool_calls[0].arguments["expr"], "1+2");

        let req = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(req.contains("\"role\":\"system\""));
        assert!(req.contains("\"content\":\"SYS\""));
        assert!(req.contains("\"content\":\"hi\""));
        assert!(req.contains("\"tools\""));
        assert!(req.contains("\"stream\":false"));
    }

    #[test]
    fn failed_turn_does_not_pollute_history() {
        // 服务端返回 500 后再关闭：send_turn 必须报错，且不再把用户消息留在历史里。
        //
        // 为什么外层重试：本用例依赖一个**本地 TCP mock**。机器负载高时（例如刚编译完、
        // 还有几十个测试并发跑），客户端可能在 mock 写出响应之前就失败，
        // 于是拿到的是连接层错误而不是 "HTTP 500" —— 与用例意图无关。
        // 实测全量跑三次会偶发一次失败。因此每次**重新建 mock 与 provider**，
        // 只有连续三次都拿不到 HTTP 500 才算真失败 —— 不会掩盖 http 层行为。
        let mut last = String::new();
        for _attempt in 0..3 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            std::thread::spawn(move || {
                if let Ok((mut s, _)) = listener.accept() {
                    let mut tmp = [0u8; 8192];
                    let _ = s.read(&mut tmp);
                    let body = "boom";
                    let resp = format!(
                        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = s.write_all(resp.as_bytes());
                    let _ = s.flush();
                }
            });
            let p = OllamaProvider::new(&test_cfg(format!("http://{addr}")));
            p.set_system_prompt("SYS").unwrap();
            match p.send_turn("hi") {
                Ok(_) => panic!("应报错但成功了"),
                Err(e) => {
                    if e.contains("HTTP 500") {
                        // 历史应只剩 system 一条
                        let n = p.messages.lock().unwrap().len();
                        assert_eq!(n, 1, "失败回合不得把用户消息留在历史里");
                        return;
                    }
                    last = e;
                }
            }
        }
        panic!("三次尝试都没拿到 HTTP 500，最后一次错误：{last}");
    }

    /// 起一个**一次性**本地 mock：accept → read → 回固定响应。
    ///
    /// 抽出来是因为多个用例都要它，而且它们都踩过同一个坑（见下）。
    fn spawn_mock(resp: String) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut tmp = [0u8; 8192];
                let _ = s.read(&mut tmp);
                let _ = s.write_all(resp.as_bytes());
                let _ = s.flush();
            }
        });
        addr
    }

    /// 对**连接层**失败重试的通用包装。
    ///
    /// 为什么需要：这些用例都依赖本地 TCP mock。机器负载高时（刚编译完 + 几十个测试并发），
    /// 客户端可能在 mock 写出响应之前就报 "error sending request" —— 与用例意图无关。
    /// 实测全量跑约 1/4 概率命中。每次重试都**重建 mock 与 provider**（不能复用旧 provider，
    /// 它会连到已关闭的端口）。连续三次都失败才算真失败，不会掩盖 http 层行为。
    fn retry_until_mock_answers<F>(mut attempt: F) -> String
    where
        F: FnMut() -> Result<(), String>,
    {
        let mut last = String::new();
        for _ in 0..3 {
            match attempt() {
                Ok(()) => return String::new(),
                Err(e) => {
                    // 只有连接层错误才重试；一旦拿到 http 层错误就是真结果
                    if !e.contains("error sending request") && !e.contains("连接") {
                        return e;
                    }
                    last = e;
                }
            }
        }
        last
    }

    #[test]
    fn missing_model_error_is_actionable() {
        let last = retry_until_mock_answers(|| {
            let addr = spawn_mock(format!(
                "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                r#"{"error":"model 'test-model' not found"}"#.len(),
                r#"{"error":"model 'test-model' not found"}"#
            ));
            let p = OllamaProvider::new(&test_cfg(format!("http://{addr}")));
            p.set_system_prompt("SYS").unwrap();
            let err = p.send_turn("hi").unwrap_err();
            assert!(err.contains("未安装"), "got: {err}");
            assert!(err.contains("ollama pull test-model"), "got: {err}");
            Ok(())
        });
        assert!(last.is_empty(), "三次尝试仍失败：{last}");
    }

    #[test]
    fn tool_call_arguments_object_is_parsed() {
        let tc: ToolCall = serde_json::from_str(
            r#"{"function":{"name":"calc","arguments":{"expr":"1+2"}}}"#,
        )
        .unwrap();
        let call = tc.into_json_call().unwrap();
        assert_eq!(call.name, "calc");
        assert_eq!(call.arguments["expr"], "1+2");
    }

    #[test]
    fn tool_call_arguments_json_string_is_parsed() {
        let tc: ToolCall = serde_json::from_str(
            r#"{"function":{"name":"now","arguments":"{}"}}"#,
        )
        .unwrap();
        let call = tc.into_json_call().unwrap();
        assert_eq!(call.name, "now");
        assert!(call.arguments.is_object());
    }

    #[test]
    fn tool_call_with_empty_name_is_dropped() {
        let tc: ToolCall =
            serde_json::from_str(r#"{"function":{"name":"  ","arguments":{}}}"#).unwrap();
        assert!(tc.into_json_call().is_none());
    }

    #[test]
    fn tags_response_accepts_name_and_model_fields() {
        let v: TagsResponse = serde_json::from_str(
            r#"{"models":[{"name":"qwen2.5:7b"},{"model":"llama3.2"}]}"#,
        )
        .unwrap();
        let names: Vec<String> = v
            .models
            .into_iter()
            .map(|m| if m.name.is_empty() { m.model } else { m.name })
            .collect();
        assert_eq!(names, vec!["qwen2.5:7b", "llama3.2"]);
    }

    #[test]
    fn error_truncation_is_char_safe() {
        let s = "错误".repeat(1000);
        assert_eq!(truncate(&s, 5).chars().count(), 5);
    }
}
