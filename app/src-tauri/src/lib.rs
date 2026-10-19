// Mica 阶段2 · M1a 主链路最小版
// - 传输：2-WebView（shell + DeepSeek）+ 矩形同步（set_browser_rect）
// - 引擎：进程内 CDP 直调；网络层拦截（SSE）读回 + 多信号完成判定
// - Agent 三件套：①提示词注入 ②工具标记解析 ③PowerShell 执行（默认拒绝的审批）+ 结果回传循环
// - 审计：每次工具执行写 data\audit.log（proto-audit，M2 正式化）
// - 安全：工具执行无引擎侧旁路——一律经 UI 审批（默认拒绝 / 超时拒绝，fail-closed）
use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl};

/// 配置与路径：**运行时优先、内嵌兜底**（可扩展性报告 v2 · S2/S3）。
/// 原先 selectors/prompt 由 `include_str!` 编译期固化，改配置必须重编译；
/// 现改为运行时读取：非安全字段改完下一回合即生效；安全配置冻结为快照，需 UI 确认重载。
mod config;
use config::{audit_log_path, chain_log_path, data_dir, tmp_dir};
// 链路自检 flag 的路径只在 debug 构建里用；release 分支不编译它（否则 dead_code 警告）
#[cfg(debug_assertions)]
use config::chain_flag_path;

// ---------- 模块拆分（可扩展性分析报告 v2 · S1） ----------
mod browser;
mod builtin;
mod cdp;
mod chat;
mod commands;
mod exec;
mod log;
mod mcp_bridge;
mod memory;
mod net;
mod plugins;
mod security;
mod selectors;
mod session;
mod skills;
mod state;
mod store;
mod tools;
mod win_job;
mod workflows;
mod agent;

// 统一引入，保持原单文件的扁平命名空间（入口与测试无需改写）。
use agent::*;
use cdp::*;
use commands::*;
use exec::*;
use log::*;
use net::*;
use selectors::*;
use state::*;

// ---------- 启动自检 + 演示（debug 探针） ----------

/// 崩溃落盘：release 是 windows_subsystem，崩溃时没有控制台可看。
/// 优先写数据目录 `crash.log`，失败再写 `%TEMP%\Mica-crash.log`，最后才 stderr。
fn append_crash_log(path: &Path, msg: &str) -> bool {
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => f.write_all(msg.as_bytes()).is_ok(),
        Err(_) => false,
    }
}

fn install_panic_hook() {
    let dir = config::data_dir();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let msg = format!("[{}] PANIC\n{info}\n{bt}\n\n", now_ms());
        let wrote = append_crash_log(&Path::new(&dir).join("crash.log"), &msg)
            || append_crash_log(&std::env::temp_dir().join("Mica-crash.log"), &msg);
        if !wrote {
            eprintln!("{msg}");
        }
    }));
}

/// 自动更新探针（debug 专用，`EAGENT_UPDATE_PROBE=1` 启用）。
///
/// 用途：在没有 UI 自动化的环境里跑通 updater 的 check → download → install。
/// 生产构建不编译此函数。
#[cfg(debug_assertions)]
fn run_update_probe(app: &tauri::AppHandle) {
    use tauri_plugin_updater::UpdaterExt;
    let handle = app.clone();
    chain_log("UPDATE PROBE: start");
    std::thread::spawn(move || {
        tauri::async_runtime::block_on(async move {
            let updater = match handle.updater() {
                Ok(u) => u,
                Err(e) => {
                    chain_log(&format!("UPDATE PROBE FAIL(updater): {e}"));
                    return;
                }
            };
            match updater.check().await {
                Ok(Some(update)) => {
                    chain_log(&format!(
                        "UPDATE PROBE: found {} (current {})",
                        update.version, update.current_version
                    ));
                    let mut got = 0usize;
                    let res = update
                        .download_and_install(
                            |chunk, total| {
                                got += chunk;
                                if got % (512 * 1024) < chunk {
                                    chain_log(&format!(
                                        "UPDATE PROBE: downloaded {got} / {total:?}"
                                    ));
                                }
                            },
                            || chain_log("UPDATE PROBE: download finished, launching installer"),
                        )
                        .await;
                    match res {
                        Ok(_) => chain_log("UPDATE PROBE: OK（安装器已启动，应用即将退出）"),
                        Err(e) => chain_log(&format!("UPDATE PROBE FAIL(install): {e}")),
                    }
                }
                Ok(None) => chain_log("UPDATE PROBE: no update"),
                Err(e) => chain_log(&format!("UPDATE PROBE FAIL(check): {e}")),
            }
        });
    });
}

