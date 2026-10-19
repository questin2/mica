//! 网络层：CDP Network 事件订阅 + SSE 回复流解析（替代 DOM 类名抓取）。
//!
//! 拆分自原单文件 `lib.rs`（可扩展性分析报告 v2 · S1）。
//! `use super::*` 继承根模块的导入与共享项，避免重复声明。

use super::*;

// ---------- 网络层捕获：CDP Network 事件订阅（替代 DOM 类名抓取的回复流信源） ----------

/// 需要订阅的 Network 域事件。DeepSeek 网页版流式回复经 WebSocket 帧下发；
/// 同时订阅 HTTP 响应/加载完成作为「回复流结束」与协议探测的旁证。
pub(crate) const NET_EVENTS: &[&str] = &[
    "Network.webSocketCreated",
    "Network.webSocketFrameReceived",
    "Network.webSocketFrameSent",
    "Network.webSocketClosed",
    "Network.responseReceived",
    "Network.loadingFinished",
    "Network.dataReceived",
];

/// 订阅 DeepSeek WebView 的 CDP 网络事件，写入 `NetCapture` 环形缓冲。
/// 幂等：enabled 置位后不再重复订阅。必须在 WebView 就绪后调用。
pub(crate) fn net_subscribe(app: &tauri::AppHandle) -> Result<(), String> {
    use webview2_com::DevToolsProtocolEventReceivedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2DevToolsProtocolEventReceivedEventArgs,
        ICoreWebView2DevToolsProtocolEventReceiver,
    };
    use windows::core::{HSTRING, PWSTR};

    let state = app.state::<EngineState>();
    if state.net.enabled.load(Ordering::SeqCst) {
        return Ok(());
    }

    // 先启用 Network 域（事件订阅生效前提）。
    cdp_call(
        &app.get_webview("deepseek").ok_or("deepseek webview not found")?,
        "Network.enable",
        "{}",
    )
    .map_err(|e| format!("Network.enable failed: {e}"))?;

    let net = Arc::clone(&state.net);
    let wv = app.get_webview("deepseek").ok_or("deepseek webview not found")?;
    // 订阅成功的事件名（闭包内写入；闭包异步执行，故日志也在闭包内打印）
    let tokens_clone: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    wv.with_webview(move |pw| {
        let controller = pw.controller();
        let core = match unsafe { controller.CoreWebView2() } {
            Ok(c) => c,
            Err(e) => {
                eprintln!("net_subscribe: CoreWebView2 failed: {e}");
                return;
            }
        };

        for ev_name in NET_EVENTS {
            // 每个事件名取一个 receiver，再挂处理器。失败（如该事件名不存在）不阻断整体。
            let receiver: ICoreWebView2DevToolsProtocolEventReceiver =
                match unsafe { core.GetDevToolsProtocolEventReceiver(&HSTRING::from(*ev_name)) } {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("net_subscribe: GetDevToolsProtocolEventReceiver({ev_name}) failed: {e}");
                        continue;
                    }
                };
            let net_clone = Arc::clone(&net);
            let ev_owned = (*ev_name).to_string();
            let handler = DevToolsProtocolEventReceivedEventHandler::create(Box::new(
                move |_sender: Option<ICoreWebView2>,
                      args: Option<ICoreWebView2DevToolsProtocolEventReceivedEventArgs>| {
                    let Some(args) = args else { return Ok(()) };
                    let mut p = PWSTR::null();
                    if unsafe { args.ParameterObjectAsJson(&mut p) }.is_err() {
                        return Ok(());
                    }
                    let json = if p.is_null() {
                        String::new()
                    } else {
                        webview2_com::take_pwstr(p)
                    };
                    if let Ok(mut q) = net_clone.events.lock() {
                        if q.len() >= 4096 {
                            q.pop_front();
                        }
                        q.push_back(NetEvent {
                            ts_ms: now_ms(),
                            method: ev_owned.clone(),
                            params: json,
                        });
                    }
                    Ok(())
                },
            ));
            let mut token: i64 = 0;
            if unsafe { receiver.add_DevToolsProtocolEventReceived(&handler, &mut token) }.is_ok() {
                if let Ok(mut t) = tokens_clone.lock() {
                    t.push((*ev_name).to_string());
                }
            } else {
                eprintln!("net_subscribe: add handler failed for {ev_name}");
            }
        }

        // 注意：with_webview 的闭包是异步派发到 WebView 线程执行的，
        // 因此「订阅成功」的日志必须在这里打印（在闭包外读 tokens 会恒为空）。
        let subscribed: Vec<String> = tokens_clone.lock().map(|v| v.clone()).unwrap_or_default();
        chain_log(&format!(
            "net_subscribe: Network.enable + {} 个事件已订阅: {}",
            subscribed.len(),
            subscribed.join(", ")
        ));
        eprintln!("net_subscribe: {} events subscribed: {:?}", subscribed.len(), subscribed);
    })
    .map_err(|e| format!("with_webview failed: {e}"))?;

    // 标记置位：闭包可能稍后才执行，但重复调用会被此标志拦下（幂等）。
    state.net.enabled.store(true, Ordering::SeqCst);
    Ok(())
}

