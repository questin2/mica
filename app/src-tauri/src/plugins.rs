//! **最小可行插件系统**（可扩展性分析报告 v2 · L-01 的第一步）。
//!
//! 定位：报告把完整插件系统（沙箱 / 签名 / 市场）列为 3–6 个月的高难度项，
//! 依赖 M-01（工具抽象）与 M-02（MCP 客户端）。本模块**不做那些**，
//! 只落地一个「插件能贡献能力」的最小闭环，且清单格式为后续扩展预留：
//!
//! ```text
//! <data_dir>/plugins/<目录名>/plugin.json
//! {
//!   "name": "my-plugin",
//!   "version": "1.0.0",
//!   "description": "一句话说明",
//!   "skills": [ { "name": "skill-a", "desc": "能干什么", "cat": "工具" } ]
//! }
//! ```
//!
//! 本轮支持：**插件贡献技能**（自动并入技能库、注入提示词、界面标注来源）。
//! 将来接 M-01/M-02 时，同一清单再加 `"tools"` / `"mcp"` 字段即可，不需要换机制。

use serde::{Deserialize, Serialize};

/// 供 serde 用：缺省 true（旧清单无此字段时插件仍然生效）。
fn default_true() -> bool { true }

/// 插件清单（`plugin.json` 的结构）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginManifest {
    /// 是否启用（缺省 true —— 旧清单没有这个字段，必须继续生效）
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// 该插件贡献的技能。
    #[serde(default)]
    pub skills: Vec<PluginSkill>,
    /// 该插件贡献的工具（M-01 工具抽象之后新增的扩展点）。
    ///
    /// 工具是一段带 `{{arg}}` 占位的 PowerShell 命令模板：模型给出参数 → 代入 → 执行。
    /// **执行仍然经过审批门**，审批卡展示的是代入后的完整命令（所见即所执行）。
    #[serde(default)]
    pub tools: Vec<PluginTool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginSkill {
    pub name: String,
    #[serde(default)]
    pub desc: String,
    #[serde(default)]
    pub cat: String,
}

/// 插件贡献的一个工具。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginTool {
    /// 工具名（用于标记：`# tool_<name>_调用工具_识别头…`）。必填。
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// 命令模板，`{{arg}}` 会被模型给的参数整串替换。必填。
    #[serde(default)]
    pub command: String,
}

/// 已扫描到的插件（含目录与可能的解析错误，便于界面显示"为什么没加载"）。
#[derive(Clone, Debug, Serialize)]
pub struct Plugin {
    pub dir: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub skill_count: usize,
    pub tool_count: usize,
    /// 解析失败原因；`None` 表示加载正常。
    pub error: Option<String>,
}

/// 一次扫描的全部产出。
#[derive(Default)]
pub struct ScanResult {
    pub plugins: Vec<Plugin>,
    pub skills: Vec<(String, PluginSkill)>,
    pub tools: Vec<(String, PluginTool)>,
}

/// 插件根目录（用户把插件文件夹丢这里）。
pub fn plugins_dir() -> String {
    format!(r"{}\plugins", crate::config::data_dir())
}

/// 只保留**名字与命令模板都非空**的工具 —— 半成品条目不应污染工具表，
/// 否则会出现「模型看得见但一调用就执行空命令」的坑。
pub fn valid_tools(tools: &[PluginTool]) -> Vec<PluginTool> {
    tools
        .iter()
        .filter(|t| !t.name.trim().is_empty() && !t.command.trim().is_empty())
        .cloned()
        .collect()
}