fn auto_boot_tasks(app: tauri::AppHandle) {
    // 【S2/S3】启动即报配置来源与生效参数 —— 便于确认「改配置有没有生效」。
    let c = config::agent_cfg();
    chain_log(&format!("boot: config 来源 → {}", config::config_source_report()));
    {
        let caps = chat::current_caps();
        let pcfg = config::provider_cfg();
        chain_log(&format!(
            "boot: 对话通道 → {}（model={}；streaming={}；工具清单上限 {} 字符；Ollama {}）",
            pcfg.active,
            pcfg.ollama_model,
            caps.streaming,
            caps.max_tool_list_chars,
            pcfg.ollama_base_url
        ));
        // 记忆层启动自检：确保 schema 能在真实数据目录创建/迁移。
        match memory::db(&app) {
            Ok(d) => match d.kb_status() {
                Ok((docs, chunks)) => chain_log(&format!(
                    "boot: 记忆层 → SQLite 就绪（知识库 {docs} 篇 / {chunks} 块）"
                )),
                Err(e) => chain_log(&format!("boot: 记忆层 → 数据库就绪但状态读取失败：{e}")),
            },
            Err(e) => chain_log(&format!("boot: 记忆层 → 数据库不可用（对话不受影响）：{e}")),
        }
        // 通道可用性（Ollama 未启动时最多等 3s；在后台线程里，不影响 UI）。
        let pl = chat::list_providers();
        let desc = pl
            .providers
            .iter()
            .map(|p| {
                format!(
                    "{}{}",
                    p.label,
                    if p.available { "✓" } else { "✗" }
                )
            })
            .collect::<Vec<_>>()
            .join(" ｜ ");
        chain_log(&format!("boot: 通道列表 → active={} ｜ {desc}", pl.active));
        // 路径探测报告：发布环境里"到底写到了哪"必须可见。
        chain_log(&format!("boot: 数据目录探测 → {}", config::path_probe_report()));
        // 崩溃落盘提示：最近 24h 有 crash.log → UI 事件 + 日志尾部。
        let crash = Path::new(&config::data_dir()).join("crash.log");
        if let Ok(md) = std::fs::metadata(&crash) {
            let recent = md
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs() < 86_400)
                .unwrap_or(false);
            if recent {
                let text = std::fs::read_to_string(&crash).unwrap_or_default();
                let tail: String = text.chars().rev().take(800).collect::<Vec<_>>().into_iter().rev().collect();
                chain_log(&format!(
                    "boot: 检测到最近 24h 崩溃日志（{} 字节）→ {}",
                    md.len(),
                    crash.display()
                ));
                chain_log(&format!("boot: 崩溃日志尾部 → {tail}"));
                emit_ev(
                    &app,
                    "engine://state",
                    "crash",
                    &format!("上次运行发生崩溃；日志：{}", crash.display()),
                );
            }
        }
    }
    chain_log(&format!(
        "boot: agentCfg → timeout={}s cap={} iter={} endpoint={}",
        c.tool_timeout_sec, c.tool_output_cap, c.max_tool_iter, c.reply_endpoint
    ));
    {
        let servers = config::mcp_servers();
        if servers.is_empty() {
            chain_log("boot: MCP → 未配置任何 server（config/mcp.json 缺失或 servers 为空），功能不生效");
        } else {
            let desc = servers
                .iter()
                .map(|s| format!("{}:{} {}", s.name, s.command, s.args.join(" ")))
                .collect::<Vec<_>>()
                .join(" ｜ ");
            chain_log(&format!("boot: MCP → {} 个 server 已启用 → {desc}", servers.len()));
        }
    }    // ★ 可写性探测（阶段 C 前置 #1/#2）：缓存目录不能想当然用 %LOCALAPPDATA%。
    // 实测本会话里从 D: 启动的子进程写 %LOCALAPPDATA%/%TEMP% 会得到"拒绝访问 (os error 5)"。
    // 这里在**应用自身进程**里探一次，用户可据此判断其真实环境有没有同样限制。
    {
        let probe = |dir: &str, label: &str| {
            let p = Path::new(dir).join(".eagent_write_probe");
            let ok = std::fs::write(&p, b"x").is_ok();
            if ok {
                let _ = std::fs::remove_file(&p);
            }
            format!("{label}={}", if ok { "✓" } else { "✗" })
        };
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let temp = std::env::var("TEMP").unwrap_or_default();
        let parts = vec![
            probe(&local, "LOCALAPPDATA"),
            probe(&temp, "TEMP"),
            probe(&data_dir(), "data_dir"),
        ];
        chain_log(&format!("boot: 可写性探测 → {}", parts.join(" ｜ ")));
    }

    // ★ 看图能力探测（仅 debug）：DeepSeek 网页里的文件上传控件长什么样？
    //
    // 已从 JS bundle 证实网页端有 `["default","expert","vision"]` 模式枚举、
    // findVisionModel / checkFilesCountOrSize / upload_file / /api/v0/file ——
    // 说明有视觉与上传链路。但**文件选择器是否存在于 DOM**、账号是否真有 vision
    // 权限，这两点 bundle 看不出来，必须在真实页面里查。
    // 探针把结果写盘，避免只能靠猜（本会话的教训）。
    #[cfg(debug_assertions)]
    {
        let h = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(12)); // 等 SPA 挂载
            let Some(wv) = h.get_webview("deepseek") else {
                chain_log("vision_probe: 找不到 deepseek webview");
                return;
            };
            let js = r#"(function(){
  var out={fileInputs:[],buttons:[],hits:[]};
  var ins=document.querySelectorAll('input[type=file]');
  for(var i=0;i<ins.length;i++){
    out.fileInputs.push({accept:ins[i].accept||'',multiple:!!ins[i].multiple,hidden:ins[i].offsetParent===null});
  }
  var btns=document.querySelectorAll('button,[role=button],div[class*=upload],div[class*=attach],div[class*=image]');
  for(var j=0;j<btns.length&&out.buttons.length<25;j++){
    var t=(btns[j].getAttribute('aria-label')||btns[j].getAttribute('title')||btns[j].textContent||'').trim().slice(0,40);
    if(t) out.buttons.push(t);
  }
  var txt=document.body?document.body.innerText:'';
  ['vision','图片','上传','附件','image'].forEach(function(k){
    if(txt.toLowerCase().indexOf(k.toLowerCase())>=0) out.hits.push(k);
  });
  out.ready=document.readyState;
  return JSON.stringify(out);
})()"#;
            match cdp_eval(&wv, js) {
                Ok(r) => {
                    let path = format!(r"{}\vision_probe.txt", data_dir());
                    let _ = std::fs::write(&path, &r);
                    chain_log(&format!("vision_probe: 结果已写入 {path}"));
                }
                Err(e) => chain_log(&format!("vision_probe: 执行失败 {e}")),
            }
        });
    }

    // 【M-01】启动即报工具注册表 —— 没插件工具时附加说明为空（协议不变）。
    {
        let extra = tools::plugin_tools();
        let extra_desc = if extra.is_empty() {
            "无（协议与升级前逐字一致）".to_string()
        } else {
            extra.iter().map(|t| format!("{}←{}", t.name, t.source)).collect::<Vec<_>>().join(", ")
        };
        chain_log(&format!(
            "boot: 工具注册表 → 共 {} 个（powershell + 内置 {} + 插件 {}）；插件说明 {} 字符：[{}]",
            tools::count(),
            builtin::all().len(),
            extra.len(),
            tools::extra_prompt_block().chars().count(),
            extra_desc
        ));
        // 【策划案 2.0 · P1.3】扩展工具（内置 + 通用 JSON 协议）的生效状态
        let builtins: Vec<&str> = builtin::all().iter().map(|b| b.name).collect();
        let json_block = if c.extended_tools { tools::tool_list_prompt_block() } else { String::new() };
        chain_log(&format!(
            "boot: 扩展工具 → extendedTools={}，内置 [{}]；清单 {} 字符；通用协议标记{}",
            c.extended_tools,
            builtins.join(", "),
            json_block.chars().count(),
            if c.extended_tools { "已启用" } else { "未注入" }
        ));
    }
    #[cfg(debug_assertions)]
    if std::env::var("EAGENT_UPDATE_PROBE").ok().as_deref() == Some("1") {
        run_update_probe(&app);
    }
    #[cfg(debug_assertions)]
    if std::env::var("EAGENT_MCP_PROBE").ok().as_deref() == Some("1") {
        let block = mcp_tool_prompt_block(&app);
        chain_log(&format!(
            "MCP PROBE: 工具清单 {} 字符 → {}",
            block.chars().count(),
            if block.trim().is_empty() { "FAIL/EMPTY" } else { "OK" }
        ));
    }
    std::thread::sleep(Duration::from_secs(15));
    chain_log("auto: 开始检查");
    let Some(wv) = app.get_webview("deepseek") else {
        chain_log("auto: deepseek webview 不存在，放弃");
        return;
    };
    let href = cdp_eval(&wv, "window.location.href").unwrap_or_default();
    chain_log(&format!("auto: href={href}"));
    if href.is_empty() || href.contains("sign_in") {
        chain_log("auto: 未登录（或被重定向到 sign_in），跳过自动任务");
        return;
    }

    // 启动时选择器健康探测。
    // 【重要】M1b 起回复读取已改走网络层（SSE），选择器不再是关键路径 ——
    // 因此网络层就绪时，选择器 0 命中只记日志、不再向 UI 报错（避免误导性告警）。
    let net_ready = app.state::<EngineState>().net.enabled.load(Ordering::SeqCst);
    match run_selfcheck(&app) {
        Ok(raw) => {
            chain_log(&format!("auto: selector probe = {}", raw.chars().take(300).collect::<String>()));
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if v.get("allHealthy").and_then(|x| x.as_bool()) == Some(false) {
                    let unhealthy = v
                        .get("unhealthy")
                        .and_then(|x| x.as_array())
                        .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                        .unwrap_or_else(|| "unknown".to_string());
                    if net_ready {
                        // 兜底路径：仅记录，不打扰用户
                        emit_ev(
                            &app,
                            "engine://state",
                            "injecting",
                            &format!("选择器兜底组 0 命中（{}）—— 主链路走网络层，不影响使用", unhealthy),
                        );
                        chain_log(&format!(
                            "SELECTOR UNHEALTHY on boot: {unhealthy}（网络层已就绪，降级为兜底）"
                        ));
                    } else {
                        emit_ev(
                            &app,
                            "engine://state",
                            "error",
                            &format!("选择器健康告警：以下组 0 命中 —— {}（网络层未就绪且选择器失效，请更新 selectors.json）", unhealthy),
                        );
                        chain_log(&format!("SELECTOR UNHEALTHY on boot: {unhealthy}（网络层未就绪！）"));
                    }
                }
            }
        }
        Err(e) => chain_log(&format!("auto: selector probe failed: {e}")),
    }

    // 网络层健康状态上报（主链路信源）
    emit_ev(
        &app,
        "engine://state",
        if net_ready { "idle" } else { "error" },
        if net_ready {
            "网络层就绪：回复读取走 SSE（/api/v0/chat/completion），不依赖页面类名"
        } else {
            "网络层未就绪：回复读取将退化为 DOM 兜底，稳定性下降"
        },
    );

    // DOM 取证探针：DS 已全面改用哈希类名（语义子串全灭）→ 用结构取证：
    // 找最深滚动容器（消息列表），dump 最后 3 个消息块的 DOM 骨架（tag.类名(文本长)，3 层）。
    // 加载时序不确定 → 独立线程重试（12s × 8），成功即停。
    {
        let probe_app = app.clone();
        std::thread::spawn(move || {
            let dom_js = r#"JSON.stringify((function(){var wrap=document.querySelector('[class*="_8a693f3"]');if(!wrap){var sc=document.querySelectorAll('[class*="ds-scroll-area"]');var best=null,bl=0;for(var i=0;i<sc.length;i++){var t=(sc[i].textContent||'').trim().length;if(t>bl){bl=t;best=sc[i];}}if(!best)return {err:'no scroller'};var cur=best;for(var d=0;d<4;d++){var m=null,ml=0;for(var i=0;i<cur.children.length;i++){var t=(cur.children[i].textContent||'').trim().length;if(t>ml){ml=t;m=cur.children[i];}}if(m&&ml>=(cur.textContent||'').trim().length*0.9&&ml>5){cur=m;}else break;}wrap=cur;}function sk(el,d){var t=(el.textContent||'').trim();var cn=(typeof el.className==='string')?el.className.split(/\s+/).slice(0,4).join('.'):'';var s=el.tagName.toLowerCase()+(cn?'.'+cn:'')+'('+t.length+')';if(d<3&&el.children.length){var k=[];for(var i=0;i<Math.min(el.children.length,6);i++){k.push(sk(el.children[i],d+1));}s+=' >> '+k.join(' ; ');}return s;}var kids=wrap.children;var out=[];var start=Math.max(0,kids.length-2);for(var i=start;i<kids.length;i++){out.push(sk(kids[i],0));}return {n:kids.length,wrapCls:(typeof wrap.className==='string')?wrap.className:'',last2:out};})())"#;
            // 首发等 45s（链路测试会把消息发出去，DOM 才有消息块可挖），此后 12s 一次重试
            std::thread::sleep(Duration::from_secs(45));
            for attempt in 0..8 {
                let Some(wv) = probe_app.get_webview("deepseek") else { return };
                match cdp_eval(&wv, dom_js) {
                    Ok(inv) => {
                        let _ = std::fs::write(format!(r"{}\dom_probe.json", data_dir()), &inv);
                        chain_log(&format!("DOM probe: saved len={} (attempt {attempt})", inv.len()));
                        if inv.contains("\"last2\"") {
                            return;
                        }
                    }
                    Err(e) => chain_log(&format!("DOM probe try{attempt} err: {e}")),
                }
                std::thread::sleep(Duration::from_secs(12));
            }
            chain_log("DOM probe: failed after retries");
        });
    }

    chain_log(&format!(
        "net: 启动以来 ws帧={} ws发送={} 响应={} 完成={} 数据块={}",
        net_count(&app, "Network.webSocketFrameReceived"),
        net_count(&app, "Network.webSocketFrameSent"),
        net_count(&app, "Network.responseReceived"),
        net_count(&app, "Network.loadingFinished"),
        net_count(&app, "Network.dataReceived"),
    ));

    // 【E-17】基础链路测试仅在 debug 构建自动执行；release 不自动发消息消耗配额。
    // 用户可在 UI 手动触发自检。
    #[cfg(debug_assertions)]
    {
        if Path::new(&chain_flag_path()).exists() {
            chain_log("auto: 已有通过记录（flag 存在），跳过");
        } else {
            emit_ev(&app, "engine://state", "chain-test", "自动链路测试（注入→发送→完成判定→读回）");
            match chat::send_turn(&app, "请只回复两个字母：OK") {
                Ok(turn) => {
                    let reply = turn.text;
                    chain_log(&format!("CHAIN OK: reply={:?}", reply.chars().take(40).collect::<String>()));
                    let _ = std::fs::write(&chain_flag_path(), "ok");
                }
                Err(e) => chain_log(&format!("CHAIN FAIL: {e}")),
            }
        }

        // 【策划案 2.0 · P1.7】工具调用闭环探针：验证**模型是否照 JSON 协议发出工具调用**
        // —— 这是 Phase 1 最大的未知项，跑一次即可、带 flag 不重复打扰。
        let probe_flag = format!(r"{}\.tool_probe_done", data_dir());
        if Path::new(&probe_flag).exists() {
            chain_log("auto: 工具调用探针已有记录，跳过");
        } else {
            run_tool_probe(&app, &probe_flag);
        }
    }
}

