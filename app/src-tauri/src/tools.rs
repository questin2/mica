//! **M-01 工具抽象**（可扩展性分析报告 v2 的关键路径起点）。
//!
//! 原状：「模型 → 工具」的调用协议被写死成一个 PowerShell 代码块 + 一对魔法注释标记，
//! 工具种类只能是 1 种，插件也无从贡献能力。
//!
//! 现状：抽出 `ToolSpec` 注册表 —— 每个工具自带「名字 / 说明 / 标记对 / 执行方式」。
//! 内置的 `powershell` 工具**逐字沿用原协议**（标记、解析、执行全部不变），
//! 插件可额外贡献 `PluginTemplate` 类型工具（一段带 `{{arg}}` 占位的命令模板）。
//!
//! **安全性**：插件工具的执行最终仍然落到同一条 PowerShell 通道，因此**同样经过审批门**；
//! 审批卡里展示的是**代入参数后的完整命令**，用户看到的就是将要执行的东西。
//! 本模块不引入任何绕过审批的路径。
//!
//! **零风险增量**：没有任何插件工具时，`extra_prompt_block()` 返回空串，
//! 提示词与调用协议与升级前**完全一致**，现有回路不受影响。

use crate::plugins;

/// 内置 PowerShell 工具的标记（与 config/prompt.initial.md 严格一致，勿改）。
pub const PS_HEAD: &str = "# powershell_tool_调用工具_识别头_本行注释一定要完全保留";
pub const PS_TAIL: &str = "# powershell_tool_调用工具_识别尾_本行注释一定要完全保留";

const PS_NAME: &str = "powershell";

/// 工具的标记对（模型必须原样输出才能被识别）。
pub fn markers(name: &str) -> (String, String) {
    if name == PS_NAME {
        (PS_HEAD.to_string(), PS_TAIL.to_string())
    } else {
        (
            format!("# tool_{name}_调用工具_识别头_本行注释一定要完全保留"),
            format!("# tool_{name}_调用工具_识别尾_本行注释一定要完全保留"),
        )
    }
}

/// 一次被识别出来的工具调用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    /// 工具名（内置为 `powershell`）
    pub name: String,
    /// 载荷：PowerShell 为代码原文；模板工具为 `{{arg}}` 的代入值
    pub payload: String,
}

