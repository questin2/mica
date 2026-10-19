//! **内置工具**（策划案 2.0 · P1.5）：不依赖外部进程、不联网、纯计算。
//!
//! 存在的意义不只是"多两个工具"，而是**证明工具注册表真的能容纳非 PowerShell 的工具**——
//! 它们通过通用 JSON 工具协议调用（`{"name":"calc","arguments":{"expr":"1+2*3"}}`），
//! 后续 MCP 工具接进来时走的是同一条通道。
//!
//! 安全：`calc` **不 eval**，用自写的递归下降解析器；只认数字与四则运算，
//! 其余一律报错。内存与耗时都由输入长度自然约束（无循环、无递归深度攻击面之外的放大）。

use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};

use serde_json::Value;

/// 一个内置工具的定义。
pub struct Builtin {
    pub name: &'static str,
    pub description: &'static str,
    /// 写进提示词的参数示例（模型照此构造 arguments）
    pub args_example: &'static str,
}

const NOW: Builtin = Builtin {
    name: "now",
    description: "获取当前时间（UTC）。无需参数；返回 unix 毫秒时间戳与 ISO-8601 UTC 字符串。",
    args_example: r#"{}"#,
};

const CALC: Builtin = Builtin {
    name: "calc",
    description: "计算一个算术表达式（仅支持 + - * / % ^ 与括号、小数；不支持变量与函数）。",
    args_example: r#"{"expr":"(1+2)*3"}"#,
};

const WEB_READ: Builtin = Builtin {
    name: "web_read",
    description: "读取一个 http/https 网页并返回纯文本（需要用户审批；单次响应上限 2MB）。",
    args_example: r#"{"url":"https://example.com"}"#,
};

const REMEMBER: Builtin = Builtin {
    name: "remember",
    description: "把一条**长期有用**的事实写进长期记忆（跨会话保留，下次会话自动注入提示词）。只记稳定事实与偏好（如「显示器 150% DPI」「文档用 python-docx 生成」），不要记临时状态；重复 key 会覆盖旧值。",
    args_example: r#"{"key":"显示器DPI","value":"150%"}"#,
};

const RECALL: Builtin = Builtin {
    name: "recall",
    description: "读回长期记忆（不传 key 则列出全部）。写入后可用它确认是否真的记住了。",
    args_example: r#"{"key":"显示器DPI"}"#,
};

const FORGET: Builtin = Builtin {
    name: "forget",
    description: "删除一条长期记忆（key 写错时用来纠正）。",
    args_example: r#"{"key":"显示器DPI"}"#,
};

const SEARCH: Builtin = Builtin {
    name: "search",
    description: "联网搜索关键词，返回若干条（标题 / 链接 / 摘要）。**不知道网址时先用它**，再对结果里的 URL 调 web_read 或 browse 取正文。",
    args_example: r#"{"query":"豆包 最新版本","limit":5}"#,
};

const SEE: Builtin = Builtin {
    name: "see",
    description: "把本地图片（png/jpg/jpeg/webp/gif/bmp/svg/tif/avif）或 PDF 附加到对话里让模型**真的看到**它 —— 用于截图理解、图表、设计稿、报错弹窗。附加后下一轮提问模型即可读到该图。不做 OCR。",
    args_example: r#"{"path":"C:\\Users\\me\\Desktop\\shot.png"}"#,
};

const BROWSE: Builtin = Builtin {
    name: "browse",
    description: "用**真实浏览器**（离屏 Chromium）打开一个 URL，等 JS 渲染稳定后返回**可见文本**。适用于 web_read 拿不到内容的动态页面（SPA、需要登录的页面、搜索结果页）。",
    args_example: r#"{"url":"https://example.com","wait_ms":6000}"#,
};

const BROWSE_CLICK: Builtin = Builtin {
    name: "browse_click",
    description: "在当前浏览器页面里点击第一个匹配 CSS 选择器的元素，并返回点击后的可见文本。配合 browse 使用。",
    args_example: r#"{"selector":"button.submit","wait_ms":1000}"#,
};

