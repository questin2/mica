//! 技能库：**数据驱动、可增删**（原为前端硬编码的 26 项，只能看不能加）。
//!
//! 存储：`<data_dir>/skills.json`。首次运行从内置清单播种，之后用户完全掌控。
//! 入口：`list` / `add` / `remove` / `restore_builtin` / `prompt_block`。
//!
//! 为什么要 `prompt_block`：技能若只停留在 UI，模型根本不知道它们存在 ——
//! 会话初始化时把技能清单写进临时目录并注入提示词，技能才真正"可用"。

use serde::{Deserialize, Serialize};

/// 技能条目。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub desc: String,
    pub cat: String,
    /// 许可证来源；空串表示"未知"（UI 会标黄）。
    #[serde(default)]
    pub licensed: String,
    /// 是否来自内置清单（UI 用于区分，删除后可用「恢复内置」找回）。
    #[serde(default)]
    pub builtin: bool,
    /// 来源标注；空串 = 本地技能库，非空如 `插件：xxx`（此类为只读，不能从技能库删除）。
    #[serde(default)]
    pub source: String,
}

/// 内置技能清单（名称来自参考实现字符串转储；用途为按名称推断）。
pub const BUILTIN: &[(&str, &str, &str)] = &[
    ("bad-apple-cli", "终端字符动画演示", "娱乐"),
    ("nx-auto-modeling", "NX 参数化建模辅助", "设计/CAD"),
    ("ai-claim-checker", "事实与声明核查", "研究"),
    ("cue-omni-reader", "多格式材料速读", "阅读"),
    ("daily-standup-journal", "每日站会与工作日志", "效率"),
    ("decision-matrix", "决策矩阵打分", "决策"),
    ("diagram-maker", "结构图/流程图生成", "制图"),
    ("documentation-writer", "工程文档撰写", "写作"),
    ("exam-ready", "备考计划与自测", "学习"),
    ("frontend-design", "前端界面设计建议", "设计"),
    ("ledger-tasks-yylo", "任务台账管理", "效率"),
    ("linkedin-post-formatter", "LinkedIn 贴文格式化", "写作"),
    ("meeting-note-summarizer", "会议纪要摘要", "效率"),
    ("orca-replay", "操作回放与复现", "工具"),
    ("plugin-scanner", "插件/扩展扫描", "安全"),
    ("retrieval-practice-generator", "检索式练习生成", "学习"),
    ("sandbase", "沙箱与数据基座", "工具"),
    ("skillhub-cli", "技能仓库命令行", "工具"),
    ("storytelling-advisor", "叙事结构建议", "写作"),
    ("study-strategy-selector", "学习策略选择", "学习"),
    ("time-blocking-scheduler", "时间块排程", "效率"),
    ("triage-nda", "NDA 条款分诊", "法务"),
    ("video-frames", "视频抽帧", "媒体"),
    ("weather", "天气查询", "生活"),
    ("zero-slop", "去 AI 味润色", "写作"),
    ("zhangxuefeng-skill", "张雪峰风格咨询", "咨询"),
];

/// 已核实许可证的内置技能。
pub const BUILTIN_LICENSED: &[(&str, &str)] =
    &[("skillhub-cli", "LICENSE.txt / NOTICE.md"), ("zhangxuefeng-skill", "LICENSE")];

/// 内置清单（构造 `Skill`，含许可标注）。
pub fn builtin_skills() -> Vec<Skill> {
    BUILTIN
        .iter()
        .map(|(n, d, c)| Skill {
            name: (*n).to_string(),
            desc: (*d).to_string(),
            cat: (*c).to_string(),
            licensed: BUILTIN_LICENSED
                .iter()
                .find(|(ln, _)| ln == n)
                .map(|(_, l)| (*l).to_string())
                .unwrap_or_default(),
            builtin: true,
            source: String::new(),
        })
        .collect()
}

pub fn skills_path() -> String {
    format!(r"{}\skills.json", crate::config::data_dir())
}

// ---- 路径可注入的内部实现（便于隔离测试，不依赖进程级环境变量） ----