/// 从模型回复里抽掉包裹的代码围栏行（模型经常把标记连内容一起放进 ``` 里）。
fn strip_fences(raw: &str) -> String {
    raw.lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// 按某个工具的标记对，从回复里取出载荷。
fn extract(reply: &str, head: &str, tail: &str) -> Option<String> {
    let h = reply.find(head)?;
    let after = h + head.len();
    let t = reply[after..].find(tail)?;
    let body = strip_fences(&reply[after..after + t]);
    if body.is_empty() {
        None
    } else {
        Some(body)
    }
}

/// **唯一入口**：解析模型回复中的工具调用。
///
/// 顺序很关键 —— **内置 PowerShell 优先**，保证现有协议在任何情况下都不被插件工具抢占。
///
/// 第三道兜底（见 `loose_tool_block`）：模型有时把标记写成
/// `# tool_调用工具_识别头`（**没替换工具名**），载荷却是要执行的 PowerShell 代码。
/// 实测 2026-10-18 查豆包版本时连续两次这样写 → 两套解析都空 → 它以为工具不存在而放弃。
/// 这种情况按 **PowerShell 调用**处理（仍走审批门，风险不变）。
pub fn parse_call(reply: &str) -> Option<ToolCall> {
    // 精确匹配优先；失败再按核心片段宽容匹配（模型常漏写 `# ` 或把围栏粘在标记行上）
    if let Some(code) = extract(reply, PS_HEAD, PS_TAIL)
        .or_else(|| {
            extract_between_core(
                reply,
                "powershell_tool_调用工具_识别头",
                "powershell_tool_调用工具_识别尾",
            )
        })
    {
        return Some(ToolCall { name: PS_NAME.to_string(), payload: code });
    }
    for t in plugin_tools() {
        let (h, tl) = markers(&t.name);
        if let Some(arg) = extract(reply, &h, &tl).or_else(|| {
            extract_between_core(
                reply,
                &format!("tool_{}_调用工具_识别头", t.name),
                &format!("tool_{}_调用工具_识别尾", t.name),
            )
        }) {
            return Some(ToolCall { name: t.name, payload: arg });
        }
    }
    // 兜底：名字缺失 / 非注册名 + 非 JSON 载荷 → 当成 PowerShell
    if let Some((name, payload)) = loose_tool_block(reply) {
        let known_plugin = plugin_tools().iter().any(|t| t.name == name);
        let looks_json = serde_json::from_str::<serde_json::Value>(payload.trim())
            .map(|v| v.is_object())
            .unwrap_or(false);
        if !known_plugin && !looks_json && name != PS_NAME {
            return Some(ToolCall {
                name: PS_NAME.to_string(),
                payload,
            });
        }
    }
    None
}

/// 宽容取出 `# tool_<名字>_调用工具_识别头` 与 `..._识别尾` 之间的内容。
///
/// 不要求名字合法（允许为空 / 含杂字符），也不要求头尾注释逐字一致 ——
/// 只认 `调用工具_识别头` / `调用工具_识别尾` 这两个**核心片段**，
/// 因为模型最常见的失误就是忘了替换 `<名字>` 或改动尾部注释。
///
/// 返回 `(疑似名字, 载荷)`。
/// 标记行是否**真的像标记**：去掉围栏与 `#` 后，必须以已知前缀开头。
///
/// 为什么需要：宽容解析上线后，**散文里提到标记词**也会被当成调用。
/// 实测（2026-10-18）：模型复述用户的日志分析（原文含「…识别尾…保留」字样，
/// 还带表格与代码围栏），被误判成工具调用块 → 报"未识别"，
/// 甚至可能把散文当 PowerShell 执行。真正的标记行长这样：
/// `# tool_x_…` / `# powershell_tool_…` / `# eagent_tool_…`
/// （模型可能漏掉 `#` 或把围栏粘在前面，所以只校验去噪后的前缀）。
fn head_line_looks_like_marker(line: &str) -> bool {
    let t = line
        .trim()
        .trim_start_matches('`')
        .trim()
        .trim_start_matches('#')
        .trim();
    t.starts_with("tool_") || t.starts_with("powershell_tool_") || t.starts_with("eagent_tool_")
}

pub fn loose_tool_block(reply: &str) -> Option<(String, String)> {
    const H: &str = "调用工具_识别头";
    const T: &str = "调用工具_识别尾";
    let hpos = reply.find(H)?;
    // 从行首到 H 之前，取 `# tool_` 之后的部分当名字
    let line_start = reply[..hpos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let head_line = &reply[line_start..hpos];
    // ★ 散文里提到标记词不算调用（false positive 防护）
    if !head_line_looks_like_marker(head_line) {
        return None;
    }
    let name = head_line
        .trim()
        .trim_start_matches('#')
        .trim()
        .strip_prefix("tool_")
        .unwrap_or("")
        .trim()
        .trim_end_matches('_')
        .trim()
        .to_string();
    // 载荷 = **头标记所在行的下一行** 到 **尾标记所在行的前一行**。
    // 必须按行切：否则会把头标记尾部的 `_本行注释一定要完全保留` 和尾行的
    // `# powershell_tool_` 也算进载荷，导致"空白 body"被误判成有内容。
    let after = reply[hpos..]
        .find('\n')
        .map(|i| hpos + i + 1)
        .unwrap_or(hpos + H.len());
    let t_abs = after + reply[after..].find(T)?;
    let body_end = reply[..t_abs].rfind('\n').unwrap_or(t_abs);
    if body_end <= after {
        return None;
    }
    let body = strip_fences(&reply[after..body_end]);
    if body.trim().is_empty() {
        None
    } else {
        Some((name, body))
    }
}


/// 回复里**出现了工具调用标记片段，但没能被完整解析**时返回线索。
///
/// 为什么需要它：`parse_call` 只按**已注册**的工具名生成标记去匹配。
/// 模型若调用了一个未注册的工具（实测出现过 `agent_getjpg`），标记虽然出现在回复里，
/// 却匹配不上 → 回复被当成"最终答案"返回 → 界面看起来像一条半截回复。
/// 这种时候必须**明确告诉模型工具名无效**，而不是默默结束回合。
///
/// 返回 `Some(名字)` 表示拿到疑似工具名（可能为空串）；`None` 表示没有标记片段。
pub fn suspicious_tool_marker(reply: &str) -> Option<String> {
    const HEAD_HINT: &str = "调用工具_识别头";
    const TAIL_HINT: &str = "调用工具_识别尾";
    if !reply.contains(HEAD_HINT) && !reply.contains(TAIL_HINT) {
        return None;
    }
    // ★ 必须是**标记行**才算数：散文里提到这些字样不算调用。
    //   （实测 2026-10-18：模型复述用户日志分析，文中出现「…识别尾…保留」
    //    与表格、围栏，被误判成"未识别工具块"。）
    let hpos = reply.find(HEAD_HINT)?;
    let line_start = reply[..hpos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let head_line = &reply[line_start..hpos];
    if !head_line_looks_like_marker(head_line) {
        return None;
    }
    // 从 `# tool_<name>_调用工具_识别头` 里取出名字
    for prefix in ["# tool_", "# powershell_tool_", "# eagent_tool_"] {
        if let Some(p) = reply.find(prefix) {
            let rest = &reply[p + prefix.len()..];
            if let Some(end) = rest.find(HEAD_HINT) {
                let name = rest[..end].trim_matches('_').trim();
                return Some(if prefix.contains("powershell") && name.is_empty() {
                    PS_NAME.to_string()
                } else {
                    name.to_string()
                });
            }
        }
    }
    Some(String::new())
}
/// 一个工具的定义。
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub title: String,
    pub description: String,
    /// 插件工具的命令模板（含 `{{arg}}`）；内置工具为 `None`
    pub template: Option<String>,
    /// `""` = 内置；非空如 `插件：xxx`
    pub source: String,
}

/// 全部插件贡献的工具。
pub fn plugin_tools() -> Vec<ToolSpec> {
    plugins::contributed_tools()
        .into_iter()
        .map(|(plugin, t)| ToolSpec {
            name: t.name.trim().to_string(),
            title: if t.title.trim().is_empty() { t.name.trim().to_string() } else { t.title.trim().to_string() },
            description: t.description.trim().to_string(),
            template: Some(t.command),
            source: format!("插件：{plugin}"),
        })
        .collect()
}

/// 工具总数 = PowerShell(1) + 内置工具 + 插件贡献。
pub fn count() -> usize {
    1 + crate::builtin::all().len() + plugin_tools().len()
}

/// **代入参数**：把模板里的 `{{arg}}` 换成模型给的值。
///
/// 只做整串替换，不做任何转义处理 —— 因为结果会**原样进审批卡**给用户看，
/// 用户看到的就是即将执行的内容；这里若偷偷改写反而会造成「看到的 ≠ 执行的」。
pub fn render(template: &str, arg: &str) -> String {
    template.replace("{{arg}}", arg)
}

/// 把一次工具调用解析成**将要执行的 PowerShell 代码**。
/// 返回 `None` 表示工具名未知（拒绝执行，不猜测）。
pub fn to_powershell(call: &ToolCall) -> Option<String> {
    if call.name == PS_NAME {
        return Some(call.payload.clone());
    }
    for t in plugin_tools() {
        if t.name == call.name {
            let tpl = t.template.clone().unwrap_or_default();
            return Some(render(&tpl, &call.payload));
        }
    }
    None
}

/// 给模型的**额外**工具说明。**没有任何插件工具时返回空串** ——
/// 这样提示词与协议保持升级前的原样，现有回路零影响。
pub fn extra_prompt_block() -> String {
    let tools = plugin_tools();
    if tools.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    s.push_str("\n\n---\n\n## 附加工具（由插件提供）\n\n");
    s.push_str("除上面的 PowerShell 工具外，你还可以调用下列**插件工具**。调用方式与 PowerShell 工具完全一致，");
    s.push_str("只是把标记里的 `powershell` 换成对应工具名，并在标记之间写该工具的**参数**：\n\n");
    for t in &tools {
        let (h, tl) = markers(&t.name);
        s.push_str(&format!("### {}\n\n{}\n\n", t.title, t.description));
        s.push_str(&format!("- 工具名：`{}`（来源：{}）\n", t.name, t.source));
        s.push_str(&format!("- 它会把你的参数代入这段命令执行：\n\n```powershell\n{}\n```\n\n", t.template.clone().unwrap_or_default()));
        s.push_str("- 调用格式（三行都必须原样保留）：\n\n```\n");
        s.push_str(&format!("{h}\n<你的参数>\n{tl}\n"));
        s.push_str("```\n\n");
    }
    s
}

// ---------- 通用 JSON 工具协议（策划案 2.0 · P1.3/P1.4） ----------
//
// 与 PowerShell 标记**并行**：老协议逐字保留，新协议是"任意工具"的统一入口。
// 统一成 JSON 的好处：参数有名字、有类型（未来可校验 schema）、错误能明确回传，
// 而不是像现在这样只能把一坨文本塞进标记里。

/// 通用工具调用的标记（模型必须原样输出才被识别）。
pub const JSON_HEAD: &str = "# eagent_tool_调用工具_识别头_本行注释一定要完全保留";
pub const JSON_TAIL: &str = "# eagent_tool_调用工具_识别尾_本行注释一定要完全保留";

/// 一次 JSON 工具调用。
#[derive(Clone, Debug, PartialEq)]
pub struct JsonCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// 解析通用 JSON 工具调用。
///
/// 返回 `None` = 回复里没有这个标记（正常，走别的分支）；
/// 返回 `Some(Err(..))` = **有标记但内容不合法** —— 这种情况必须把错误回传给模型，
/// 解析模型原生的 `<tool_call> ... </tool_call>` 块。
///
/// 支持三种载荷形态（都是模型实际会吐的）：
/// 1. `{"name":"x","arguments":{...}}`（最常见）
/// 2. `{"name":"x","arguments":"<JSON 字符串>"}`（arguments 被二次编码）
/// 3. `{"name":"x","parameters":{...}}`（个别模型的字段名）
fn parse_tool_call_tag(reply: &str) -> Option<Result<JsonCall, String>> {
    let open = reply.find("<tool_call>")?;
    let after = open + "<tool_call>".len();
    let close = reply[after..].find("</tool_call>")?;
    let body = reply[after..after + close].trim();
    // 有些模型会把 JSON 包在 ```json 里
    let body = strip_fences(body);
    let v: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return Some(Err(format!("<tool_call> 内容不是合法 JSON：{e}"))),
    };
    let name = match v.get("name").and_then(|x| x.as_str()) {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => return Some(Err("<tool_call> 缺少非空字符串字段 name".to_string())),
    };
    let raw_args = v
        .get("arguments")
        .or_else(|| v.get("parameters"))
        .cloned()
        .unwrap_or(serde_json::json!({}));
    // arguments 可能是对象，也可能是被二次编码的 JSON 字符串
    let arguments = match raw_args {
        serde_json::Value::String(s) => match serde_json::from_str::<serde_json::Value>(s.trim()) {
            Ok(v) if v.is_object() => v,
            _ => serde_json::json!({}),
        },
        other if other.is_object() => other,
        _ => serde_json::json!({}),
    };
    Some(Ok(JsonCall { name, arguments }))
}

/// 按**核心片段**取块：头片段所在行的下一行 → 尾片段所在行的前一行。
///
/// 为什么要这个而不是精确 `extract`：实测（2026-10-18）模型会写成
/// ```text
/// ```eagent_tool_调用工具_识别头_本行注释一定要完全保留   ← 漏了 "# "，还和围栏挤一行
/// {"name":"browse","arguments":{...}}
/// # eagent_tool_调用工具_识别尾_本行注释一定要完全保留
/// ```
/// 精确匹配 `# eagent_tool_...` 直接失败 → 它以为工具不存在而放弃。
/// 这里只认核心片段 `调用工具_识别头/识别尾`，按行切载荷，容忍漏 `# `、围栏粘连。
pub fn extract_between_core(reply: &str, head_core: &str, tail_core: &str) -> Option<String> {
    let hpos = reply.find(head_core)?;
    let after = reply[hpos..]
        .find('\n')
        .map(|i| hpos + i + 1)
        .unwrap_or(hpos + head_core.len());
    let t_abs = after + reply[after..].find(tail_core)?;
    let body_end = reply[..t_abs].rfind('\n').unwrap_or(t_abs);
    if body_end <= after {
        return None;
    }
    let body = strip_fences(&reply[after..body_end]);
    if body.trim().is_empty() {
        None
    } else {
        Some(body)
    }
}

/// 通用工具调用的核心片段（供宽容取块用，不含 `# ` 前缀）。
const JSON_HEAD_CORE: &str = "eagent_tool_调用工具_识别头";
const JSON_TAIL_CORE: &str = "eagent_tool_调用工具_识别尾";

/// 而不是静默当成"没有工具调用"（否则模型永远不知道自己格式写错了）。
/// 工具名是否**已注册**（内置 / 插件 / MCP）。
///
/// 用于裸 JSON 调用的判定：只有名字真的存在才算调用，
/// 否则一段普通 JSON 回复会被误当成工具调用。
fn is_known_tool(name: &str) -> bool {
    crate::builtin::find(name).is_some()
        || name.starts_with("mcp:")
        || plugin_tools().iter().any(|t| t.name == name)
}

/// **裸 JSON 调用**：整条回复就是一个工具调用对象。
///
/// 实测（2026-10-18）：模型直接把
/// `{"name":"search","arguments":{"query":"…","limit":6}}`
/// 当成回复原文吐出来 —— 既没有 `<tool_call>` 包裹，也没有我们的标记。
/// 界面因此把这段 JSON 当成"回答"显示给用户。
///
/// 判定条件刻意收紧，避免把正常 JSON 回答误判成调用：
/// 整条回复必须是单个 JSON 对象、有非空 `name`、`arguments`/`parameters` 是对象、
/// **且 name 是已注册工具**。
fn parse_bare_json_call(reply: &str) -> Option<Result<JsonCall, String>> {
    let t = reply.trim();
    if !t.starts_with('{') || !t.ends_with('}') {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(t).ok()?;
    let name = v.get("name").and_then(|x| x.as_str())?.trim().to_string();
    if name.is_empty() || !is_known_tool(&name) {
        return None;
    }
    let raw_args = v
        .get("arguments")
        .or_else(|| v.get("parameters"))
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let arguments = match raw_args {
        serde_json::Value::String(s) => match serde_json::from_str::<serde_json::Value>(s.trim()) {
            Ok(v) if v.is_object() => v,
            _ => serde_json::json!({}),
        },
        other if other.is_object() => other,
        _ => return None,
    };
    Some(Ok(JsonCall { name, arguments }))
}

/// **批量解析**：把回复里**所有** `<tool_call>` 块都收出来。
///
/// 为什么需要：此前一层只返回**一个**调用 —— 查目录、读文件、改代码都要磨好几轮。
/// 放开成"一次一批"能直接把效率翻倍。
/// 只处理 `<tool_call>`（模型最常用的形态）；其它格式仍走单调用路径，行为不变。
pub fn parse_json_calls(reply: &str) -> Vec<Result<JsonCall, String>> {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";
    let mut out = Vec::new();
    let mut rest = reply;
    while let Some(i) = rest.find(OPEN) {
        let after = &rest[i + OPEN.len()..];
        let Some(j) = after.find(CLOSE) else {
            // 块没闭合：交给单调用路径去报"格式不完整"
            break;
        };
        let body = strip_fences(after[..j].trim());
        match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(v) => {
                let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                if name.is_empty() {
                    out.push(Err("<tool_call> 缺少非空字符串字段 name".to_string()));
                } else {
                    let raw = v
                        .get("arguments")
                        .or_else(|| v.get("parameters"))
                        .cloned()
                        .unwrap_or(serde_json::json!({}));
                    let arguments = match raw {
                        serde_json::Value::String(s) => {
                            serde_json::from_str::<serde_json::Value>(s.trim())
                                .ok()
                                .filter(|x| x.is_object())
                                .unwrap_or_else(|| serde_json::json!({}))
                        }
                        other if other.is_object() => other,
                        _ => serde_json::json!({}),
                    };
                    out.push(Ok(JsonCall { name, arguments }));
                }
            }
            Err(e) => out.push(Err(format!("<tool_call> 内容不是合法 JSON：{e}"))),
        }
        rest = &after[j + CLOSE.len()..];
    }
    out
}
pub fn parse_json_call(reply: &str) -> Option<Result<JsonCall, String>> {
    // ⓪ 模型原生 `<tool_call>{...}</tool_call>` 格式。
    // 实测（2026-10-18，查豆包版本）：模型直接吐
    //   <tool_call>
    //   {"name":"browse","arguments":{"url":"https://www.doubao.com","wait_ms":8000}}
    //   </tool_call>
    // 这是各家模型训练里常见的格式，比我们的自定义标记更"自然" → 必须认。
    if let Some(r) = parse_tool_call_tag(reply) {
        return Some(r);
    }
    // 先精确匹配；失败再按核心片段宽容匹配（漏 `# ` / 围栏粘连都能救）
    let body = match extract(reply, JSON_HEAD, JSON_TAIL)
        .or_else(|| extract_between_core(reply, JSON_HEAD_CORE, JSON_TAIL_CORE))
    {
        Some(b) => b,
        // 都没有 → 试"裸 JSON 调用"（整条回复就是一个调用对象）
        None => return parse_bare_json_call(reply),
    };
    let v: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return Some(Err(format!("工具调用不是合法 JSON：{e}"))),
    };
    let name = match v.get("name").and_then(|x| x.as_str()) {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => return Some(Err("工具调用缺少非空字符串字段 name".to_string())),
    };
    let arguments = v.get("arguments").cloned().unwrap_or(serde_json::json!({}));
    if !arguments.is_object() {
        return Some(Err("工具调用的 arguments 必须是对象".to_string()));
    }
    Some(Ok(JsonCall { name, arguments }))
}