const BROWSE_FILL: Builtin = Builtin {
    name: "browse_fill",
    description: "给当前页面的输入框填值（可选提交表单），并返回操作后的可见文本。",
    args_example: r#"{"selector":"input[name=q]","text":"Mica","submit":true}"#,
};

const BROWSE_READ: Builtin = Builtin {
    name: "browse_read",
    description: "读回当前浏览器页面的可见文本（不导航），用于点击/跳转后再次确认内容。",
    args_example: r#"{}"#,
};

pub fn all() -> &'static [Builtin] {
    &[NOW, CALC, WEB_READ, SEARCH, SEE, REMEMBER, RECALL, FORGET, BROWSE, BROWSE_CLICK, BROWSE_FILL, BROWSE_READ]
}

pub fn find(name: &str) -> Option<&'static Builtin> {
    all().iter().find(|b| b.name == name)
}

/// 执行内置工具。失败必须返回明确错误（不静默）。
pub fn execute(name: &str, args: &Value) -> Result<String, String> {
    match name {
        "now" => Ok(exec_now()),
        "calc" => {
            let expr = args
                .get("expr")
                .and_then(|v| v.as_str())
                .ok_or("calc 需要字符串参数 expr")?;
            let v = calc(expr)?;
            // 整数结果不带小数点，避免 42 显示成 42.0
            if v.fract() == 0.0 && v.abs() < 1e15 {
                Ok(format!("{}", v as i64))
            } else {
                Ok(format!("{v}"))
            }
        }
        "web_read" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("web_read 需要字符串参数 url")?;
            web_read(url)
        }
        "search" => {
            let q = args
                .get("query")
                .or_else(|| args.get("queries"))
                .and_then(|v| {
                    v.as_str().map(|s| s.to_string()).or_else(|| {
                        // 模型有时把 queries 写成数组
                        v.as_array().map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str())
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                    })
                })
                .ok_or("search 需要字符串参数 query")?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
            web_search(&q, limit)
        }
        // 记忆类工具需要 AppHandle（访问 SQLite），由 agent.rs 拦截执行；
        // 走到这里说明调用链漏了拦截，明确报错而不是静默失败。
        "remember" | "recall" | "forget" => {
            Err(format!("{name} 需要应用上下文，应由 agent 层处理"))
        }

        _ => Err(format!("未知内置工具：{name}")),
    }
}

// ---------- web_read ----------

/// 读取网页正文。安全边界（策划案 2.0 §5.7）：
/// - 仅 http/https；
/// - 解析 DNS 后拒绝环回/私网/链路本地/元数据地址（含每一次重定向）；
/// - 不自动跟随重定向，逐跳校验；
/// - 20s 超时、2MB 响应上限、输出截断到 8000 字符。
pub fn web_read(url: &str) -> Result<String, String> {
    let mut current = reqwest::Url::parse(url.trim()).map_err(|e| format!("URL 不合法：{e}"))?;
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(8))
        .user_agent("Mica/0.2 web_read")
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;

    for _ in 0..5 {
        validate_public_url(&current)?;
        let resp = client
            .get(current.clone())
            .send()
            .map_err(|e| format!("请求失败：{e}"))?;
        let status = resp.status();
        if status.is_redirection() {
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("重定向缺少 Location")?;
            current = current.join(loc).map_err(|e| format!("重定向地址不合法：{e}"))?;
            continue;
        }
        if !status.is_success() {
            return Err(format!("HTTP {}：{}", status.as_u16(), current));
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut buf = Vec::new();
        resp.take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut buf)
            .map_err(|e| format!("读取响应失败：{e}"))?;
        if buf.len() > 2 * 1024 * 1024 {
            return Err("响应超过 2MB 上限，已中止".to_string());
        }
        let raw = String::from_utf8_lossy(&buf).to_string();
        let text = if content_type.contains("html") {
            crate::memory::kb::strip_html(&raw)
        } else {
            raw
        };
        let out: String = text.chars().take(8000).collect();
        return Ok(format!("URL: {current}  {out}"));
    }
    Err("重定向次数过多（上限 5）".into())
}