/// 工具调用闭环探针（debug 专用，跑一次落 flag）。
///
/// 依次验证三件事：
/// 1. 模型会不会**照 JSON 标记格式**发出工具调用；
/// 2. 引擎能否解析并**执行**内置工具；
/// 3. 把执行结果回传后，模型能否**据此作答**（完整闭环）。
#[cfg(debug_assertions)]
fn run_tool_probe(app: &tauri::AppHandle, flag_path: &str) {
    emit_ev(app, "engine://state", "tool-probe", "验证模型是否照 JSON 工具格式调用");
    let ask = format!(
        "这是一次格式自检。请严格按下面格式调用 calc 工具计算 1+2*3，\
         只输出这两行注释和中间的 JSON，不要有任何其它内容：\n{}\n\
         {{\"name\":\"calc\",\"arguments\":{{\"expr\":\"1+2*3\"}}}}\n{}",
        tools::JSON_HEAD,
        tools::JSON_TAIL
    );
    let reply = match chat::send_turn(app, &ask) {
        Ok(t) => t.text,
        Err(e) => {
            chain_log(&format!("TOOL PROBE FAIL(send): {e}"));
            return;
        }
    };
    chain_log(&format!(
        "TOOL PROBE: 回复 {} 字符 → {:?}",
        reply.chars().count(),
        reply.chars().take(240).collect::<String>()
    ));

    match tools::parse_json_call(&reply) {
        None => chain_log("TOOL PROBE FAIL: 未识别到 JSON 工具调用 —— 模型没有照格式走"),
        Some(Err(e)) => chain_log(&format!("TOOL PROBE PARTIAL: 找到标记但内容非法 → {e}")),
        Some(Ok(call)) => {
            chain_log(&format!("TOOL PROBE: 识别到工具「{}」参数 {}", call.name, call.arguments));
            // 探针不得成为审批旁路：只允许无副作用的 calc/now。
            // web_read 有出站网络副作用，必须走正常审批；插件/MCP 更不允许。
            if call.name != "calc" && call.name != "now" {
                chain_log(&format!(
                    "TOOL PROBE: 拒绝工具「{}」——探针只允许 calc/now，禁止绕过审批",
                    call.name
                ));
                return;
            }
            match builtin::execute(&call.name, &call.arguments) {
                Err(e) => chain_log(&format!("TOOL PROBE FAIL(exec): {e}")),
                Ok(out) => {
                    chain_log(&format!("TOOL PROBE: 内置工具执行成功 → {out}"));
                    let follow = format!("[工具回传][{}]\n{out}\n\n请直接告诉我计算结果是几。", call.name);
                    match chat::send_turn(app, &follow).map(|t| t.text) {
                        Ok(r2) => {
                            chain_log(&format!(
                                "TOOL PROBE E2E OK: 回传后模型回答 → {:?}",
                                r2.chars().take(120).collect::<String>()
                            ));
                            let _ = std::fs::write(flag_path, "ok");
                        }
                        Err(e) => chain_log(&format!("TOOL PROBE PARTIAL: 第二轮失败 {e}")),
                    }
                }
            }
        }
    }
}