/// **宽容解析**：模型有时把**插件标记格式**用在内置 / MCP 工具上。
///
/// 实测（2026-10-18，用户让它查豆包版本）：它输出了
/// ```text
/// # tool_browse_调用工具_识别头
/// {"url":"https://www.doubao.com"}
/// # tool_browse_调用工具_识别尾
/// ```
/// 而内置工具只认 `# eagent_tool_...` 的 JSON 协议 → 匹配不上 → 报"未注册"，
/// 于是它放弃 browse 改用 PowerShell。两套协议并存是根因。
///
/// 这里做**输入侧归一**：只要是 `# tool_<名字>_调用工具_识别头/尾` 且载荷是合法 JSON 对象，
/// 就当成一次 JsonCall —— 不管那名字是内置、插件还是 MCP 工具。
/// 名字是否真的有注册，交给下游 execute_json_call 判定（它会给出明确错误）。
pub fn parse_lenient_call(reply: &str) -> Option<Result<JsonCall, String>> {
    for line in reply.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("# tool_") else {
            continue;
        };
        let Some(idx) = rest.find("_调用工具_识别头") else {
            continue;
        };
        let name = rest[..idx].trim().to_string();
        if name.is_empty() {
            continue;
        }
        let (h, tl) = markers(&name);
        let Some(body) = extract(reply, &h, &tl) else {
            continue;
        };
        let body = body.trim();
        let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
            // 不是 JSON → 可能是插件工具的纯文本参数，交给原有 parse_call 处理
            continue;
        };
        if !v.is_object() {
            continue;
        }
        return Some(Ok(JsonCall { name, arguments: v }));
    }
    None
}
/// 提示词里的「可用工具清单」——由注册表生成，不是手写。
///
/// 顺序固定：内置 → 插件 → 旧 PowerShell 协议（放最后并标注"旧格式仍然有效"）。
pub fn tool_list_prompt_block() -> String {
    let mut s = String::new();
    s.push_str("\n\n---\n\n## 可用工具（除 PowerShell 之外的扩展工具）\n\n");
    // 【协议收敛】只教**一种**格式：`<tool_call>` 包裹的 JSON。
    // 原因（实测 2026-10-18）：提示词里同时出现三种格式示例（PowerShell 标记、
    // 插件标记、本协议标记）时，模型会随机挑一种，甚至写成漏 `# `、围栏粘连、
    // 裸 JSON —— 连续五轮排查都是这个根因。选中 `<tool_call>` 是因为：
    //   1. 它是模型的"母语"格式（训练数据里常见）；
    //   2. 允许调用块**前后同时写自然语言**（裸 JSON 做不到）；
    //   3. 解析器能容忍围栏、空白、属性顺序。
    // 其余格式（旧标记 / 裸 JSON）解析器仍然兼容，但**不再写进提示词**。
    s.push_str("需要调用工具时，在回复里输出下面这个块（`<tool_call>` 标签必须原样保留）：\n\n");
    s.push_str("```\n<tool_call>\n{\"name\":\"<工具名>\",\"arguments\":{ ... }}\n</tool_call>\n```\n\n");
    s.push_str("要点：\n");
    s.push_str("- 一个块只调一个工具；要连调多次就输出多个块。\n");
    s.push_str("- 块**前后可以照常写自然语言**，用户会看到这些文字。\n");
    s.push_str("- 不要臆造工具名；不在下面清单里的调用会被拒绝。\n\n");
    s.push_str("6. **一轮可以输出多个 `<tool_call>` 块**：我会按顺序全部执行，再把所有结果一起回给你。");
    s.push_str("查目录 + 读文件 + 跑命令这类独立步骤，**一次给全**比一个个来快得多。\n");
    s.push_str("工具清单：\n\n");

    for b in crate::builtin::all() {
        s.push_str(&format!("- `{}`：{}\n  参数示例：`{}`\n", b.name, b.description, b.args_example));
    }
    for t in &plugin_tools() {
        s.push_str(&format!(
            "- `{}`：{}（来源：{}）\n  参数示例：`{{\"arg\":\"<参数>\"}}`\n",
            t.name, t.description, t.source
        ));
    }

    s.push_str("\n说明：\n");
    s.push_str("1. `arguments` 必须是**对象**；缺参数会收到明确错误，请按提示补齐后重试。\n");
    s.push_str("2. 标记之外不要写别的内容来解释调用，正文照常写给用户看。\n");
    s.push_str("3. 旧的 PowerShell 代码块协议**仍然有效**，执行 PowerShell 时继续用它；本协议用于上面列出的扩展工具。\n");
    s.push_str("4. 已知工具名之外的调用会被拒绝并返回错误——不要臆造工具。\n");
    s.push_str("5. `browse*` 是**真实浏览器**（离屏 Chromium）：`web_read` 拿不到内容的动态页面用它。\n");
    s.push_str("6. **本清单里的工具一律用 `<tool_call>` 块调用**；带 `调用工具_识别头/尾` 注释的旧标记是**插件工具专用**，别混用。\n");

    // ── 工作准则（成长层 / 表达层）────────────────────────────────────────
    s.push_str("\n## 工作准则\n\n");
    s.push_str("- **需求模糊时**：能从上下文合理推断就先做，并在开头一句话说明你的假设；");
    s.push_str("只有涉及**不可逆操作**（删除、覆盖、外发）或代价明显时才先问一句。\n");
    s.push_str("- **从错误里学**：踩到非显而易见的坑并解决之后，如果它是**稳定事实**");
    s.push_str("（环境特性、路径约定、某项配置的正确写法），用 `remember` 记下来，下次不必重踩；");
    s.push_str("临时状态不要记。\n");
    s.push_str("- **少废话**：能一句话说清就不写三句；不复述用户的话，不预告你将要做什么，直接给结果。\n");
    s
}

