//! 会话：注入 → 发送 → 多信号完成判定 → 读回。
//!
//! 拆分自原单文件 `lib.rs`（可扩展性分析报告 v2 · S1）。
//! `use super::*` 继承根模块的导入与共享项，避免重复声明。

use super::*;

// ---------- 发送 + 完成判定 v2 + 全文读回 ----------

pub(crate) fn read_js_full(as_json: &str, sb_json: &str) -> String {
    format!(
        "JSON.stringify((function(){{\
          var diag=[];\
          var ta=document.querySelector('textarea');\
          if(ta){{\
            var taRect=ta.getBoundingClientRect();\
            var all=document.querySelectorAll('*');var best='',bestY=-1,bestN=0;\
            for(var i=0;i<all.length;i++){{\
              var el=all[i];if(el.offsetParent===null)continue;\
              var t=(el.textContent||'').trim();if(t.length<80)continue;\
              var r=el.getBoundingClientRect();\
              if(r.bottom>taRect.top-10)continue;\
              if(r.width<200)continue;\
              if(r.bottom>bestY){{bestY=r.bottom;best=t;bestN++;}}\
            }}\
            diag.push('spatial:taY='+Math.round(taRect.top)+' candidates='+bestN);\
            if(best.length>0)return {{n:bestN,t:best,src:'spatial',diag:diag}};\
          }}\
          var SB={sb_json};var sb=null;\
          for(var si=0;si<SB.length&&!sb;si++){{var found=document.querySelectorAll(SB[si]);for(var fj=0;fj<found.length;fj++){{if(found[fj].offsetParent!==null){{sb=found[fj];break;}}}}}}\
          var chatWrap=null;\
          if(sb){{var p=sb.parentElement;while(p&&!chatWrap){{var cn=(p.className&&typeof p.className==='string')?p.className:'';if(cn.indexOf('ds-scroll-area')>=0||cn.indexOf('_8a693f3')>=0||(p.scrollHeight>p.clientHeight&&p.clientHeight>100))chatWrap=p;p=p.parentElement;}}}}\
          diag.push('sendBtn:'+(sb?'found':'missing')+' chatWrap:'+(chatWrap?(chatWrap.className||'').toString().slice(0,40):'null'));\
          if(chatWrap){{\
            var msgs=chatWrap.querySelectorAll('[class*=\"_3098d02\"]');\
            diag.push('chatMsgs:'+msgs.length);\
            for(var i=msgs.length-1;i>=0;i--){{\
              var t=(msgs[i].textContent||'').trim();\
              if(t.length>0)return {{n:msgs.length,t:t,src:'chat-msg',idx:i,diag:diag}};\
            }}\
            function deepLast(el,d){{if(!el)return '';var t=(el.textContent||'').trim();if(t.length>50||!el.children||el.children.length===0||d>8)return t;for(var i=el.children.length-1;i>=0;i--){{var ct=deepLast(el.children[i],d+1);if(ct.length>30)return ct;}}return t;}}\
            for(var i=chatWrap.children.length-1;i>=0;i--){{\
              var t=deepLast(chatWrap.children[i],0);\
              if(t.length>30)return {{n:chatWrap.children.length,t:t,src:'chat-deep',idx:i,diag:diag}};\
            }}\
          }}\
          var sc=document.querySelectorAll('[class*=\"ds-scroll-area\"]');\
          var bestText='',bestLen=0;\
          for(var s=0;s<sc.length;s++){{\
            var wrap=sc[s];\
            if(wrap.children&&wrap.children.length>0){{\
              for(var i=wrap.children.length-1;i>=0;i--){{\
                var t=(wrap.children[i].textContent||'').trim();\
                if(t.length>bestLen){{bestLen=t.length;bestText=t;}}\
                if(t.length>0)break;\
              }}\
            }}\
          }}\
          if(bestText.length>0)return {{n:1,t:bestText,src:'scroll-best',diag:diag}};\
          var AS={as_json};var els=[];\
          for(var i=0;i<AS.length;i++){{var f=document.querySelectorAll(AS[i]);for(var j=0;j<f.length;j++){{if(els.indexOf(f[j])<0)els.push(f[j]);}}}}\
          for(var i=els.length-1;i>=0;i--){{\
            var t=(els[i].textContent||'').trim();\
            if(t.length>0)return {{n:els.length,t:t,src:'selector',idx:i,diag:diag}};\
          }}\
          return {{n:0,t:'',diag:diag}};\
        }})())"
    )
}