// ---------- 入口 ----------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_hook();
    // 崩溃落盘验证：debug 构建 + 环境变量显式启用（生产不会触发）。
    #[cfg(debug_assertions)]
    if std::env::var("EAGENT_PANIC_TEST").ok().as_deref() == Some("1") {
        panic!("EAGENT_PANIC_TEST: intentional panic for crash-log verification");
    }
    #[cfg(windows)]
    unsafe {
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", config::webview_dir());
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 第二个实例启动 → 聚焦已有窗口后退出（插件负责退出）。
            if let Some(w) = app.get_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(EngineState::default())
        .invoke_handler(tauri::generate_handler![
            set_browser_rect,
            test_roundtrip,
            start_agent_session,
            agent_turn,
            tool_decision,
            selfcheck,
            net_probe,
            list_skills,
            add_skill,
            remove_skill,
            restore_builtin_skills,
            save_session,
            load_session,
            clear_session,
            new_chat,
            session_path_info,
            app_paths,
            list_providers,
            set_provider,
            list_sessions,
            open_session,
            delete_session,
            memory_list,
            memory_set,
            memory_delete,
            kb_import,
            kb_list,
            kb_delete,
            kb_search,
            kb_status,
            list_workflows,
            run_workflow,
            cancel_workflow,
            list_workflow_runs,
            approval_rules,
            auto_approve_rules,
            security_config_status,
            reload_security_config,
            log_audit,
            read_audit_log,
            audit_log_path_info,
            list_plugins,
            plugin_dir_info,
            plugin_details,
            create_plugin,
            delete_plugin,
            open_plugins_dir,
            read_attachments,
            mcp_status,
            mcp_save_server,
            mcp_remove_server,
            mcp_import,
            mcp_catalog,
            mcp_set_catalog,
            plugin_catalog,
            plugin_set_catalog
        ])
        .setup(|app| {
            // 初始尺寸：在 1280×840 与「主显示器逻辑工作区」之间取较小值，
            // 避免高 DPI 缩放后窗口超出屏幕（右侧/底部被裁掉）。
            let (mut w, mut h) = (1280.0_f64, 840.0_f64);
            if let Ok(Some(m)) = app.primary_monitor() {
                let sf = m.scale_factor();
                let logical = m.size().to_logical::<f64>(sf);
                w = w.min((logical.width * 0.90).max(900.0));
                h = h.min((logical.height * 0.85).max(600.0));
                chain_log(&format!(
                    "window: monitor {}x{} physical @ {sf}x（逻辑 {:.0}x{:.0}）→ initial {:.0}x{:.0}",
                    m.size().width,
                    m.size().height,
                    logical.width,
                    logical.height,
                    w,
                    h
                ));
            }
            let window = tauri::window::WindowBuilder::new(app, "main")
                .title("Mica")
                .inner_size(w, h)
                .min_inner_size(900.0, 600.0)
                .maximizable(false)
                .build()?;
            let _ = window.center();
            // 注：本机存在 DPI 感知坐标系混淆（tauri 报的"物理"尺寸带 1.5× 系数），
            // 任何显式 set_size / Resized 手动同步都会在错误坐标系里放大 → 曾把 shell webview 撑出窗口裁切。
            // 结论：窗口尺寸交给系统创建 + auto_resize，不做任何手动干预（2026-10-17 实测回滚）。

            let _shell = window.add_child(
                tauri::webview::WebviewBuilder::new("shell", WebviewUrl::App("index.html".into()))
                    .auto_resize(),
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(w, h),
            )?;

            let _ds = window.add_child(
                tauri::webview::WebviewBuilder::new(
                    "deepseek",
                    WebviewUrl::External("https://chat.deepseek.com".parse().unwrap()),
                )
                .on_page_load(|wv, payload| {
                    println!("PAGE_LOAD webview={} event={:?} url={}", wv.label(), payload.event(), payload.url());
                }),
                LogicalPosition::new(800.0, 48.0),
                LogicalSize::new(480.0, h - 48.0),
            )?;

            let handle = app.handle().clone();
            // 网络层捕获：页面就绪时序不确定 → 独立线程重试订阅（本方案的核心信源，
            // 越早挂上越好；失败不阻塞后续流程）。
            {
                let net_handle = app.handle().clone();
                std::thread::spawn(move || {
                    for attempt in 1..=20 {
                        std::thread::sleep(Duration::from_secs(3));
                        match net_subscribe(&net_handle) {
                            Ok(()) => {
                                chain_log(&format!("net: 网络层捕获已就绪（第 {attempt} 次尝试）"));
                                return;
                            }
                            Err(e) => {
                                if attempt == 1 || attempt % 5 == 0 {
                                    chain_log(&format!("net: 订阅未就绪（第 {attempt} 次）：{e}"));
                                }
                            }
                        }
                    }
                    chain_log("net: 20 次尝试后仍未订阅成功 —— 回退纯 DOM 通道");
                });
            }
            std::thread::spawn(move || auto_boot_tasks(handle));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ---------- 单元测试（不涉及 UI / 网络 / 模型；执行器的测试命令为本文件静态常量） ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_log_appends_to_given_path() {
        let p = std::env::temp_dir().join(format!("eagent-crash-test-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&p);
        assert!(append_crash_log(&p, "boom\n"));
        assert!(append_crash_log(&p, "again\n"));
        let text = std::fs::read_to_string(&p).expect("crash log should exist");
        assert!(text.contains("boom") && text.contains("again"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn exec_powershell_ascii_roundtrip() {
        let out = execute_powershell("", r#"Write-Output "EAGENT_TEST_OK""#).expect("exec ok");
        assert!(out.contains("EAGENT_TEST_OK"), "got: {out}");
    }

    #[test]
    fn exec_powershell_utf8_chinese_roundtrip() {
        let out = execute_powershell("", r#"Write-Output "中文测试输出""#).expect("exec ok");
        assert!(out.contains("中文测试输出"), "got: {out}");
    }

    #[test]
    fn exec_powershell_error_is_captured() {
        let out = execute_powershell("", "Write-Error 'boom'; exit 2").expect("exec ok");
        assert!(out.contains("boom"), "got: {out}");
    }

    // ---------- 网络层 SSE 协议解析（样本取自 2026-10-17 真实抓包） ----------

    /// 真实 SSE 帧序列（节选）：THINK 片段（思考）与 RESPONSE 片段（回答）必须区分。
    const SSE_SAMPLE: &str = concat!(
        "event: ready\n",
        "data: {\"request_message_id\":7,\"response_message_id\":8,\"model_type\":\"default\"}\n\n",
        "event: update_session\n",
        "data: {\"updated_at\":1792174440.6803598}\n\n",
        "data: {\"v\":{\"response\":{\"message_id\":8,\"role\":\"ASSISTANT\",\"status\":\"WIP\",",
        "\"fragments\":[{\"id\":2,\"type\":\"THINK\",\"content\":\"The\"}]}}}\n\n",
        "data: {\"p\":\"response/fragments/-1/content\",\"o\":\"APPEND\",\"v\":\" user\"}\n\n",
        "data: {\"v\":\" only\"}\n\n",
        "data: {\"p\":\"response/fragments/-1/elapsed_secs\",\"o\":\"SET\",\"v\":2.534041017}\n\n",
        "data: {\"p\":\"response/fragments\",\"o\":\"APPEND\",\"v\":[{\"id\":3,\"type\":\"RESPONSE\",\"content\":\"工作\"}]}\n\n",
        "data: {\"p\":\"response/fragments/-1/content\",\"v\":\"目录\"}\n\n",
        "data: {\"p\":\"response\",\"o\":\"BATCH\",\"v\":[{\"p\":\"accumulated_token_usage\",\"v\":7093},",
        "{\"p\":\"quasi_status\",\"v\":\"FINISHED\"}]}\n\n",
        "data: {\"p\":\"response/status\",\"o\":\"SET\",\"v\":\"FINISHED\"}\n\n",
    );

    #[test]
    fn sse_extracts_response_fragment_not_think() {
        let text = parse_sse_assistant_text(SSE_SAMPLE);
        // 只取 RESPONSE 片段：THINK 的 "The user only" 必须被排除
        assert_eq!(text, "工作目录");
        assert!(!text.contains("user"), "THINK 内容泄漏进回答: {text}");
    }

    #[test]
    fn sse_detects_finished_status() {
        assert!(sse_declares_finished(SSE_SAMPLE));
        assert!(!sse_declares_finished("data: {\"v\":\"only\"}\n\n"));
    }

    #[test]
    fn sse_handles_openai_style_delta() {
        // 兼容 OpenAI 风格（若上游换成该格式）
        let s = "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\n";
        assert_eq!(parse_sse_assistant_text(s), "OK");
    }

    #[test]
    fn sse_empty_on_garbage() {
        assert_eq!(parse_sse_assistant_text("not sse at all"), "");
        assert_eq!(parse_sse_assistant_text(""), "");
    }

    #[test]
    fn echo_guard_rejects_user_message_readback() {
        // 注入 8k 提示词时，空间启发式读回可能正好抓到这条用户消息 → 必须判为回声
        let prompt = "你是自动化 Agent，通过工具实现与用户电脑交互。".repeat(200);
        assert!(looks_like_echo(&prompt, &prompt));
        assert!(looks_like_echo(&format!("前言{prompt}"), &prompt));
        // 真正的助手回复不应被判为回声
        assert!(!looks_like_echo("好的，我来查看目录。", &prompt));
        // 助手回复比用户消息长很多 → 不是回声
        let long_reply = format!("{prompt}{}", "补充内容".repeat(500));
        assert!(!looks_like_echo(&long_reply, &prompt));
    }

    #[test]
    fn echo_guard_ignores_empty() {
        assert!(!looks_like_echo("", "abc"));
        assert!(!looks_like_echo("abc", ""));
    }
}