/// 扫描 `<plugins>/<dir>/plugin.json`。
pub fn scan() -> ScanResult {
    let root = plugins_dir();
    let mut out = ScanResult::default();

    let Ok(entries) = std::fs::read_dir(&root) else {
        return out; // 目录不存在 = 没有插件，不算错误
    };

    for e in entries.flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest_path = dir.join("plugin.json");
        let dir_s = dir.to_string_lossy().to_string();

        let raw = match std::fs::read_to_string(&manifest_path) {
            Ok(s) => s,
            Err(_) => {
                out.plugins.push(Plugin {
                    dir: dir_s,
                    name: String::new(),
                    version: String::new(),
                    description: String::new(),
                    skill_count: 0,
                    tool_count: 0,
                    error: Some("缺少 plugin.json".into()),
                });
                continue;
            }
        };

        match serde_json::from_str::<PluginManifest>(&raw) {
            Ok(m) if !m.enabled => {
                continue; // 已停用：不并入技能/工具，也不报错
            }
            Ok(m) if !m.name.trim().is_empty() => {
                let ns = m.skills.len();
                for s in &m.skills {
                    out.skills.push((m.name.clone(), s.clone()));
                }
                // 只有「名字与命令模板都非空」的工具才算有效，避免半成品插件污染工具表
                let vt = valid_tools(&m.tools);
                let nt = vt.len();
                for t in &vt {
                    out.tools.push((m.name.clone(), t.clone()));
                }
                out.plugins.push(Plugin {
                    dir: dir_s,
                    name: m.name.clone(),
                    version: m.version.clone(),
                    description: m.description.clone(),
                    skill_count: ns,
                    tool_count: nt,
                    error: None,
                });
            }
            Ok(_) => out.plugins.push(Plugin {
                dir: dir_s,
                name: String::new(),
                version: String::new(),
                description: String::new(),
                skill_count: 0,
                tool_count: 0,
                error: Some("plugin.json 缺少 name".into()),
            }),
            Err(e) => out.plugins.push(Plugin {
                dir: dir_s,
                name: String::new(),
                version: String::new(),
                description: String::new(),
                skill_count: 0,
                tool_count: 0,
                error: Some(format!("plugin.json 解析失败：{e}")),
            }),
        }
    }

    out.plugins.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// 仅取插件贡献的技能（供 skills 模块合并）。
pub fn contributed_skills() -> Vec<(String, PluginSkill)> {
    scan().skills
}

/// 仅取插件贡献的工具（供 tools 模块合并）。
pub fn contributed_tools() -> Vec<(String, PluginTool)> {
    scan().tools
}

/// 确保插件目录存在（首次运行时给用户一个明确的位置放插件）。
pub fn ensure_dir() -> Result<String, String> {
    let d = plugins_dir();
    std::fs::create_dir_all(&d).map_err(|e| format!("建插件目录失败：{e}"))?;
    Ok(d)
}

// ---------- 插件详情 / 创建 / 删除（供「插件」页使用） ----------

/// 插件详情：把一个插件**贡献了什么**摊开给界面看。
#[derive(Clone, Debug, Serialize)]
pub struct PluginDetail {
    pub dir: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub skills: Vec<PluginSkill>,
    pub tools: Vec<PluginTool>,
    /// 解析失败原因；`None` 表示正常
    pub error: Option<String>,
    /// 是否已停用（停用的插件不进技能库/工具表，但仍在列表里可见）
    #[serde(default)]
    pub disabled: bool,
}

fn err_detail(dir_s: String, msg: String) -> PluginDetail {
    PluginDetail {
        dir: dir_s,
        name: String::new(),
        version: String::new(),
        description: String::new(),
        skills: vec![],
        tools: vec![],
        error: Some(msg),
        disabled: false,
    }
}