/// 注入→发送→完成判定 v2→全文读回。返回最后一条助手消息的**完整文本**。
pub(crate) fn send_and_wait_full(app: &tauri::AppHandle, message: &str) -> Result<String, String> {
    let sel = load_sel();
    let wv = app.get_webview("deepseek").ok_or("deepseek webview not found")?;

    // 【网络层】回合开始前清空网络事件缓冲，只保留本回合活动。
    net_clear(app);

    let msg_json = serde_json::to_string(message).map_err(|e| e.to_string())?;
    let inject_js = format!(
        "JSON.stringify((function(){{var ta=document.querySelector('textarea');if(!ta)return {{ok:false,err:'no textarea'}};ta.focus();var setter=Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype,'value').set;setter.call(ta,{msg_json});ta.dispatchEvent(new Event('input',{{bubbles:true}}));ta.dispatchEvent(new Event('change',{{bubbles:true}}));return {{ok:true,len:ta.value.length}};}})())"
    );
    emit_ev(app, "engine://state", "injecting", &format!("注入 {} 字符", message.chars().count()));
    let mut r = cdp_eval(&wv, &inject_js)?;
    if !r.contains("\"ok\":true") {
        // 页面可能正在导航/聚焦抖动：1 秒后重试一次
        std::thread::sleep(Duration::from_millis(1000));
        r = cdp_eval(&wv, &inject_js)?;
        if !r.contains("\"ok\":true") {
            emit_ev(app, "engine://state", "error", "注入失败");
            return Err(format!("inject failed: {r}"));
        }
    }

    std::thread::sleep(Duration::from_millis(300));

    let send_js = "JSON.stringify((function(){var ta=document.querySelector('textarea');if(!ta)return {ok:false};ta.focus();function ev(t){var e=new KeyboardEvent(t,{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true,cancelable:true});ta.dispatchEvent(e);}ev('keydown');ev('keypress');ev('keyup');return {ok:true};})())".to_string();
    let r = cdp_eval(&wv, &send_js)?;
    if !r.contains("\"ok\":true") {
        emit_ev(app, "engine://state", "error", "发送失败");
        return Err(format!("send failed: {r}"));
    }

    // 发送落地校验：DS 接受发送后会清空输入框；未清空 → 重按 Enter（最多 3 次）
    let val_js = "JSON.stringify({len:(function(){var ta=document.querySelector('textarea');return ta?ta.value.length:-1;})()})";
    for _ in 0..3 {
        std::thread::sleep(Duration::from_millis(700));
        let v = cdp_eval(&wv, val_js).unwrap_or_default();
        if v.contains("\"len\":0") || v.contains("\"len\":-1") {
            break;
        }
        let _ = cdp_eval(&wv, &send_js);
    }

    let sb_json = serde_json::to_string(&sel.send_btn).unwrap_or_else(|_| "[]".into());
    let as_json = serde_json::to_string(&sel.assistant).unwrap_or_else(|_| "[]".into());
    // 【E-06】合并轮询：一次 Runtime.evaluate 同时返回按钮状态 + 文本末尾，CDP 调用量减半
    let poll_js = format!(
        "JSON.stringify((function(){{var SB={sb_json};var out=[];for(var i=0;i<SB.length;i++){{var els=document.querySelectorAll(SB[i]);for(var j=0;j<els.length;j++){{var el=els[j];var rc=el.getBoundingClientRect();var p=el.querySelector('svg path');var d=p?(p.getAttribute('d')||''):'';out.push([el.offsetParent!==null?1:0,Math.round(rc.x),Math.round(rc.y),d.substring(0,30)]);}}}}var btn={{n:out.length,items:out}};function findText(){{var ta=document.querySelector('textarea');if(ta){{var taRect=ta.getBoundingClientRect();var all=document.querySelectorAll('*');var best='',bestY=-1;for(var i=0;i<all.length;i++){{var el=all[i];if(el.offsetParent===null)continue;var t=(el.textContent||'').trim();if(t.length<80)continue;var r=el.getBoundingClientRect();if(r.bottom>taRect.top-10)continue;if(r.width<200)continue;if(r.bottom>bestY){{bestY=r.bottom;best=t;}}}}if(best.length>0)return best;}}var SB={sb_json};var sb=null;for(var si=0;si<SB.length&&!sb;si++){{var found=document.querySelectorAll(SB[si]);for(var fj=0;fj<found.length;fj++){{if(found[fj].offsetParent!==null){{sb=found[fj];break;}}}}}}var chatWrap=null;if(sb){{var p=sb.parentElement;while(p&&!chatWrap){{var cn=(p.className&&typeof p.className==='string')?p.className:'';if(cn.indexOf('ds-scroll-area')>=0||cn.indexOf('_8a693f3')>=0||(p.scrollHeight>p.clientHeight&&p.clientHeight>100))chatWrap=p;p=p.parentElement;}}}}if(chatWrap){{var msgs=chatWrap.querySelectorAll('[class*=\"_3098d02\"]');for(var i=msgs.length-1;i>=0;i--){{var t=(msgs[i].textContent||'').trim();if(t.length>0)return t;}}function deepLast(el,d){{if(!el)return '';var t=(el.textContent||'').trim();if(t.length>50||!el.children||el.children.length===0||d>8)return t;for(var i=el.children.length-1;i>=0;i--){{var ct=deepLast(el.children[i],d+1);if(ct.length>30)return ct;}}return t;}}for(var i=chatWrap.children.length-1;i>=0;i--){{var t=deepLast(chatWrap.children[i],0);if(t.length>30)return t;}}}}var sc=document.querySelectorAll('[class*=\"ds-scroll-area\"]');var best='';for(var s=0;s<sc.length;s++){{var wrap=sc[s];if(wrap.children&&wrap.children.length>0){{for(var i=wrap.children.length-1;i>=0;i--){{var t=(wrap.children[i].textContent||'').trim();if(t.length>best.length)best=t;if(t.length>0)break;}}}}}}if(best.length>0)return best;var AS={as_json};var els2=[];for(var i=0;i<AS.length;i++){{var f=document.querySelectorAll(AS[i]);for(var j=0;j<f.length;j++){{if(els2.indexOf(f[j])<0)els2.push(f[j]);}}}}for(var i=els2.length-1;i>=0;i--){{var t=(els2[i].textContent||'').trim();if(t.length>0)return t;}}return '';}}var ft=findText();var tail={{n:ft.length>0?1:0,t:ft.substring(Math.max(0,ft.length-160))}};return {{btn:btn,tail:tail}};}})())"
    );
    let full_js = read_js_full(&as_json, &sb_json);

    let t0 = SystemTime::now();
    let timeout = Duration::from_secs(sel.timeout_sec);
    let (mut saw_gen, mut arrow_ticks, mut stable) = (false, 0u32, 0u32);
    let mut emitted_gen = false;
    let mut prev = String::new();
    // 选择器失效检测：saw_gen 后若文本持续为空，说明按钮状态正常但消息读不回来 → 选择器挂了
    let mut gen_started_at: Option<SystemTime> = None;
    let mut selector_warning_emitted = false;
    // 【网络层】本回合的回复流 requestId（SSE 端点 /api/v0/chat/completion）。
    // 这是本回合的权威标识：流结束 = 生成完毕，与 DOM 类名/SVG 图标完全解耦。
    let mut reply_rid: Option<String> = None;
    let mut emitted_net_gen = false;

    loop {
        if t0.elapsed().map(|d| d > timeout).unwrap_or(false) {
            let selector_hint = if saw_gen && prev.is_empty() {
                "（生成已开始但文本始终为空，疑似选择器失效，请运行自检）"
            } else {
                ""
            };
            let net_hint = match reply_rid.as_deref() {
                Some(rid) => format!(
                    "（网络层：已捕获回复流，收到 {} 字节，流未结束）",
                    net_reply_bytes(app, rid)
                ),
                None => "（网络层：未捕获到 /chat/completion 回复流）".to_string(),
            };
            emit_ev(app, "engine://state", "timeout", &format!("{}s 未完成{selector_hint}{net_hint}", sel.timeout_sec));
            chain_log(&format!("TIMEOUT after {}s: {selector_hint} {net_hint}", sel.timeout_sec));
            return Err(format!("timeout after {}s {selector_hint}{net_hint}", sel.timeout_sec).into());
        }
        std::thread::sleep(Duration::from_millis(sel.poll_ms));

        // 【网络层主信号】本回合的 SSE 回复流一旦出现，其 loadingFinished 即权威完成信号。
        if reply_rid.is_none() {
            reply_rid = net_find_reply_request(app);
            if reply_rid.is_some() && !emitted_net_gen {
                emitted_net_gen = true;
                emit_ev(app, "engine://state", "generating", "已捕获回复流（SSE /chat/completion）");
            }
        }
        let net_stream_done = reply_rid
            .as_deref()
            .map(|rid| net_reply_stream_finished(app, rid))
            .unwrap_or(false);

        // 【E-06】单次 CDP 调用同时获取按钮状态 + 文本末尾（原两次调用合并为一次）
        let poll_raw = cdp_eval(&wv, &poll_js).unwrap_or_default();
        let (mut vis_nonarrow, mut vis_arrow) = (false, false);
        let mut t = String::new();
        if let Ok(poll) = serde_json::from_str::<serde_json::Value>(&poll_raw) {
            // 按钮状态：poll.btn.items[]
            if let Some(arr) = poll.get("btn").and_then(|b| b.get("items")).and_then(|x| x.as_array()) {
                for it in arr {
                    let vis = it.get(0).and_then(|x| x.as_i64()).unwrap_or(0) == 1;
                    let head = it.get(3).and_then(|x| x.as_str()).unwrap_or("");
                    if vis {
                        if head.starts_with(&sel.arrow_prefix) {
                            vis_arrow = true;
                        } else if !head.is_empty() {
                            vis_nonarrow = true;
                        }
                    }
                }
            }
            // 文本末尾：poll.tail.t
            if let Some(tail_t) = poll.get("tail").and_then(|tl| tl.get("t")).and_then(|x| x.as_str()) {
                t = tail_t.to_string();
            }
        }
        if vis_nonarrow && !saw_gen {
            saw_gen = true;
            gen_started_at = Some(SystemTime::now());
            emitted_gen = true;
            emit_ev(app, "engine://state", "generating", "检测到生成中（stop 图标）");
        }
        arrow_ticks = if vis_arrow && saw_gen { arrow_ticks + 1 } else { 0 };
        // 【选择器失效检测】stop 按钮已出现（证明 DeepSeek 接受了消息并开始生成），
        // 但持续读不到助手文本 → 助手消息选择器几乎肯定已失效。
        // 15s 告警，30s 直接报错（不再傻等 180s 超时）。
        if saw_gen && t.is_empty() {
            if let Some(gs) = gen_started_at {
                let elapsed = gs.elapsed().unwrap_or_default();
                if elapsed.as_secs() >= 15 && !selector_warning_emitted {
                    selector_warning_emitted = true;
                    emit_ev(
                        app,
                        "engine://state",
                        "error",
                        "生成已开始 15s 但读不到助手文本 —— 选择器可能已失效，请运行自检",
                    );
                    chain_log("SELECTOR FAILURE: generation started (stop icon seen) but text empty for 15s");
                }
                if elapsed.as_secs() >= 30 {
                    emit_ev(
                        app,
                        "engine://state",
                        "error",
                        "选择器失效：生成已开始 30s 但助手文本始终为空，已终止回合",
                    );
                    chain_log("SELECTOR FAILURE: aborted after 30s of empty text post-generation");
                    return Err(
                        "selector failure: generation confirmed via stop button but assistant text unreadable for 30s; run selfcheck".into(),
                    );
                }
            }
        }
        if !t.is_empty() && t == prev {
            stable += 1;
        } else {
            stable = 0;
            prev = t;
        }

        let mut done = false;
        if net_stream_done {
            emit_ev(app, "engine://state", "finalizing", "回复流已结束（网络层主判定），复核中");
            done = true;
        } else if vis_arrow && saw_gen && arrow_ticks >= sel.cooldown_ticks {
            emit_ev(app, "engine://state", "finalizing", "发送键回归，复核中");
            done = true;
        } else if stable >= sel.fallback_stable && !prev.is_empty() {
            emit_ev(app, "engine://state", "finalizing", "文本稳定（兜底判定）");
            done = true;
        }
        if done {
            // 【网络层读回·主信源】直接从 SSE 正文提取回答，完全不依赖 DeepSeek 的 CSS 类名。
            // 注意：DOM/图标信号可能比 SSE 流关闭早几秒 → 先给流一个收尾窗口再读，
            // 避免「DOM 先触发 → 退回读到用户消息/页面文案」这类错误。
            // 【②】跟随重连的新 requestId：DeepSeek 断流后会发起新的 /chat/completion。
            // 若只认第一个 rid，就会一直读那条已结束的旧流，永远拿不到后续内容。
            if let Some(latest) = net_find_reply_request(app) {
                if reply_rid.as_deref() != Some(latest.as_str()) {
                    chain_log(&format!(
                        "net: 发现更新的回复流 rid={}（原 {:?}）→ 改读最新",
                        latest, reply_rid
                    ));
                    reply_rid = Some(latest);
                }
            }
            let net_text: Option<String> = match reply_rid.as_deref() {
                Some(rid) => {
                    for _ in 0..12 {
                        if net_reply_stream_finished(app, rid) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(400));
                    }
                    if !net_reply_stream_finished(app, rid) {
                        chain_log("net: 等待 SSE 流结束超时（约 5s）→ 回退 DOM 读回");
                        None
                    } else {
                        std::thread::sleep(Duration::from_millis(200));
                        match net_read_reply_body(app, rid) {
                            Ok(body) => {
                                let _ = std::fs::write(format!(r"{}\sse_body.txt", data_dir()), &body);
                                let t = parse_sse_assistant_text(&body);
                                // 思考过程：DeepSeek 把 THINK 片段放在 SSE 的 fragments 里。
                                // 单独提取并广播，让界面能像 DSH 那样显示"想到哪了"。
                                let think = crate::net::parse_sse_thinking(&body);
                                if !think.is_empty() {
                                    chain_log(&format!(
                                        "think: 提取思考 {} 字符",
                                        think.chars().count()
                                    ));
                                    emit_ev(app, "engine://state", "think", &think);
                                }
                                chain_log(&format!(
                                    "net: SSE 正文 {} 字节 / finished={} → 解析出回答 {} 字符",
                                    body.len(),
                                    sse_declares_finished(&body),
                                    t.chars().count()
                                ));
                                if t.trim().is_empty() {
                                    chain_log("net: SSE 解析为空 → 回退 DOM 读回");
                                    None
                                } else if !sse_declares_finished(&body) {
                                    // 【①】流关了但流内没宣告 FINISHED = 疑似断流/暂停。
                                    // 此时正文可能只有半截，采用它会把半截回复当成最终答案。
                                    chain_log(
                                        "net: 流已关但未宣告 FINISHED（疑似断流）→ 不采用该正文，交给 DOM 复核",
                                    );
                                    None
                                } else {
                                    Some(t)
                                }
                            }
                            Err(e) => {
                                chain_log(&format!("net: 读取 SSE 正文失败：{e} → 回退 DOM 读回"));
                                None
                            }
                        }
                    }
                }
                None => {
                    chain_log("net: 本回合未捕获到 /chat/completion 回复流 → 回退 DOM 读回");
                    None
                }
            };

            // DOM 读回复核（保留为兜底与截断防护）
            let read_full = || -> String {
                let full_raw = cdp_eval(&wv, &full_js).unwrap_or_default();
                let text = json_path(&full_raw, &["t"])
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                let src = json_path(&full_raw, &["src"])
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "?".into());
                let diag = if let Some(v) = json_path(&full_raw, &["diag"]) {
                    if let Some(arr) = v.as_array() {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect::<Vec<String>>()
                            .join(" | ")
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                if text.is_empty() {
                    chain_log(&format!("read_full EMPTY diag: raw={}", full_raw.chars().take(500).collect::<String>()));
                } else {
                    chain_log(&format!("read_full OK src={src} len={} diag={diag}", text.chars().count()));
                }
                text
            };
            std::thread::sleep(Duration::from_millis(1500));
            let snap1 = read_full();
            let mut stop_again = false;
            if let Ok(poll_raw) = cdp_eval(&wv, &poll_js) {
                if let Ok(poll) = serde_json::from_str::<serde_json::Value>(&poll_raw) {
                    if let Some(arr) = poll.get("btn").and_then(|b| b.get("items")).and_then(|x| x.as_array()) {
                        for it in arr {
                            let vis = it.get(0).and_then(|x| x.as_i64()).unwrap_or(0) == 1;
                            let head = it.get(3).and_then(|x| x.as_str()).unwrap_or("");
                            if vis && !head.is_empty() && !head.starts_with(&sel.arrow_prefix) {
                                stop_again = true;
                            }
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(1500));
            let snap2 = read_full();

            // 落盘网络取证（诊断用）
            {
                let net_json = net_dump(app);
                let _ = std::fs::write(format!(r"{}\net_probe.json", data_dir()), &net_json);
            }

            let log_tail = |s: &str| -> String {
                let c: Vec<char> = s.chars().collect();
                c[c.len().saturating_sub(24)..].iter().collect()
            };

            // ① 网络层正文（权威）：非空即采用。
            if let Some(text) = net_text {
                chain_log(&format!(
                    "done: 网络层读回 {} 字符, tail={:?}",
                    text.chars().count(),
                    log_tail(&text)
                ));
                emit_ev(app, "engine://state", "done", &format!("完成，{} 字符（网络层）", text.chars().count()));
                return Ok(text);
            }

            // ② DOM 兜底：仍需复核（防中途暂停误判截断）
            if snap2.trim().is_empty() {
                chain_log("completion: 网络层与 DOM 读回均为空（疑似发送/思考竞态）→ 继续等待");
                arrow_ticks = 0;
                stable = 0;
                prev = String::new();
                continue;
            }
            if looks_like_echo(&snap2, message) {
                chain_log("completion: DOM 读回命中了刚发送的用户消息（回声）→ 丢弃并继续等待网络层");
                arrow_ticks = 0;
                stable = 0;
                prev = String::new();
                continue;
            }
            if stop_again || snap2 != snap1 {
                chain_log(&format!(
                    "completion: 复核发现仍在生成（stop_again={stop_again}, grew={}）→ 继续等待",
                    snap2 != snap1
                ));
                arrow_ticks = 0;
                stable = 0;
                prev = String::new();
                continue;
            }
            chain_log(&format!(
                "done: DOM 兜底读回 {} 字符, tail={:?}",
                snap2.chars().count(),
                log_tail(&snap2)
            ));
            emit_ev(app, "engine://state", "done", &format!("完成，{} 字符（DOM 兜底）", snap2.chars().count()));
            return Ok(snap2);
        }
        if !emitted_gen && t0.elapsed().map(|d| d.as_secs() >= 30).unwrap_or(false) {
            emit_ev(app, "engine://state", "generating", "等待生成中…（未捕获 stop 状态，走文本兜底）");
            emitted_gen = true;
        }
    }
}
