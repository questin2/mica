//! CDP 基建：进程内 CallDevToolsProtocolMethod 直调。
//!
//! 拆分自原单文件 `lib.rs`（可扩展性分析报告 v2 · S1）。
//! `use super::*` 继承根模块的导入与共享项，避免重复声明。

use super::*;

// ---------- CDP 基建 ----------

pub(crate) fn cdp_call(wv: &tauri::webview::Webview, method: &str, params_json: &str) -> Result<String, String> {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;

    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    let tx_handler = tx.clone();
    let method_owned = method.to_string();
    let params_owned = params_json.to_string();

    wv.with_webview(move |pw| {
        let controller = pw.controller();
        let core = match unsafe { controller.CoreWebView2() } {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(Err(format!("CoreWebView2 failed: {e}")));
                return;
            }
        };
        let handler =
            CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |error, result| {
                match error {
                    Err(e) => {
                        let _ = tx_handler.send(Err(format!("CDP error: {e}")));
                    }
                    Ok(()) => {
                        let _ = tx_handler.send(Ok(result));
                    }
                }
                Ok(())
            }));
        let m = HSTRING::from(method_owned.as_str());
        let p = HSTRING::from(params_owned.as_str());
        if let Err(e) = unsafe { core.CallDevToolsProtocolMethod(&m, &p, &handler) } {
            let _ = tx.send(Err(format!("CallDevToolsProtocolMethod failed: {e}")));
        }
    })
    .map_err(|e| format!("with_webview: {e}"))?;

    rx.recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|_| Err("CDP timeout".into()))
}

pub(crate) fn json_path(raw: &str, path: &[&str]) -> Option<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let mut cur = &v;
    for k in path {
        cur = cur.get(k)?;
    }
    Some(cur.clone())
}

/// 执行 JS 表达式并取回字符串值（表达式返回对象，勿再 JSON.stringify）
pub(crate) fn cdp_eval(wv: &tauri::webview::Webview, expr_js: &str) -> Result<String, String> {
    let expr = serde_json::json!({ "expression": expr_js, "returnByValue": true }).to_string();
    let raw = cdp_call(wv, "Runtime.evaluate", &expr)?;
    json_path(&raw, &["result", "value"])
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| format!("no value in: {}", raw.chars().take(200).collect::<String>()))
}