fn load_at(path: &str) -> Vec<Skill> {
    match std::fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str::<Vec<Skill>>(&s) {
            Ok(v) if !v.is_empty() => v,
            _ => {
                let seed = builtin_skills();
                let _ = save_at(path, &seed);
                seed
            }
        },
        Err(_) => {
            let seed = builtin_skills();
            let _ = save_at(path, &seed);
            seed
        }
    }
}

fn save_at(path: &str, list: &[Skill]) -> Result<(), String> {
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败: {e}"))?;
    }
    let json = serde_json::to_string_pretty(list).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("写入 skills.json 失败: {e}"))
}

fn add_at(path: &str, name: &str, desc: &str, cat: &str) -> Result<Vec<Skill>, String> {
    let name = name.trim();
    let desc = desc.trim();
    if name.is_empty() {
        return Err("技能名不能为空".into());
    }
    if desc.is_empty() {
        return Err("技能描述不能为空".into());
    }
    let mut list = load_at(path);
    if list.iter().any(|s| s.name.eq_ignore_ascii_case(name)) {
        return Err(format!("技能「{name}」已存在"));
    }
    list.push(Skill {
        name: name.to_string(),
        desc: desc.to_string(),
        cat: if cat.trim().is_empty() { "自定义".to_string() } else { cat.trim().to_string() },
        licensed: String::new(),
        builtin: false,
        source: String::new(),
    });
    save_at(path, &list)?;
    Ok(list)
}

fn remove_at(path: &str, name: &str) -> Result<Vec<Skill>, String> {
    let mut list = load_at(path);
    let before = list.len();
    list.retain(|s| !s.name.eq_ignore_ascii_case(name.trim()));
    if list.len() == before {
        return Err(format!("未找到技能「{name}」"));
    }
    save_at(path, &list)?;
    Ok(list)
}

fn restore_builtin_at(path: &str) -> Result<Vec<Skill>, String> {
    let mut list = load_at(path);
    for b in builtin_skills() {
        if !list.iter().any(|s| s.name.eq_ignore_ascii_case(&b.name)) {
            list.push(b);
        }
    }
    save_at(path, &list)?;
    Ok(list)
}

fn prompt_block_of(list: &[Skill]) -> String {
    if list.is_empty() {
        return "（当前技能库为空）".to_string();
    }
    let mut s = String::new();
    for it in list {
        s.push_str(&format!("- {}｜{}｜分类：{}\n", it.name, it.desc, it.cat));
    }
    s
}

// ---- 对外接口（走默认路径 <data_dir>/skills.json） ----

/// 读取技能库；文件不存在或损坏 → 用内置清单播种并落盘。
pub fn load() -> Vec<Skill> {
    load_at(&skills_path())
}

/// 新增技能（名称去空白、唯一、非空描述）。
pub fn add(name: &str, desc: &str, cat: &str) -> Result<Vec<Skill>, String> {
    add_at(&skills_path(), name, desc, cat)
}

/// 删除技能（按名称，大小写不敏感）。
pub fn remove(name: &str) -> Result<Vec<Skill>, String> {
    remove_at(&skills_path(), name)
}

/// 恢复内置清单（保留用户自建技能，去重后合并）。
pub fn restore_builtin() -> Result<Vec<Skill>, String> {
    restore_builtin_at(&skills_path())
}

/// 生成给模型看的技能清单（会话初始化时写入临时目录 + 注入提示词）。
pub fn prompt_block() -> String {
    prompt_block_of(&load_all())
}

