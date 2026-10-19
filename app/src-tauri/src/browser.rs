//! **真实浏览器工具**（策划案 2.0 · 能力层）
//!
//! 与 `builtin::web_read` 的分工：
//! - `web_read`：纯 HTTP 拉取 → HTML 转文本。**快、省**，但拿不到 JS 渲染出来的内容；
//! - 本模块：用一个**离屏 webview** 真正加载页面，能读到渲染后的 DOM，并能点击 / 填表。
//!
//! # 为什么用离屏 webview
//! 现有两个 webview（`shell` 应用界面、`deepseek` 模型页）都不能动：
//! 前者是 UI，后者一旦导航就会丢掉模型会话。所以第三个 webview 放在**窗口外**
//! （负坐标），用户永远看不到，但它是一个完整的 Chromium 渲染上下文。
//!
//! # 安全边界
//! - 导航前复用 `builtin::validate_public_url`（与 `web_read` 同一套 SSRF 防护）；
//! - 只允许 http/https / about:blank；
//! - 读回内容有长度上限，避免把整页 DOM 灌进提示词。

use super::*;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{LogicalPosition, LogicalSize, Manager, WebviewUrl};

/// 离屏窗口位置：远在可视区之外，用户不可见。
const OFFSCREEN_X: f64 = -4000.0;
const OFFSCREEN_Y: f64 = -4000.0;
const VIEW_W: f64 = 1280.0;
const VIEW_H: f64 = 900.0;

/// 读回文本上限（字符）
const TEXT_CAP: usize = 20_000;

/// 懒创建（只建一次）。用 Mutex 保证并发首次调用不会建两个。
static CREATED: Mutex<bool> = Mutex::new(false);

/// 取（必要时创建）离屏浏览器 webview。
pub(crate) fn ensure(app: &tauri::AppHandle) -> Result<tauri::Webview, String> {
    if let Some(wv) = app.get_webview("web") {
        return Ok(wv);
    }
    let mut guard = CREATED.lock().map_err(|e| e.to_string())?;
    if let Some(wv) = app.get_webview("web") {
        return Ok(wv);
    }
    let win = app.get_window("main").ok_or("找不到主窗口")?;
    let wv = win
        .add_child(
            tauri::webview::WebviewBuilder::new(
                "web",
                WebviewUrl::External("about:blank".parse().map_err(|e| format!("{e}"))?),
            )
            // ★ 独立 user-data 目录：否则新 webview 会与 DeepSeek 共用 cookie，
            // 访问任意站点时存在 cookie-stuffing / CSRF 面。隔离后互不可见。
            .data_directory(std::path::PathBuf::from(format!(
                r"{}\web_profile",
                crate::config::data_dir()
            )))
            .auto_resize(),
            LogicalPosition::new(OFFSCREEN_X, OFFSCREEN_Y),
            LogicalSize::new(VIEW_W, VIEW_H),
        )
        .map_err(|e| format!("创建浏览器 webview 失败：{e}"))?;
    *guard = true;
    crate::log::chain_log("browser: 离屏 webview 已创建");
    Ok(wv)
}

/// 解析并校验目标 URL（复用 web_read 的 SSRF 防护）。
fn check_url(url: &str) -> Result<reqwest::Url, String> {
    let u = reqwest::Url::parse(url.trim()).map_err(|e| format!("URL 不合法：{e}"))?;
    match u.scheme() {
        "http" | "https" | "about" => {}
        other => return Err(format!("只允许 http/https，收到：{other}")),
    }
    if u.scheme() != "about" {
        crate::builtin::validate_public_url(&u)?;
    }
    Ok(u)
}