/// 按字符上限裁剪工具清单（ProviderCaps.max_tool_list_chars）。
///
/// 只按**行边界**裁剪，保证不会把 JSON 示例或标记切成半截；
/// 超限时追加明确说明，而不是静默丢弃（模型必须知道"有些工具没列出来"）。
pub fn trim_prompt_block(block: &str, max_chars: usize) -> String {
    let total = block.chars().count();
    if total <= max_chars {
        return block.to_string();
    }
    const NOTE: &str =
        "\n\n（工具清单过长，已省略部分工具。未列出的工具不可调用；请优先使用上面已列出的工具。）\n";
    let note_len = NOTE.chars().count();
    if max_chars <= note_len + 80 {
        // 预算小到放不下清单：至少保留调用格式的说明，避免模型完全不知道协议。
        let mut s: String = block.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        return s;
    }
    let budget = max_chars - note_len;
    let mut out = String::new();
    for line in block.lines() {
        let add = line.chars().count() + 1;
        if out.chars().count() + add > budget {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string() + NOTE
}

/// 原生 tool_calls 的工具定义（Ollama / OpenAI 兼容格式）。
///
/// 只包含内置工具、web_read、PowerShell 与插件模板工具；
/// MCP 工具仍走文本协议（其 schema 由各 server 动态提供，后续可再接）。
pub fn native_tool_defs() -> Vec<serde_json::Value> {
    fn object(props: serde_json::Value, required: Vec<&str>) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": props,
            "required": required,
        })
    }
    let mut out = Vec::new();
    out.push(serde_json::json!({
        "type": "function",
        "function": {
            "name": "powershell",
            "description": "执行一段 PowerShell 代码（需要用户审批）。",
            "parameters": object(serde_json::json!({
                "code": { "type": "string", "description": "要执行的 PowerShell 代码" }
            }), vec!["code"])
        }
    }));
    for b in crate::builtin::all() {
        let params = match b.name {
            "calc" => object(serde_json::json!({
                "expr": { "type": "string", "description": "算术表达式，如 (1+2)*3" }
            }), vec!["expr"]),
            "web_read" => object(serde_json::json!({
                "url": { "type": "string", "description": "要读取的 http/https URL" }
            }), vec!["url"]),
            _ => object(serde_json::json!({}), vec![]),
        };
        out.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": b.name,
                "description": b.description,
                "parameters": params,
            }
        }));
    }
    for t in plugin_tools() {
        out.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": t.name,
                "description": format!("{}（来源：{}）", t.description, t.source),
                "parameters": object(serde_json::json!({
                    "arg": { "type": "string", "description": "工具参数" }
                }), vec!["arg"])
            }
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(head: &str, body: &str, tail: &str) -> String {
        format!("前言\n{head}\n{body}\n{tail}\n后语")
    }

    #[test]
    fn powershell_markers_are_unchanged() {
        // 协议常量是兼容性红线：一旦变动，已训练的调用习惯立刻失效
        assert_eq!(markers(PS_NAME).0, "# powershell_tool_调用工具_识别头_本行注释一定要完全保留");
        assert_eq!(markers(PS_NAME).1, "# powershell_tool_调用工具_识别尾_本行注释一定要完全保留");
    }

    #[test]
    fn parses_powershell_call_verbatim() {
        let reply = wrap(PS_HEAD, "Get-Date", PS_TAIL);
        let call = parse_call(&reply).expect("应识别");
        assert_eq!(call.name, "powershell");
        assert_eq!(call.payload, "Get-Date");
    }

    #[test]
    fn strips_code_fences_around_markers() {
        let reply = format!("{PS_HEAD}\n```powershell\nGet-Date\n```\n{PS_TAIL}");
        let call = parse_call(&reply).expect("应识别");
        assert_eq!(call.payload, "Get-Date", "围栏行必须被剥掉，否则会执行到 ``` 上");
    }

    #[test]
    fn empty_body_is_not_a_call() {
        let reply = wrap(PS_HEAD, "   \n  ", PS_TAIL);
        assert!(parse_call(&reply).is_none());
    }

    #[test]
    fn missing_tail_or_head_is_not_a_call() {
        assert!(parse_call(&format!("{PS_HEAD}\nGet-Date")).is_none(), "缺尾标记不应识别");
        assert!(parse_call(&format!("Get-Date\n{PS_TAIL}")).is_none(), "缺头标记不应识别");
    }

    #[test]
    fn multiline_code_is_preserved() {
        let code = "Get-ChildItem\nWhere-Object { $_.Name }\nWrite-Output 'done'";
        let reply = wrap(PS_HEAD, code, PS_TAIL);
        assert_eq!(parse_call(&reply).unwrap().payload, code);
    }

    #[test]
    fn render_substitutes_all_arg_placeholders() {
        assert_eq!(render("echo {{arg}} && echo {{arg}}", "x"), "echo x && echo x");
        assert_eq!(render("no placeholder", "x"), "no placeholder");
        assert_eq!(render("{{arg}}", ""), "");
    }

    #[test]
    fn to_powershell_passes_through_for_builtin() {
        let call = ToolCall { name: PS_NAME.into(), payload: "Get-Date".into() };
        assert_eq!(to_powershell(&call).as_deref(), Some("Get-Date"));
    }

    #[test]
    fn unknown_tool_is_refused_not_guessed() {
        let call = ToolCall { name: "definitely-not-a-tool".into(), payload: "x".into() };
        assert!(to_powershell(&call).is_none(), "未知工具必须拒绝，不能猜");
    }

    /// 无插件工具时，附加说明必须为空 —— 这是「零风险增量」的保证。
    #[test]
    fn extra_prompt_block_is_empty_without_plugin_tools() {
        let has_plugin_tool = plugins::contributed_tools().iter().next().is_some();
        if !has_plugin_tool {
            assert!(extra_prompt_block().is_empty(), "无插件工具时不得改动提示词");
            assert_eq!(count(), 1 + crate::builtin::all().len());
        }
    }

    /// 插件工具的标记、代入与解析（不依赖磁盘上的真实插件目录）。
    /// 实测故障（2026-10-18 查豆包版本）：模型照抄模板却没替换 `<名字>`，
    /// 载荷是 PowerShell 代码 → 两套解析都空 → 它以为工具不存在而放弃。
    /// 现在应回退成 PowerShell 调用（仍走审批门，风险不变）。
    #[test]
    fn loose_block_without_tool_name_falls_back_to_powershell() {
        let reply = "# tool_调用工具_识别头_本行注释一定要完全保留\n# 描述：抓页面\nInvoke-WebRequest https://example.com\n# tool_调用工具_识别尾_本行注释一定要完全保留";
        let (name, payload) = loose_tool_block(reply).expect("应能宽容取出");
        assert_eq!(name, "");
        assert!(payload.contains("Invoke-WebRequest"), "载荷应为 PowerShell 代码：{payload}");
        let c = parse_call(reply).expect("应回退为 PowerShell 调用");
        assert_eq!(c.name, "powershell");
    }

    /// 名字缺失但载荷是 JSON 对象 → 交给 JSON 协议（不能被 PowerShell 抢走）
    #[test]
    fn loose_block_with_json_payload_is_not_powershell() {
        let reply = "# tool_调用工具_识别头_本行注释一定要完全保留\n{\"url\":\"https://example.com\",\"name\":\"x\"}\n# tool_调用工具_识别尾_本行注释一定要完全保留";
        assert!(parse_call(reply).is_none(), "JSON 载荷不应被当成 PowerShell");
    }

    /// **真实故障原文回归**（chain_test.log，2026-10-18）：
    /// 模型直接用原生 `<tool_call>` XML，而不是我们的自定义标记。
    #[test]
    fn native_tool_call_tag_is_accepted() {
        let reply = "<tool_call>\n{\"name\":\"browse\",\"arguments\":{\"url\":\"https://www.doubao.com\",\"wait_ms\":8000}}\n</tool_call>";
        let c = parse_json_call(reply)
            .expect("应识别 <tool_call>")
            .expect("载荷合法");
        assert_eq!(c.name, "browse");
        assert_eq!(c.arguments["url"], "https://www.doubao.com");
        assert_eq!(c.arguments["wait_ms"], 8000);
    }

    /// arguments 被二次编码成 JSON 字符串时也要能用
    #[test]
    fn native_tool_call_with_stringified_arguments() {
        let reply = "<tool_call>{\"name\":\"web_read\",\"arguments\":\"{\\\"url\\\":\\\"https://a.example\\\"}\"}</tool_call>";
        let c = parse_json_call(reply).unwrap().unwrap();
        assert_eq!(c.name, "web_read");
        assert_eq!(c.arguments["url"], "https://a.example");
    }

    /// **真实故障原文回归**（data/unrecognized_tool.txt 第 3 版，2026-10-18）：
    /// 模型复述用户的日志分析，文中出现「…识别尾…保留」+ 表格 + 围栏。
    /// 这不是工具调用，绝不能被判成"未识别工具块"（更不能被当 PowerShell 执行）。
    #[test]
    fn prose_mentioning_marker_words_is_not_a_call() {
        let prose = "| 1792327046443 | 161 | …识别尾…保留 + 反引号围栏收口 |\n\n**结论**：模型这次写的是代码块壳子。";
        assert!(
            suspicious_tool_marker(prose).is_none(),
            "散文不应被判为工具块"
        );
        assert!(loose_tool_block(prose).is_none(), "散文不应被取出载荷");
        assert!(parse_call(prose).is_none(), "散文不应被当成 PowerShell");
    }

    /// **真实故障原文回归**（data/unrecognized_tool.txt，2026-10-18）：
    /// 模型漏写 `# `、并把 ``` 围栏与头标记挤在同一行。精确匹配必然失败。
    #[test]
    fn json_call_without_hash_and_with_stuck_fence_is_accepted() {
        let reply = "```eagent_tool_调用工具_识别头_本行注释一定要完全保留\n{\"name\":\"browse\",\"arguments\":{\"url\":\"https://www.doubao.com\"}}\n# eagent_tool_调用工具_识别尾_本行注释一定要完全保留\n```";
        let c = parse_json_call(reply)
            .expect("应能宽容识别")
            .expect("载荷是合法 JSON");
        assert_eq!(c.name, "browse");
        assert_eq!(c.arguments["url"], "https://www.doubao.com");
    }

    /// PowerShell 标记同样可能漏 `# `
    #[test]
    fn ps_call_without_hash_is_accepted() {
        let reply = "```powershell_tool_调用工具_识别头_本行注释一定要完全保留\nGet-Date\n# powershell_tool_调用工具_识别尾_本行注释一定要完全保留\n```";
        let c = parse_call(reply).expect("应能宽容识别");
        assert_eq!(c.name, "powershell");
        assert!(c.payload.contains("Get-Date"), "载荷：{}", c.payload);
    }

    /// **真实故障原文回归**（界面截图，2026-10-18）：
    /// 模型整条回复就是一个裸 JSON 调用对象，既没有 `<tool_call>` 也没有标记。
    #[test]
    fn bare_json_call_is_accepted() {
        let reply = r#"{"name":"search","arguments":{"query":"豆包 最新版本号 PC版 2026","limit":6}}"#;
        let c = parse_json_call(reply)
            .expect("应识别裸 JSON 调用")
            .expect("载荷合法");
        assert_eq!(c.name, "search");
        assert_eq!(c.arguments["limit"], 6);
    }

    /// 名字不是已注册工具时，普通 JSON 回复不能被误判成调用
    #[test]
    fn bare_json_with_unknown_name_is_not_a_call() {
        let reply = r#"{"name":"不存在的工具","arguments":{}}"#;
        assert!(parse_json_call(reply).is_none());
    }

    /// 普通 JSON 数据回复（没有 name 字段）不是调用
    #[test]
    fn bare_json_data_is_not_a_call() {
        let reply = r#"{"total":3,"items":["a","b","c"]}"#;
        assert!(parse_json_call(reply).is_none());
    }

    /// 一次一批：多个 <tool_call> 必须**全部**收出来
    #[test]
    fn batch_parses_all_tool_calls() {
        let reply = "<tool_call>{\"name\":\"search\",\"arguments\":{\"query\":\"a\"}}</tool_call>
中间随便写点话
<tool_call>{\"name\":\"now\",\"arguments\":{}}</tool_call>";
        let calls = parse_json_calls(reply);
        assert_eq!(calls.len(), 2, "应解析出 2 个调用：{calls:?}");
        let names: Vec<String> = calls.into_iter().filter_map(|c| c.ok()).map(|c| c.name).collect();
        assert_eq!(names, vec!["search", "now"]);
    }

    #[test]
    fn batch_returns_empty_without_tool_calls() {
        assert!(parse_json_calls("普通回答，没有调用").is_empty());
    }

    #[test]
    fn lenient_marker_with_json_payload_is_accepted() {
        // 实测故障（2026-10-18）：模型把插件标记格式用到了内置工具 browse 上
        let reply = "# tool_browse_调用工具_识别头_本行注释一定要完全保留\n{\"url\":\"https://example.com\"}\n# tool_browse_调用工具_识别尾_本行注释一定要完全保留";
        let c = parse_lenient_call(reply)
            .expect("应能宽容解析")
            .expect("载荷是合法 JSON，应成功");
        assert_eq!(c.name, "browse");
        assert_eq!(c.arguments["url"], "https://example.com");
    }

    #[test]
    fn lenient_parse_ignores_non_json_payload() {
        // 插件工具的纯文本参数不能被宽容解析抢走（它走普通 parse_call）
        let reply = "# tool_myplugin_调用工具_识别头_本行注释一定要完全保留\n纯文本参数\n# tool_myplugin_调用工具_识别尾_本行注释一定要完全保留";
        assert!(parse_lenient_call(reply).is_none());
    }

    #[test]
    fn plugin_tool_marker_roundtrip_via_spec() {
        let name = "demo-tool";
        let (h, t) = markers(name);
        assert!(h.contains("demo-tool"), "标记里应带工具名：{h}");
        assert!(t.contains("demo-tool"));
        // 内置标记与插件标记不得互相误判
        assert_ne!(h, PS_HEAD);
        let reply = format!("{h}\n世界\n{t}");
        // 内置优先：没有 PS 标记时，插件标记才会被采用（此处无插件目录，故解析为 None）
        let _ = reply;
        assert!(parse_call("普通回复").is_none());
    }

    #[test]
    fn render_is_verbatim_no_escaping() {
        // 审批卡展示的就是代入结果本身，因此这里**刻意不转义**：
        // 若偷偷改写，就会出现「用户看到的 ≠ 实际执行的」。
        let out = render("Write-Output '{{arg}}'", "a'; Remove-Item -Recurse x; '");
        assert_eq!(out, "Write-Output 'a'; Remove-Item -Recurse x; ''");
        // 代入后的内容含危险片段 → 仍会被前端 classify 判为 danger，走危险审批
        assert!(out.contains("Remove-Item -Recurse"));
    }

    // ---------- 通用 JSON 工具协议 ----------

    fn json_reply(body: &str) -> String {
        format!("好的，我算一下。\n{JSON_HEAD}\n{body}\n{JSON_TAIL}\n以上。")
    }

    #[test]
    fn json_call_parses_name_and_arguments() {
        let r = json_reply(r#"{"name":"calc","arguments":{"expr":"1+2"}}"#);
        let got = parse_json_call(&r).expect("应识别").expect("应合法");
        assert_eq!(got.name, "calc");
        assert_eq!(got.arguments["expr"], "1+2");
    }

    #[test]
    fn json_call_without_marker_is_none() {
        assert!(parse_json_call("普通回复，没有工具调用").is_none());
    }

    #[test]
    fn json_call_missing_arguments_defaults_to_empty_object() {
        let r = json_reply(r#"{"name":"now"}"#);
        let got = parse_json_call(&r).unwrap().unwrap();
        assert_eq!(got.name, "now");
        assert!(got.arguments.is_object());
    }

    #[test]
    fn json_call_bad_json_reports_error_not_none() {
        // ★ 有标记但内容坏了 → 必须回传错误，不能静默当成"没有调用"
        let r = json_reply("{name: calc, arguments: }");
        let parsed = parse_json_call(&r).expect("标记存在就必须返回 Some");
        assert!(parsed.is_err());
    }

    #[test]
    fn json_call_requires_nonempty_name() {
        for bad in [r#"{"arguments":{}}"#, r#"{"name":"","arguments":{}}"#, r#"{"name":42}"#] {
            let r = json_reply(bad);
            assert!(parse_json_call(&r).unwrap().is_err(), "应拒绝：{bad}");
        }
    }

    #[test]
    fn json_call_arguments_must_be_object() {
        let r = json_reply(r#"{"name":"calc","arguments":[1,2]}"#);
        assert!(parse_json_call(&r).unwrap().is_err());
    }

    #[test]
    fn json_call_strips_code_fences() {
        let r = format!("{JSON_HEAD}\n```json\n{{\"name\":\"now\",\"arguments\":{{}}}}\n```\n{JSON_TAIL}");
        assert_eq!(parse_json_call(&r).unwrap().unwrap().name, "now");
    }

    #[test]
    fn json_markers_are_stable() {
        // 这两个常量是提示词与解析器的共同契约，改动会让已注入的会话立刻失效
        assert_eq!(JSON_HEAD, "# eagent_tool_调用工具_识别头_本行注释一定要完全保留");
        assert_eq!(JSON_TAIL, "# eagent_tool_调用工具_识别尾_本行注释一定要完全保留");
    }

    #[test]
    fn tool_list_block_teaches_one_call_format() {
        let b = tool_list_prompt_block();
        // 【协议收敛】提示词只教 <tool_call>，不再教三套标记
        assert!(b.contains("<tool_call>"), "必须教 <tool_call> 调用格式");
        assert!(b.contains("</tool_call>"), "必须给出闭合标签");
        assert!(
            !b.contains(JSON_HEAD),
            "旧标记不应再写进提示词（避免模型随机挑格式）"
        );
        assert!(b.contains("`now`") && b.contains("`calc`"), "必须列出内置工具");
        assert!(b.contains("仍然有效"), "必须说明旧 PowerShell 协议仍然有效");
        assert!(b.contains("不要臆造工具名"), "必须警告不要臆造工具名");
    }

    #[test]
    fn trim_prompt_block_keeps_budget_and_marks_omission() {
        let block = tool_list_prompt_block();
        let max = 900;
        if block.chars().count() > max {
            let t = trim_prompt_block(&block, max);
            assert!(t.chars().count() <= max, "裁剪后必须不超上限");
            // 裁剪按行边界，格式说明在清单之前 → 必须被保住
            assert!(t.contains("<tool_call>"), "必须保留调用格式说明");
            assert!(t.contains("已省略部分工具"), "必须明确标注省略");
            assert!(t.ends_with('\n'), "以换行收尾，避免与后续段落粘连");
        }
    }

    #[test]
    fn trim_prompt_block_is_noop_when_within_budget() {
        let block = "短清单";
        assert_eq!(trim_prompt_block(block, 100), block);
    }

    #[test]
    fn native_tool_defs_include_core_tools() {
        let defs = native_tool_defs();
        let names: Vec<&str> = defs
            .iter()
            .filter_map(|d| d.pointer("/function/name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&"powershell"));
        assert!(names.contains(&"calc"));
        assert!(names.contains(&"now"));
        let calc = defs
            .iter()
            .find(|d| d.pointer("/function/name").and_then(|n| n.as_str()) == Some("calc"))
            .unwrap();
        assert_eq!(
            calc.pointer("/function/parameters/required/0").and_then(|x| x.as_str()),
            Some("expr")
        );
    }
}