/// 逐个插件返回完整详情（含其贡献的技能与工具明细）。
pub fn details() -> Vec<PluginDetail> {
    let root = plugins_dir();
    let mut out: Vec<PluginDetail> = Vec::new();

    let Ok(entries) = std::fs::read_dir(&root) else {
        return out;
    };

    for e in entries.flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let dir_s = dir.to_string_lossy().to_string();
        let raw = match std::fs::read_to_string(dir.join("plugin.json")) {
            Ok(s) => s,
            Err(_) => {
                out.push(err_detail(dir_s, "缺少 plugin.json".into()));
                continue;
            }
        };
        match serde_json::from_str::<PluginManifest>(&raw) {
            Ok(m) if !m.enabled => out.push(PluginDetail {
                dir: dir_s,
                name: m.name.trim().to_string(),
                version: m.version.trim().to_string(),
                description: m.description.trim().to_string(),
                skills: m.skills,
                tools: valid_tools(&m.tools),
                error: None,
                disabled: true,
            }),
            Ok(m) if !m.name.trim().is_empty() => out.push(PluginDetail {
                dir: dir_s,
                name: m.name.trim().to_string(),
                version: m.version.trim().to_string(),
                description: m.description.trim().to_string(),
                skills: m.skills,
                tools: valid_tools(&m.tools),
                error: None,
                disabled: false,
            }),
            Ok(_) => out.push(err_detail(dir_s, "plugin.json 缺少 name".into())),
            Err(e) => out.push(err_detail(dir_s, format!("plugin.json 解析失败：{e}"))),
        }
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// 从插件名生成一个安全的目录名（只保留字母数字与 `-` `_`，其余转 `-`）。
pub fn safe_dir_name(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let mut collapsed = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.chars() {
        if c == '-' {
            if prev_dash {
                continue;
            }
            prev_dash = true;
        } else {
            prev_dash = false;
        }
        collapsed.push(c);
    }
    collapsed.trim_matches('-').to_string()
}

/// 新建插件目录 + 写 `plugin.json`。
///
/// `with_skill` / `with_tool` 为真时各带一个**开箱即用**的示例条目，
/// 让用户建完立刻能在技能库 / 工具注册表里看到效果（而不是只得到一个空壳）。
pub fn create(
    name: &str,
    version: &str,
    description: &str,
    with_skill: bool,
    with_tool: bool,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("插件名不能为空".into());
    }
    let dir_name = safe_dir_name(name);
    if dir_name.is_empty() {
        return Err("插件名里没有任何可用字符（需含字母或数字）".into());
    }
    let root = plugins_dir();
    let dir = std::path::Path::new(&root).join(&dir_name);
    if dir.exists() {
        return Err(format!("插件目录已存在：{dir_name}"));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("建插件目录失败：{e}"))?;

    let mut skills: Vec<PluginSkill> = Vec::new();
    if with_skill {
        skills.push(PluginSkill {
            name: format!("{dir_name}-demo"),
            desc: "示例技能（可自行改名或删掉）".into(),
            cat: "插件示例".into(),
        });
    }
    let mut tools: Vec<PluginTool> = Vec::new();
    if with_tool {
        tools.push(PluginTool {
            name: format!("{dir_name}-echo"),
            title: "示例工具".into(),
            description: "把参数原样输出（用来看清插件工具是怎么被调用的）".into(),
            command: "Write-Output '插件工具收到参数：{{arg}}'".into(),
        });
    }

    let manifest = PluginManifest {
        enabled: true,
        name: name.to_string(),
        version: if version.trim().is_empty() { "1.0.0".into() } else { version.trim().to_string() },
        description: description.trim().to_string(),
        skills,
        tools,
    };
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| format!("序列化失败：{e}"))?;
    std::fs::write(dir.join("plugin.json"), json).map_err(|e| format!("写入 plugin.json 失败：{e}"))?;
    Ok(dir.to_string_lossy().to_string())
}