/// **完整技能视图** = 本地技能库 + 插件贡献（插件项标注来源、只读）。
///
/// `load()` 只读本地 `skills.json`；`load_all()` 才对上层可见 ——
/// 这样增删只动本地库，插件技能不会被误删。
pub fn load_all() -> Vec<Skill> {
    let mut list = load();
    for (plugin, ps) in crate::plugins::contributed_skills() {
        if ps.name.trim().is_empty() {
            continue;
        }
        // 同名以本地技能为准（用户可显式覆盖插件提供的技能）
        if list.iter().any(|s| s.name.eq_ignore_ascii_case(ps.name.trim())) {
            continue;
        }
        list.push(Skill {
            name: ps.name.trim().to_string(),
            desc: ps.desc.trim().to_string(),
            cat: if ps.cat.trim().is_empty() { "插件".to_string() } else { ps.cat.trim().to_string() },
            licensed: String::new(),
            builtin: false,
            source: format!("插件：{plugin}"),
        });
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_seed_is_wellformed() {
        let b = builtin_skills();
        assert_eq!(b.len(), BUILTIN.len());
        assert!(b.iter().all(|s| !s.name.is_empty() && !s.desc.is_empty() && !s.cat.is_empty()));
        assert!(b.iter().all(|s| s.builtin));
        // 许可标注只落在已核实的两个技能上
        let licensed: Vec<&str> =
            b.iter().filter(|s| !s.licensed.is_empty()).map(|s| s.name.as_str()).collect();
        assert_eq!(licensed.len(), BUILTIN_LICENSED.len());
    }

    #[test]
    fn builtin_names_are_unique() {
        let b = builtin_skills();
        let mut names: Vec<&str> = b.iter().map(|s| s.name.as_str()).collect();
        names.sort_unstable();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n, "内置技能名应唯一");
    }

    /// 增删改查 + 落盘 + 提示词注入的完整往返（用临时文件，不依赖环境变量）。
    #[test]
    fn add_remove_restore_and_persist_roundtrip() {
        let dir = std::env::temp_dir().join(format!("eagent-skills-test-{}", std::process::id()));
        let p = dir.join("skills.json").to_string_lossy().to_string();
        let _ = std::fs::remove_dir_all(&dir);

        // 首次读取 → 播种内置清单
        let seed = load_at(&p);
        assert_eq!(seed.len(), BUILTIN.len(), "首次应播种全部内置技能");

        // 新增（分类留空 → 归入「自定义」）
        let after = add_at(&p, "  my-skill  ", "我的自定义技能", "").expect("add 应成功");
        assert_eq!(after.len(), BUILTIN.len() + 1);
        let mine = after.iter().find(|s| s.name == "my-skill").expect("应写入且名称已 trim");
        assert!(!mine.builtin, "自建项不应标为内置");
        assert_eq!(mine.cat, "自定义");
        assert!(mine.licensed.is_empty());

        // 重名（大小写不敏感）必须被拒
        assert!(add_at(&p, "MY-SKILL", "dup", "x").is_err(), "重名应拒绝");
        // 空名 / 空描述必须被拒
        assert!(add_at(&p, "", "d", "").is_err());
        assert!(add_at(&p, "n", "   ", "").is_err());

        // 重新读盘 → 自建项确实持久化了
        let reread = load_at(&p);
        assert_eq!(reread.len(), BUILTIN.len() + 1);
        assert!(reread.iter().any(|s| s.name == "my-skill"));

        // 提示词块：行数 = 技能数，且含新增项
        let block = prompt_block_of(&reread);
        assert_eq!(block.lines().count(), BUILTIN.len() + 1);
        assert!(block.contains("my-skill"));

        // 删除
        let gone = remove_at(&p, "my-skill").expect("remove 应成功");
        assert_eq!(gone.len(), BUILTIN.len());
        assert!(remove_at(&p, "my-skill").is_err(), "重复删除应报错");

        // 恢复内置：已全部存在 → 数量不变（幂等）
        let restored = restore_builtin_at(&p).expect("restore");
        assert_eq!(restored.len(), BUILTIN.len());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 损坏的 skills.json 不应让程序崩，而是回退为播种内置清单。
    #[test]
    fn corrupt_file_falls_back_to_seed() {
        let dir = std::env::temp_dir().join(format!("eagent-skills-bad-{}", std::process::id()));
        let p = dir.join("skills.json").to_string_lossy().to_string();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, "{ 这不是合法 JSON").unwrap();

        let list = load_at(&p);
        assert_eq!(list.len(), BUILTIN.len(), "损坏文件应回退播种");
        // 且已把文件修复为合法 JSON
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(serde_json::from_str::<Vec<Skill>>(&s).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