/// 打开一个 URL，等到渲染稳定后返回**可见文本**。
///
/// 等待策略：`document.readyState === 'complete'` 后，再连续两次采样文本长度不变即视为稳定
/// （SPA 常常 readyState 完成后才把内容填进去）。总预算 `wait_ms`。
pub(crate) fn browse(app: &tauri::AppHandle, url: &str, wait_ms: u64) -> Result<String, String> {
    let target = check_url(url)?;
    let wv = ensure(app)?;
    wv.navigate(target.clone())
        .map_err(|e| format!("导航失败：{e}"))?;

    let budget = Duration::from_millis(wait_ms.clamp(500, 30_000));
    let t0 = Instant::now();
    let mut last_len = 0usize;
    let mut stable = 0;
    std::thread::sleep(Duration::from_millis(400));
    loop {
        if t0.elapsed() > budget {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
        let ready = cdp_eval(&wv, "document.readyState").unwrap_or_default();
        let ready = ready.contains("complete");
        let len = cdp_eval(&wv, "document.body ? document.body.innerText.length : 0")
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if ready && len > 0 {
            if len == last_len {
                stable += 1;
                if stable >= 2 {
                    break;
                }
            } else {
                stable = 0;
            }
            last_len = len;
        }
    }

    let title = cdp_eval(&wv, "document.title").unwrap_or_default();
    let text = read_text(&wv, TEXT_CAP)?;
    Ok(format!(
        "URL: {}\n标题: {}\n（离屏 Chromium 渲染后读回，{} 字符{}）\n\n{}",
        target,
        title.trim(),
        text.chars().count(),
        if text.chars().count() >= TEXT_CAP { "，已截断" } else { "" },
        text
    ))
}

/// 在当前页面里点击第一个匹配 `selector` 的元素，并返回点击后的可见文本。
pub(crate) fn click(app: &tauri::AppHandle, selector: &str, wait_ms: u64) -> Result<String, String> {
    let wv = ensure(app)?;
    let js = format!(
        "(function(){{var e=document.querySelector({sel});if(!e)return 'NOT_FOUND';e.scrollIntoView({{block:'center'}});e.click();return 'CLICKED';}})()",
        sel = serde_json::to_string(selector).map_err(|e| e.to_string())?
    );
    let r = cdp_eval(&wv, &js)?;
    if r.contains("NOT_FOUND") {
        return Err(format!("找不到元素：{selector}"));
    }
    std::thread::sleep(Duration::from_millis(wait_ms.clamp(200, 10_000)));
    let text = read_text(&wv, TEXT_CAP)?;
    Ok(format!("已点击 {selector}。当前页面文本：\n\n{text}"))
}

/// 在输入框里填值（派发 input/change 事件，React 等受控组件也能感知）。
pub(crate) fn fill(
    app: &tauri::AppHandle,
    selector: &str,
    value: &str,
    submit: bool,
) -> Result<String, String> {
    let wv = ensure(app)?;
    let js = format!(
        r#"(function(){{
  var e=document.querySelector({sel});
  if(!e) return 'NO_EL';
  var proto=Object.getPrototypeOf(e);
  var desc=Object.getOwnPropertyDescriptor(proto,'value');
  if(desc&&desc.set){{desc.set.call(e,{val});}}else{{e.value={val};}}
  e.dispatchEvent(new Event('input',{{bubbles:true}}));
  e.dispatchEvent(new Event('change',{{bubbles:true}}));
  {submit_js}
  return 'FILLED';
}})()"#,
        sel = serde_json::to_string(selector).map_err(|e| e.to_string())?,
        val = serde_json::to_string(value).map_err(|e| e.to_string())?,
        submit_js = if submit {
            "if(e.form&&e.form.requestSubmit){e.form.requestSubmit();}else if(e.form){e.form.submit();}"
        } else {
            ""
        }
    );
    let r = cdp_eval(&wv, &js)?;
    if r.contains("NO_EL") {
        return Err(format!("找不到输入框：{selector}"));
    }
    if submit {
        std::thread::sleep(Duration::from_millis(800));
    }
    let text = read_text(&wv, TEXT_CAP)?;
    Ok(format!("已填入 {selector}。当前页面文本：\n\n{text}"))
}

/// 读回当前页面的可见文本（不导航）。
pub(crate) fn read_current(app: &tauri::AppHandle) -> Result<String, String> {
    let wv = ensure(app)?;
    let url = cdp_eval(&wv, "location.href").unwrap_or_default();
    let text = read_text(&wv, TEXT_CAP)?;
    Ok(format!("当前 URL: {}\n\n{}", url.trim(), text))
}