/// 删除插件。
///
/// **安全约束**：目标必须是 `plugins_dir` 的**直接子目录**且包含 `plugin.json`。
/// 只删除该插件目录本身，绝不越出插件根目录。
pub fn delete(dir_path: &str) -> Result<(), String> {
    let root = std::path::Path::new(&plugins_dir())
        .canonicalize()
        .map_err(|e| format!("插件根目录不可用：{e}"))?;
    let target = std::path::Path::new(dir_path)
        .canonicalize()
        .map_err(|e| format!("插件目录不存在：{e}"))?;

    let parent = target.parent().ok_or("目标没有父目录")?;
    if parent != root {
        return Err("拒绝删除：目标不是插件根目录的直接子目录".into());
    }
    if !target.join("plugin.json").exists() {
        return Err("拒绝删除：目标目录里没有 plugin.json".into());
    }
    std::fs::remove_dir_all(&target).map_err(|e| format!("删除失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parses_and_tolerates_missing_optional_fields() {
        let raw = r#"{ "name": "p1", "skills": [ { "name": "s1" } ] }"#;
        let m: PluginManifest = serde_json::from_str(raw).expect("应可解析");
        assert_eq!(m.name, "p1");
        assert_eq!(m.version, "");
        assert_eq!(m.skills.len(), 1);
        assert_eq!(m.skills[0].desc, "");
        assert_eq!(m.skills[0].cat, "");
    }

    #[test]
    fn manifest_requires_name() {
        let raw = r#"{ "version": "1.0" }"#;
        assert!(serde_json::from_str::<PluginManifest>(raw).is_err(), "缺 name 应失败");
    }

    #[test]
    fn full_manifest_roundtrip() {
        let raw = r#"{
            "name": "demo-plugin",
            "version": "1.0.0",
            "description": "演示插件",
            "skills": [
                { "name": "demo-a", "desc": "演示 A", "cat": "工具" },
                { "name": "demo-b", "desc": "演示 B", "cat": "写作" }
            ]
        }"#;
        let m: PluginManifest = serde_json::from_str(raw).expect("应可解析");
        assert_eq!(m.version, "1.0.0");
        assert_eq!(m.description, "演示插件");
        assert_eq!(m.skills.len(), 2);
        assert_eq!(m.skills[1].cat, "写作");
        assert!(m.tools.is_empty(), "未声明 tools 时应为空（向后兼容旧清单）");
    }

    #[test]
    fn manifest_with_tools_parses() {
        let raw = r#"{
            "name": "tool-plugin",
            "tools": [
                { "name": "say-hi", "title": "打招呼", "description": "输出一句问候", "command": "Write-Output 'hi {{arg}}'" }
            ]
        }"#;
        let m: PluginManifest = serde_json::from_str(raw).expect("应可解析");
        assert_eq!(m.tools.len(), 1);
        assert_eq!(m.tools[0].name, "say-hi");
        assert_eq!(m.tools[0].title, "打招呼");
        assert!(m.tools[0].command.contains("{{arg}}"));
    }

    #[test]
    fn tool_optional_fields_default_to_empty() {
        let raw = r#"{ "name": "p", "tools": [ { "name": "t", "command": "echo {{arg}}" } ] }"#;
        let m: PluginManifest = serde_json::from_str(raw).expect("应可解析");
        assert_eq!(m.tools[0].title, "");
        assert_eq!(m.tools[0].description, "");
    }

    #[test]
    fn valid_tools_filters_incomplete_entries() {
        let mk = |name: &str, cmd: &str| PluginTool {
            name: name.into(),
            title: String::new(),
            description: String::new(),
            command: cmd.into(),
        };
        let input = vec![
            mk("good", "echo {{arg}}"),
            mk("", "echo x"),        // 名称为空
            mk("no-cmd", ""),        // 命令为空
            mk("   ", "echo x"),     // 名称只有空白
            mk("blank-cmd", "   "),  // 命令只有空白
            mk("good2", "echo y"),
        ];
        let got = valid_tools(&input);
        assert_eq!(got.len(), 2, "应只保留两条完整工具");
        assert_eq!(got[0].name, "good");
        assert_eq!(got[1].name, "good2");
    }

    #[test]
    fn empty_plugin_tools_list_is_fine() {
        assert!(valid_tools(&[]).is_empty());
    }

    // ---------- 目录名安全 / 创建 / 删除 ----------

    #[test]
    fn safe_dir_name_keeps_safe_chars_only() {
        assert_eq!(safe_dir_name("my-plugin"), "my-plugin");
        assert_eq!(safe_dir_name("  My Plugin  "), "My-Plugin");
        assert_eq!(safe_dir_name("a/b\\c:d"), "a-b-c-d");
        assert_eq!(safe_dir_name("插件A1"), "A1"); // 中文转 - 后被 trim
        assert_eq!(safe_dir_name("!!!"), "");
        assert_eq!(safe_dir_name("a--b---c"), "a-b-c");
    }

    #[test]
    fn safe_dir_name_blocks_path_traversal() {
        // ★ 目录名不允许出现分隔符与点点，避免把插件建到插件根目录之外
        let got = safe_dir_name("../../etc");
        assert!(!got.contains('/'), "不得含分隔符：{got}");
        assert!(!got.contains('\\'));
        assert!(!got.contains(".."), "不得含 ..：{got}");
    }

    #[test]
    fn safe_dir_name_rejects_empty_result() {
        assert!(safe_dir_name("").is_empty());
        assert!(safe_dir_name("---").is_empty());
    }

    #[test]
    fn create_rejects_empty_name() {
        assert!(create("", "", "", false, false).is_err());
        assert!(create("   ", "", "", false, false).is_err());
        assert!(create("!!!", "", "", false, false).is_err(), "无可用字符也应拒绝");
    }

    #[test]
    fn delete_refuses_path_outside_plugins_root() {
        // 用临时目录冒充插件根，验证"必须是直接子目录且含 plugin.json"的两道闸
        let root = std::env::temp_dir().join(format!("eagent-pl-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let outside = std::env::temp_dir().join(format!("eagent-pl-out-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("plugin.json"), "{}").unwrap();

        // 直接复刻 delete 的判定（不碰真实 plugins_dir）
        let check = |target: &std::path::Path, root: &std::path::Path| -> bool {
            let r = root.canonicalize().unwrap();
            let t = match target.canonicalize() { Ok(t) => t, Err(_) => return false };
            t.parent() == Some(r.as_path()) && t.join("plugin.json").exists()
        };
        assert!(!check(&outside, &root), "插件根之外的目录必须拒绝");

        let inside = root.join("ok-plugin");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::write(inside.join("plugin.json"), "{}").unwrap();
        assert!(check(&inside, &root), "根内直接子目录且含 plugin.json 应允许");

        let no_manifest = root.join("no-manifest");
        std::fs::create_dir_all(&no_manifest).unwrap();
        assert!(!check(&no_manifest, &root), "缺 plugin.json 应拒绝");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