pub(crate) fn validate_public_url(u: &reqwest::Url) -> Result<(), String> {
    let scheme = u.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("只允许 http/https，收到「{scheme}」"));
    }
    let host = u.host_str().ok_or("URL 缺少主机名")?;
    if host.eq_ignore_ascii_case("localhost") {
        return Err("不允许访问 localhost（SSRF 防护）".into());
    }
    let port = u.port_or_known_default().unwrap_or(80);
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("解析主机失败：{e}"))?;
    let mut any = false;
    for a in addrs {
        any = true;
        if is_private_ip(a.ip()) {
            return Err(format!("不允许访问内网/本机地址 {}（SSRF 防护）", a.ip()));
        }
    }
    if !any {
        return Err("主机没有解析到任何地址".into());
    }
    Ok(())
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                // 100.64.0.0/10（CGNAT）
                || (v4.octets()[0] == 100 && (64..=127).contains(&v4.octets()[1]))
                // 198.18.0.0/15（基准测试）
                || (v4.octets()[0] == 198 && (v4.octets()[1] == 18 || v4.octets()[1] == 19))
                // 0.0.0.0/8
                || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 unique local
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10 link local
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // IPv4-mapped：按 IPv4 规则再判一次
                || v6.to_ipv4_mapped().map(|v4| is_private_ip(IpAddr::V4(v4))).unwrap_or(false)
        }
    }
}

// ---------- now ----------

fn exec_now() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    format!(r#"{{"unix_ms":{ms},"utc":"{}"}}"#, utc_iso(ms))
}