/// `document.body.innerText` 是**渲染后可读文本**：
/// 不含 `<script>`、不含 `display:none` 的内容，和用户看到的一致。
fn read_text(wv: &tauri::Webview, cap: usize) -> Result<String, String> {
    let js = format!(
        "JSON.stringify((document.body ? document.body.innerText : '').slice(0, {}))",
        cap
    );
    let raw = cdp_eval(wv, &js)?;
    // cdp_eval 返回的是 JS 结果；若是 JSON 字符串则解出真实文本
    let out = serde_json::from_str::<String>(raw.trim()).unwrap_or(raw);
    Ok(out.trim().to_string())
}

// ---------- 看图：把本地图片附加到 DeepSeek 网页的输入区 ----------
//
// 依据（2026-10-18 探针 data/vision_probe.txt）：
// DeepSeek 页面上**正好有一个隐藏的 `input[type=file]`**，`multiple=true`，
// `accept` 白名单里含 .png/.jpg/.jpeg/.webp/.gif/.bmp/.svg/.tif/.avif，
// 以及 .pdf/.doc/.docx/.xls/.xlsx/.ppt/.pptx。
//
// 纯 JS 无法给 file input 赋值（安全限制），但 **CDP 可以**：
//   DOM.getDocument → DOM.querySelector('input[type=file]') → DOM.setFileInputFiles
// 赋值后由页面**自己的** change 处理器上传 —— 我们不实现上传逻辑，只负责喂文件。
//
// 注意：必须作用在 `deepseek` webview（模型页），不是 `web`（浏览页）。

/// `see` 允许的扩展名（与页面 accept 白名单的子集）。
const SEE_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "svg", "tif", "tiff", "avif", "ico",
    "pdf",
];

/// 把本地文件附加到 DeepSeek 输入区。
pub(crate) fn attach_file(app: &tauri::AppHandle, path: &str) -> Result<String, String> {
    let p = std::path::Path::new(path.trim());
    if !p.is_file() {
        return Err(format!("文件不存在：{}", p.display()));
    }
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if !SEE_EXT.contains(&ext.as_str()) {
        return Err(format!(
            "不支持的格式 .{ext}；允许：{}",
            SEE_EXT.join(" / ")
        ));
    }
    let abs = std::fs::canonicalize(p)
        .map_err(|e| format!("解析绝对路径失败：{e}"))?;
    // Windows 的 canonicalize 会带 \\?\ 前缀，CDP 需要普通路径
    let abs = abs
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string();

    let wv = app
        .get_webview("deepseek")
        .ok_or("找不到 deepseek webview（模型页未就绪）")?;

    cdp_call(&wv, "DOM.enable", "{}")?;
    let doc = cdp_call(&wv, "DOM.getDocument", "{\"depth\":1}")?;
    let root = json_path(&doc, &["result", "root", "nodeId"])
        .and_then(|v| v.as_u64())
        .ok_or("无法取得文档根节点")?;
    let q = cdp_call(
        &wv,
        "DOM.querySelector",
        &serde_json::json!({ "nodeId": root, "selector": "input[type=file]" }).to_string(),
    )?;
    let node = json_path(&q, &["result", "nodeId"])
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if node == 0 {
        return Err("页面上找不到文件选择器（DeepSeek 页面结构可能已变化）".to_string());
    }
    cdp_call(
        &wv,
        "DOM.setFileInputFiles",
        &serde_json::json!({ "nodeId": node, "files": [abs] }).to_string(),
    )?;
    // 给页面自己的上传/解析留时间（图片要传上去才能被模型看到）
    std::thread::sleep(std::time::Duration::from_millis(2500));
    chain_log(&format!("see: 已附加 {}", abs));
    Ok(format!(
        "已把 `{}` 附加到对话输入区（页面自己的上传流程会把它送上去）。\
         下一轮提问即可让模型看到这张图。",
        p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or(abs)
    ))
}