/// 清空网络事件缓冲（每次回合开始前调用，只保留本回合的网络活动）。
pub(crate) fn net_clear(app: &tauri::AppHandle) {
    if let Ok(mut q) = app.state::<EngineState>().net.events.lock() {
        q.clear();
    }
}

/// 自订阅起累积的、匹配给定 Network 方法的事件数。
pub(crate) fn net_count(app: &tauri::AppHandle, method_prefix: &str) -> usize {
    let state = app.state::<EngineState>();
    let Ok(q) = state.net.events.lock() else {
        return 0;
    };
    q.iter().filter(|e| e.method.starts_with(method_prefix)).count()
}

/// 取回累积的网络事件原始 JSON 行（探针/诊断用，最多 200 条）。
pub(crate) fn net_dump(app: &tauri::AppHandle) -> String {
    let state = app.state::<EngineState>();
    let Ok(q) = state.net.events.lock() else {
        return "[]".to_string();
    };
    let mut out = Vec::new();
    for e in q.iter().rev().take(200) {
        out.push(serde_json::json!({ "method": e.method, "ts_ms": e.ts_ms, "params": e.params }));
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}

// ---------- 回复流：SSE 端点识别 / 流结束判定 / 正文读取 ----------

// 【S3】回复端点已外置到 config/agent.json 的 replyEndpoint（默认值见 config::AgentCfg::default）。

pub(crate) fn params_request_id(params: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(params)
        .ok()?
        .get("requestId")?
        .as_str()
        .map(|s| s.to_string())
}

pub(crate) fn params_response_url(params: &str) -> String {
    serde_json::from_str::<serde_json::Value>(params)
        .ok()
        .and_then(|v| v.pointer("/response/url").and_then(|x| x.as_str()).map(|s| s.to_string()))
        .unwrap_or_default()
}

/// 最近一次回复流（端点见 config/agent.json → replyEndpoint）的 requestId。这是本回合的唯一权威标识。
pub(crate) fn net_find_reply_request(app: &tauri::AppHandle) -> Option<String> {
    let state = app.state::<EngineState>();
    let endpoint = config::agent_cfg().reply_endpoint;
    let q = state.net.events.lock().ok()?;
    let mut found = None;
    for e in q.iter() {
        if e.method == "Network.responseReceived" && params_response_url(&e.params).contains(endpoint.as_str()) {
            if let Some(rid) = params_request_id(&e.params) {
                found = Some(rid);
            }
        }
    }
    found
}

/// 回复流是否已结束（`Network.loadingFinished`）。SSE 关闭即模型生成完毕 ——
/// 这是替代「SVG 图标魔法字符串」的完成判定主信号。
pub(crate) fn net_reply_stream_finished(app: &tauri::AppHandle, request_id: &str) -> bool {
    let state = app.state::<EngineState>();
    let Ok(q) = state.net.events.lock() else {
        return false;
    };
    q.iter().any(|e| {
        e.method == "Network.loadingFinished"
            && params_request_id(&e.params).as_deref() == Some(request_id)
    })
}

/// 该 reply request 已收到的数据字节数（用于「流仍在增长」判定）。
pub(crate) fn net_reply_bytes(app: &tauri::AppHandle, request_id: &str) -> usize {
    let state = app.state::<EngineState>();
    let Ok(q) = state.net.events.lock() else {
        return 0;
    };
    q.iter()
        .filter(|e| e.method == "Network.dataReceived" && params_request_id(&e.params).as_deref() == Some(request_id))
        .filter_map(|e| {
            serde_json::from_str::<serde_json::Value>(&e.params)
                .ok()?
                .get("dataLength")?
                .as_u64()
        })
        .sum::<u64>() as usize
}

/// 直接读取回复流正文（`Network.getResponseBody`）——彻底摆脱 CSS 类名依赖。
pub(crate) fn net_read_reply_body(app: &tauri::AppHandle, request_id: &str) -> Result<String, String> {
    let wv = app.get_webview("deepseek").ok_or("deepseek webview not found")?;
    let params = serde_json::json!({ "requestId": request_id }).to_string();
    let raw = cdp_call(&wv, "Network.getResponseBody", &params)?;
    let body = json_path(&raw, &["body"])
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let is_b64 = json_path(&raw, &["base64Encoded"]).and_then(|v| v.as_bool()).unwrap_or(false);
    if is_b64 {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(body.as_bytes())
            .map_err(|e| format!("base64 decode failed: {e}"))?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    } else {
        Ok(body)
    }
}

/// 从 SSE 正文中提取 assistant 回答（仅 RESPONSE 片段，排除 THINK 思考内容）。
///
/// DeepSeek 网页版 SSE 补丁协议（2026-10-17 实测）：
///   - 初始全量：`data: {"v":{"response":{"status":"WIP","fragments":[{"type":"THINK","content":".."}]}}}`
///   - 追加片段：`data: {"p":"response/fragments","o":"APPEND","v":[{"type":"RESPONSE","content":".."}]}`
///   - 追加内容：`data: {"p":"response/fragments/-1/content","o":"APPEND","v":".."}`（无 `o` 视为 APPEND）
///   - 粘性路径：`data: {"v":".."}` 续写上一个 `p` 指向的位置
///   - 批量补丁：`data: {"p":"response","o":"BATCH","v":[{"p":..,"v":..}]}`
///   - 完成状态：`data: {"p":"response/status","o":"SET","v":"FINISHED"}`
pub(crate) fn parse_sse_assistant_text(body: &str) -> String {
    let mut frags: Vec<(String, String)> = Vec::new();
    let mut cur_path = String::new();
    // 容错：若上游改用 OpenAI 风格增量（`choices[0].delta.content`），单独累积。
    let mut openai_fallback = String::new();

    for line in body.lines() {
        let line = line.trim();
        let Some(payload) = line.strip_prefix("data:") else {
            continue; // 跳过 event: 行与空行
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue;
        };

        // OpenAI 风格增量（容错路径）
        for ptr in ["/choices/0/delta/content", "/choices/0/message/content"] {
            if let Some(s) = v.pointer(ptr).and_then(|x| x.as_str()) {
                openai_fallback.push_str(s);
            }
        }

        // 初始全量状态（含 fragments 数组）
        if let Some(resp) = v.get("v").and_then(|x| x.pointer("/response")) {
            if let Some(arr) = resp.get("fragments").and_then(|f| f.as_array()) {
                frags = arr.iter().map(frag_from_json).collect();
            }
            continue;
        }

        if let Some(p) = v.get("p").and_then(|x| x.as_str()) {
            cur_path = p.to_string();
            if v.get("o").and_then(|x| x.as_str()) == Some("BATCH") {
                if let Some(arr) = v.get("v").and_then(|x| x.as_array()) {
                    for sub in arr {
                        if let Some(sp) = sub.get("p").and_then(|x| x.as_str()) {
                            apply_fragment_patch(
                                &mut frags,
                                sp,
                                sub.get("o").and_then(|x| x.as_str()),
                                sub.get("v"),
                            );
                        }
                    }
                }
                continue;
            }
            apply_fragment_patch(&mut frags, p, v.get("o").and_then(|x| x.as_str()), v.get("v"));
            continue;
        }

        // 无 p：续写粘性路径
        if let Some(val) = v.get("v") {
            apply_fragment_patch(&mut frags, &cur_path, Some("APPEND"), Some(val));
        }
    }

    let deepseek_text = frags
        .iter()
        .filter(|(t, _)| t == "RESPONSE")
        .map(|(_, c)| c.as_str())
        .collect::<String>()
        .trim()
        .to_string();
    if !deepseek_text.is_empty() {
        return deepseek_text;
    }
    openai_fallback.trim().to_string()
}

pub(crate) fn frag_from_json(f: &serde_json::Value) -> (String, String) {
    (
        f.get("type").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        f.get("content").and_then(|x| x.as_str()).unwrap_or("").to_string(),
    )
}

pub(crate) fn apply_fragment_patch(
    frags: &mut Vec<(String, String)>,
    path: &str,
    op: Option<&str>,
    val: Option<&serde_json::Value>,
) {
    let Some(val) = val else { return };
    // 追加片段（数组形式）
    if path == "response/fragments" {
        if let Some(arr) = val.as_array() {
            for f in arr {
                frags.push(frag_from_json(f));
            }
        }
        return;
    }
    // 追加/设置某片段的内容字段
    if let Some(rest) = path.strip_prefix("response/fragments/") {
        let mut parts = rest.splitn(2, '/');
        let idx_s = parts.next().unwrap_or("");
        let field = parts.next().unwrap_or("");
        if field != "content" {
            return;
        }
        let idx: isize = idx_s.parse().unwrap_or(-1); // -1 = 最后一片段
        let real = if idx < 0 {
            match frags.len().checked_sub((-idx) as usize) {
                Some(i) => i,
                None => return,
            }
        } else {
            idx as usize
        };
        if let Some(f) = frags.get_mut(real) {
            if let Some(s) = val.as_str() {
                match op.unwrap_or("APPEND") {
                    "SET" => f.1 = s.to_string(),
                    _ => f.1.push_str(s),
                }
            }
        }
    }
}

/// SSE 正文是否已宣告生成完成（`response/status` → `FINISHED`）。
/// 这是流内的权威完成标志，可与 `Network.loadingFinished` 互为佐证。
pub(crate) fn sse_declares_finished(body: &str) -> bool {
    body.contains("\"quasi_status\",\"v\":\"FINISHED\"")
        || body.contains("\"response/status\",\"o\":\"SET\",\"v\":\"FINISHED\"")
        || body.contains("\"status\":\"FINISHED\"")
}

/// DOM 兜底读回的「回声检测」：识别读回内容其实就是自己刚发出去的用户消息。
/// 这是空间启发式抓取最典型的误读（长提示词注入时尤其明显）——
/// 返回 true 表示该结果不可信，应继续等待真正的助手回复。
pub(crate) fn looks_like_echo(readback: &str, sent: &str) -> bool {
    let r = readback.trim();
    let s = sent.trim();
    if r.is_empty() || s.is_empty() {
        return false;
    }
    // 读回与已发送消息高度重合，且未比其显著更长 → 判为回声
    let rn = r.chars().count();
    let sn = s.chars().count();
    rn <= sn + sn / 5 && (r == s || r.contains(s) || s.contains(r))
}


/// 从 SSE 正文里提取**思考过程**（DeepSeek 网页把思考放在 fragments 里）。
///
/// 依据（app/data/tmp/sse_body_snapshot.txt 实测）：
/// ```json
/// "response": {
///   "thinking_enabled": true,
///   "fragments": [
///     { "id": 2, "type": "THINK",    "content": "用户..." },
///     { "id": ?, "type": "RESPONSE", "content": "..." }
///   ]
/// }
/// ```
/// 只有两种 type：`THINK` 与 `RESPONSE`。取**最后一条**含 fragments 的事件
/// （流里是累加式的，最后一个最全），把 type==THINK 的 content 按出现顺序拼起来。
///
/// 返回空串表示这一轮没有思考过程（未开思考、或还没开始）。
pub(crate) fn parse_sse_thinking(body: &str) -> String {
    // 【真实协议，2026-10-19 从 app/data/sse_body.txt 实测得到】
    // 思考内容是**增量分片**推上来的，两种形态都会出现：
    //   data: {"p":"response/fragments/-1/content","o":"APPEND","v":"就"}   ← 路径追加
    //   data: {"v":"说了"}  data: {"v":"句"}  ...                           ← 纯文本续写
    // 把这些按顺序拼起来才是完整思考（实测：就说了句"你好"。没啥任务。…）
    // 而 fragments 元数据里的 content 只有 "用户"（占位/标签），**不能**当思考用。
    //
    // 判定规则：Thinking 阶段从流开始；一旦出现 type=="RESPONSE" 的片段，
    // 之后的分片属于正文，不再计入思考。
    let mut thinking = String::new();
    let mut in_think = true;
    for line in body.lines() {
        let Some(rest) = line.strip_prefix("data:") else { continue };
        let t = rest.trim();
        if !t.starts_with('{') {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else { continue };

        // 阶段切换：出现 RESPONSE 片段后，后续分片属于正文
        if let Some(frags) = v.pointer("/v/response/fragments").and_then(|f| f.as_array()) {
            for f in frags {
                if f.get("type").and_then(|x| x.as_str()) == Some("RESPONSE") {
                    in_think = false;
                }
            }
        }
        // 批量路径事件里也可能带 RESPONSE 片段
        if let Some(arr) = v.pointer("/v").and_then(|x| x.as_array()) {
            for f in arr {
                if f.get("type").and_then(|x| x.as_str()) == Some("RESPONSE") {
                    in_think = false;
                }
            }
        }
        if !in_think {
            continue;
        }

        // ① 路径追加：{"p":"…/content","o":"APPEND","v":"就"}
        // ② 纯文本续写：{"v":"说了"}
        // 其它带 v 对象的事件（初始状态等）不看，只取字符串分片。
        if let Some(s) = v.get("v").and_then(|x| x.as_str()) {
            // 过滤协议标记词（{"v":"thinking"} / {"v":"FINISHED"} 之类）
            if s != "thinking" && s != "FINISHED" && !s.is_empty() {
                thinking.push_str(s);
            }
        }
    }
    thinking.trim().to_string()
}

#[cfg(test)]
mod thinking_tests {
    use super::parse_sse_thinking;

    // 真实形态（2026-10-19 修正）：同一 id 的 content 是**累加式**的，
    // 后到的事件里同一 id 内容更长；不同 id 是不同 stage，内容不重叠。
    // 真实形态（2026-10-19 取自 app/data/sse_body.txt）：
    // 思考是增量分片；fragments 元数据里的 content 只有 "用户"（占位）。
    const SAMPLE: &str = r#"event: ready
data: {"request_message_id":5,"response_message_id":6,"model_type":"default"}

data: {"v":{"response":{"message_id":6,"thinking_enabled":true,"fragments":[{"id":2,"type":"THINK","content":"用户"}],"status":"WIP"}}}

data: {"p":"response/fragments/-1/content","o":"APPEND","v":"就"}
data: {"v":"说了"}
data: {"v":"句“你好”。"}
data: {"p":"response/fragments","o":"APPEND","v":[{"id":4,"type":"RESPONSE","content":"你好，我是 Agent。"}]}
data: {"v":"这些属于正文，不该进思考"}
"#;

    #[test]
    fn extracts_think_fragments_only() {
        // 按 id 跨事件聚合：同一 id 取最长（累加式），只拼 THINK，不含 RESPONSE
        assert_eq!(
            parse_sse_thinking(SAMPLE),
            "就说了句“你好”。"
        );
    }

    #[test]
    fn no_thinking_returns_empty() {
        let s = r#"data: {"v":{"response":{"fragments":[{"id":4,"type":"RESPONSE","content":"只有答案"}]}}}"#;
        assert_eq!(parse_sse_thinking(s), "");
    }

    #[test]
    fn garbage_is_tolerated() {
        assert_eq!(parse_sse_thinking(""), "");
        assert_eq!(parse_sse_thinking("data: not json\n\n"), "");
    }
}