/// unix 毫秒 → ISO-8601 UTC（`YYYY-MM-DDTHH:MM:SSZ`）。
///
/// 不引 chrono：日期换算用「自 1970-01-01 起的天数 → 公历年月日」的经典算法
/// （Howard Hinnant 的 civil_from_days）。
pub fn utc_iso(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// days since 1970-01-01 → (year, month, day)
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    // 算法出处：http://howardhinnant.github.io/date_algorithms.html#civil_from_days
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------- calc ----------

/// 递归下降求值：expr := term (('+'|'-') term)*
pub fn calc(input: &str) -> Result<f64, String> {
    if input.trim().is_empty() {
        return Err("表达式为空".into());
    }
    if input.len() > 1000 {
        return Err("表达式过长（上限 1000 字符）".into());
    }
    let mut p = Parser { s: input.as_bytes(), i: 0 };
    let v = p.expr()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("表达式在第 {} 个字符处有多余内容", p.i + 1));
    }
    if !v.is_finite() {
        return Err("结果不是有限数（可能溢出或除以 0）".into());
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    /// expr := term (('+'|'-') term)*
    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        while let Some(c) = self.peek() {
            match c {
                b'+' => {
                    self.i += 1;
                    v += self.term()?;
                }
                b'-' => {
                    self.i += 1;
                    v -= self.term()?;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    /// term := unary (('*'|'/'|'%') unary)*
    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            match self.peek() {
                Some(b'*') => {
                    self.i += 1;
                    v *= self.unary()?;
                }
                Some(b'/') => {
                    self.i += 1;
                    let d = self.unary()?;
                    if d == 0.0 {
                        return Err("除以 0".into());
                    }
                    v /= d;
                }
                Some(b'%') => {
                    self.i += 1;
                    let d = self.unary()?;
                    if d == 0.0 {
                        return Err("对 0 取模".into());
                    }
                    v %= d;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    /// unary := ('+'|'-') unary | power
    fn unary(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Ok(-self.unary()?)
            }
            Some(b'+') => {
                self.i += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    /// power := primary ('^' unary)?   右结合
    fn power(&mut self) -> Result<f64, String> {
        let base = self.primary()?;
        if self.peek() == Some(b'^') {
            self.i += 1;
            let exp = self.unary()?;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<f64, String> {
        self.ws();
        match self.s.get(self.i).copied() {
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                self.ws();
                if self.s.get(self.i) != Some(&b')') {
                    return Err("括号未闭合".into());
                }
                self.i += 1;
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let start = self.i;
                let mut seen_dot = false;
                while let Some(&c) = self.s.get(self.i) {
                    if c.is_ascii_digit() {
                        self.i += 1;
                    } else if c == b'.' && !seen_dot {
                        seen_dot = true;
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                let text = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| "非法数字".to_string())?;
                text.parse::<f64>().map_err(|_| format!("非法数字：{text}"))
            }
            Some(c) => Err(format!("无法识别的字符：'{}'", c as char)),
            None => Err("表达式意外结束".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 搜索结果页解析（Bing 真实结构，不联网）
    #[test]
    fn search_results_are_parsed() {
        let html = r#"
<li class="b_algo"><h2><a href="https://chat.doubao.com/chat/" h="ID=SERP,1">豆包 - 字节跳动旗下 AI 智能助手</a></h2>
<div class="b_caption"><p class="b_lineclamp2" data-x="">Seedance 2.0 视频生成模型现已全面接入豆包。<a class="b_algoReadMore" href="https://chat.doubao.com/chat/">阅读更多</a></p></div></li>
<li class="b_algo"><h2><a href="https://example.com/v" h="ID=SERP,2">版本信息</a></h2>
<div class="b_caption"><p class="b_lineclamp2">最新版本 <b>2.31.4</b></p></div></li>
"#;
        let hits = parse_search_results(html, 5);
        assert_eq!(hits.len(), 2, "应解析出 2 条：{hits:?}");
        assert!(hits[0].0.contains("豆包"), "标题：{}", hits[0].0);
        assert_eq!(hits[0].1, "https://chat.doubao.com/chat/");
        assert!(hits[0].2.contains("Seedance"), "摘要：{}", hits[0].2);
        assert!(hits[1].2.contains("2.31.4"), "第二条摘要：{}", hits[1].2);
    }

    #[test]
    fn registry_has_expected_tools() {
        let names: Vec<&str> = all().iter().map(|b| b.name).collect();
        assert_eq!(names, vec!["now", "calc", "web_read", "search", "see", "remember", "recall", "forget", "browse", "browse_click", "browse_fill", "browse_read"]);
        assert!(find("calc").is_some());
        assert!(find("nope").is_none());
    }

    #[test]
    fn calc_handles_precedence_and_parens() {
        assert_eq!(calc("1+2*3").unwrap(), 7.0);
        assert_eq!(calc("(1+2)*3").unwrap(), 9.0);
        assert_eq!(calc("2*(3+4)-5").unwrap(), 9.0);
        assert_eq!(calc("10/4").unwrap(), 2.5);
        assert_eq!(calc("10%3").unwrap(), 1.0);
        assert_eq!(calc("2^10").unwrap(), 1024.0);
        assert_eq!(calc("-3+5").unwrap(), 2.0);
        assert_eq!(calc("-(2+3)").unwrap(), -5.0);
        assert_eq!(calc(" 1.5 * 2 ").unwrap(), 3.0);
    }

    #[test]
    fn calc_right_associative_power() {
        // 2^3^2 = 2^(3^2) = 512（右结合）
        assert_eq!(calc("2^3^2").unwrap(), 512.0);
    }

    #[test]
    fn calc_rejects_garbage_without_eval() {
        for bad in ["1+", "(1+2", "abc", "1/0", "1%0", "", "   ", "system('calc')", "1;2"] {
            assert!(calc(bad).is_err(), "应拒绝：{bad:?}");
        }
    }

    #[test]
    fn calc_rejects_overlong_input() {
        let long = "1+".repeat(600) + "1";
        assert!(calc(&long).is_err());
    }

    #[test]
    fn execute_calc_formats_integers_without_decimals() {
        assert_eq!(execute("calc", &json!({"expr":"1+2"})).unwrap(), "3");
        assert_eq!(execute("calc", &json!({"expr":"1/2"})).unwrap(), "0.5");
    }

    #[test]
    fn execute_calc_requires_expr() {
        assert!(execute("calc", &json!({})).is_err());
        assert!(execute("calc", &json!({"expr": 42})).is_err());
    }

    #[test]
    fn execute_unknown_is_error() {
        assert!(execute("nope", &json!({})).is_err());
    }

    #[test]
    fn utc_iso_formats_known_instants() {
        assert_eq!(utc_iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_iso(1_000), "1970-01-01T00:00:01Z");
        // 2026-10-17T00:00:00Z 的 epoch 秒
        let secs = 1_792_195_200u64;
        assert_eq!(utc_iso(secs * 1000), "2026-10-17T00:00:00Z");
    }

    #[test]
    fn now_returns_parseable_json() {
        let s = exec_now();
        let v: Value = serde_json::from_str(&s).expect("应是合法 JSON");
        assert!(v.get("unix_ms").and_then(|x| x.as_u64()).unwrap() > 0);
        assert!(v.get("utc").and_then(|x| x.as_str()).unwrap().ends_with('Z'));
    }

    #[test]
    fn private_ips_are_rejected() {
        for s in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            let ip: IpAddr = s.parse().unwrap();
            assert!(is_private_ip(ip), "应判为内网：{s}");
        }
        for s in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            let ip: IpAddr = s.parse().unwrap();
            assert!(!is_private_ip(ip), "不应判为内网：{s}");
        }
    }

    #[test]
    fn web_read_rejects_non_http_schemes() {
        let err = web_read("file:///C:/Windows/win.ini").unwrap_err();
        assert!(err.contains("http/https"), "got: {err}");
        let err = web_read("http://localhost:1234/").unwrap_err();
        assert!(err.contains("localhost"), "got: {err}");
    }

    #[test]
    fn web_read_requires_url_argument() {
        assert!(execute("web_read", &json!({})).is_err());
        assert!(execute("web_read", &json!({"url": 42})).is_err());
    }
}

// ---------- web_search：联网搜索（策划案 2.0 · 认知层）----------
//
// 为什么需要它：模型反复想调 `search` —— 之前**真的没有**这个工具，
// 于是它只能靠 PowerShell 硬啃搜索引擎页面，或者干脆瞎编工具名。
// `web_read` 只能读"已知 URL"，`browse` 也要先知道网址；缺的正是"从关键词到网址"这一步。
//
// 实现取向：用 DuckDuckGo 的 HTML 版（无 API key、无 JS），POST 表单拿结果页，
// 再手写扫描 `.result__a` / `.result__snippet`。不引入新依赖。

/// 去掉标签并还原常见实体（搜索结果标题/摘要用）。
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&ensp;", " ")
        .trim()
        .to_string()
}

/// 解析搜索结果页 → `Vec<(标题, 链接, 摘要)>`。
///
/// 目标是 **Bing**（`cn.bing.com`）的通用结果结构：
/// ```html
/// <h2><a href="https://…" …>标题</a></h2>
/// <div class="b_caption"><p class="b_lineclamp2" …>摘要…<a class="b_algoReadMore">阅读更多</a></p></div>
/// ```
/// 抽成独立函数是为了**可单测**：给一段真实结构 HTML，断言解析结果，不联网。
pub fn parse_search_results(html: &str, limit: usize) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    while out.len() < limit {
        // 结果标题锚点：<h2 …><a …href="http…" …>标题</a></h2>
        let Some(h2) = rest.find("<h2") else { break };
        let after = &rest[h2..];
        let Some(a) = after.find("<a") else { break };
        let after_a = &after[a..];
        let Some(hp) = after_a.find("href=\"http") else {
            // 本块没有外链（广告位等）→ 跳过这个 h2 继续找
            rest = &rest[h2 + 3..];
            continue;
        };
        let url_start = hp + 6;
        let Some(url_end) = after_a[url_start..].find('"') else { break };
        let url = after_a[url_start..url_start + url_end].to_string();
        // 标题 = 锚点 > 之后到 </a>
        let after_url = &after_a[url_start + url_end..];
        let title = after_url
            .find('>')
            .map(|gt| {
                let s = &after_url[gt + 1..];
                s.find("</a>").map(|e| strip_html(&s[..e])).unwrap_or_default()
            })
            .unwrap_or_default();
        // 摘要：往后找 b_lineclamp（Bing 的摘要 class），没有就留空
        let snippet = after_url
            .find("class=\"b_lineclamp")
            .or_else(|| after_url.find("class=\"b_caption"))
            .map(|p| {
                let s = &after_url[p..];
                let body = s.find('>').map(|gt| &s[gt + 1..]).unwrap_or(s);
                let end = body.find("</p>").unwrap_or(body.len());
                strip_html(&body[..end])
            })
            .unwrap_or_default();
        if !url.is_empty() && !title.is_empty() {
            out.push((title, url, snippet));
        }
        rest = &rest[h2 + 3..];
    }
    out
}

/// URL 查询参数编码。
fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            other => format!("%{other:02X}"),
        })
        .collect::<String>()
}

/// 联网搜索：关键词 → 若干条 (标题, 链接, 摘要)。
///
/// 后端选型（实测 2026-10-18）：**DuckDuckGo 在本机完全连不通**
/// （curl 20s 超时、TLS 握手失败 —— 网络受限），而 `cn.bing.com` 返回 200 且结构稳定。
/// 所以用 Bing。将来若换网络环境，只需改这里的 endpoint 与 parse_search_results。
pub fn web_search(query: &str, limit: usize) -> Result<String, String> {
    let q = query.trim();
    if q.is_empty() {
        return Err("search 需要非空 query".to_string());
    }
    let limit = limit.clamp(1, 10);
    let url = format!(
        "https://cn.bing.com/search?q={}&count={}&setlang=zh-CN&ensearch=0",
        url_encode(q),
        limit
    );
    let parsed = reqwest::Url::parse(&url).map_err(|e| format!("URL 不合法：{e}"))?;
    validate_public_url(&parsed)?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/140.0 Safari/537.36",
        )
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;

    let resp = client
        .get(parsed)
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .send()
        .map_err(|e| format!("搜索请求失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("搜索返回 HTTP {}", resp.status().as_u16()));
    }
    let mut buf = Vec::new();
    resp.take(4 * 1024 * 1024)
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取响应失败：{e}"))?;
    let html = String::from_utf8_lossy(&buf).to_string();

    let hits = parse_search_results(&html, limit);
    if hits.is_empty() {
        return Ok(format!(
            "搜索「{q}」没有解析到结果（可能被限流或页面结构变化）。可改用 web_read 直接读已知网址，或 browse 打开搜索页。"
        ));
    }
    let mut s = format!("搜索「{q}」的前 {} 条结果：\n\n", hits.len());
    for (i, (title, url, snippet)) in hits.iter().enumerate() {
        s.push_str(&format!("{}. {title}\n   {url}\n", i + 1));
        if !snippet.is_empty() {
            s.push_str(&format!("   摘要：{snippet}\n"));
        }
        s.push('\n');
    }
    s.push_str("提示：需要正文就对这些 URL 调 web_read（静态页）或 browse（动态渲染页）。");
    Ok(s)
}
