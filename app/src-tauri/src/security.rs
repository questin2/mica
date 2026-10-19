//! 安全辅助：敏感路径启发式识别 + MCP 子进程环境白名单。
//!
//! 注意：这里的路径识别是**兜底**，不是边界。`'conf'+'ig'`、`[IO.File]::WriteAllText`
//! 这类混淆可以绕过它；真正的第一道防线是前端的只读/自动放行白名单（format.ts）。

use std::path::Path;

/// 配置/密钥相关路径片段。输入会先做小写 + 分隔符归一化（`/` → `\`）。
const SENSITIVE_NAMES: &[&str] = &[
    "agent.json",
    "approval.json",
    "mcp.json",
    "provider.json",
    "prompt.initial.md",
    "selectors.json",
];

const SENSITIVE_ENV: &[&str] = &[
    "eagent_config_dir",
    "eagent_data_dir",
    "eagent_secrets_dir",
];

const SENSITIVE_PREFIXES: &[&str] = &[
    "appdata\\eagent",
    "appdata\\roaming\\eagent",
    "appdata\\local\\eagent",
    "localappdata\\eagent",
];

/// 归一化：小写 + 统一分隔符。Windows 路径大小写不敏感，`/` 与 `\` 等价。
fn normalize(s: &str) -> String {
    s.to_ascii_lowercase().replace('/', "\\")
}

/// 判断 `text` 里是否出现名为 `name` 的**路径组件**（`config` / `secrets`）。
///
/// 不能用 `contains("config")` —— 那会把 `configuration.txt`、`myconfig` 也算命中。
/// 组件边界：前面是行首/分隔符/引号/空白；后面是行尾/分隔符/引号/空白/冒号/分号/管道。
fn contains_component(text: &str, name: &str) -> bool {
    let bytes = text.as_bytes();
    let mut start = 0;
    while let Some(pos) = text[start..].find(name) {
        let abs = start + pos;
        let before_ok = abs == 0
            || matches!(
                bytes[abs - 1],
                b'\\' | b'/' | b'"' | b'\'' | b' ' | b'\t' | b'\r' | b'\n' | b';' | b'|' | b'&'
            );
        let after = abs + name.len();
        let after_ok = after == bytes.len()
            || matches!(
                bytes[after],
                b'\\' | b'/' | b'"' | b'\'' | b' ' | b'\t' | b'\r' | b'\n' | b';' | b'|' | b'&' | b':'
            );
        if before_ok && after_ok {
            return true;
        }
        start = abs + name.len();
    }
    false
}

/// 真实配置/数据/缓存目录（只取绝对路径，避免把 `config` 这种相对项变成过宽的匹配）。
fn real_dirs() -> Vec<String> {
    let mut out = Vec::new();
    for d in crate::config::config_dirs() {
        if d.is_absolute() {
            out.push(d.to_string_lossy().to_string());
        }
    }
    for d in [crate::config::data_dir(), crate::config::cache_dir()] {
        let p = Path::new(&d);
        if p.is_absolute() {
            out.push(d);
        }
    }
    out
}

/// 命中则返回一个简短原因（用于审批卡警告行）；未命中返回 `None`。
pub fn sensitive_path_hit(text: &str) -> Option<&'static str> {
    let lower = normalize(text);

    if contains_component(&lower, "config") {
        return Some("配置目录");
    }
    if contains_component(&lower, "secrets") {
        return Some("密钥目录");
    }
    for name in SENSITIVE_NAMES {
        if lower.contains(name) {
            return Some("配置文件");
        }
    }
    for name in SENSITIVE_ENV {
        if lower.contains(name) {
            return Some("Mica 环境变量");
        }
    }
    for prefix in SENSITIVE_PREFIXES {
        if lower.contains(prefix) {
            return Some("Mica 数据目录");
        }
    }
    for d in real_dirs() {
        let nd = normalize(&d);
        if nd.len() > 3 && lower.contains(&nd) {
            return Some("Mica 目录");
        }
    }
    None
}

// ---------- MCP 子进程环境 ----------

/// MCP 子进程允许继承的环境变量。缺 `ComSpec`/`PATHEXT`/`PATH` 会让
/// `npx.cmd → cmd.exe → node` 这条链直接断掉。
const ENV_ALLOW: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "HOME",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
];

/// 合并父进程环境与 server 配置环境：
/// 1. 只保留白名单里的父变量（大小写不敏感、去重）；
/// 2. `cfg_env` 覆盖同名项（用户显式配置优先）；
/// 3. 默认注入 `npm_config_ignore_scripts=true`（用户显式覆盖时尊重用户）。
pub fn filtered_env(
    parent: &[(String, String)],
    cfg_env: &[(String, String)],
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (k, v) in parent {
        if ENV_ALLOW.iter().any(|a| a.eq_ignore_ascii_case(k))
            && !out.iter().any(|(ok, _)| ok.eq_ignore_ascii_case(k))
        {
            out.push((k.clone(), v.clone()));
        }
    }
    for (k, v) in cfg_env {
        out.retain(|(ok, _)| !ok.eq_ignore_ascii_case(k));
        out.push((k.clone(), v.clone()));
    }
    if !out
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("npm_config_ignore_scripts"))
    {
        out.push(("npm_config_ignore_scripts".to_string(), "true".to_string()));
    }
    out
}

/// 从当前进程收集父环境（测试之外的生产路径用）。
/// 用 `vars_os` 而不是 `vars`：后者遇到非 Unicode 环境变量会 panic。
pub fn filtered_current_env(cfg_env: &[(String, String)]) -> Vec<(String, String)> {
    let parent: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    filtered_env(&parent, cfg_env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_config_and_secrets_paths() {
        let cases = [
            r"Get-Content x | Set-Content D:\Mica\config\agent.json",
            r"Get-Content x > config\agent.json",
            r"Get-ChildItem D:\Mica\secrets",
            r"Set-Content C:/Mica/config/mcp.json x",
            r"Get-ChildItem C:\Users\demo\AppData\Roaming\Mica",
            r"$env:EAGENT_CONFIG_DIR",
            r"Get-Content C:\project\config\settings.json",
        ];
        for c in cases {
            assert!(sensitive_path_hit(c).is_some(), "应命中：{c}");
        }
    }

    #[test]
    fn normal_commands_do_not_hit() {
        let cases = [
            r"Get-ChildItem C:\Users\demo\Desktop",
            r"Get-Content notes.txt",
            r"Get-Process | Sort-Object CPU",
            r"Get-ChildItem D:\project\configuration.txt",
            r"Get-Date",
        ];
        for c in cases {
            assert!(sensitive_path_hit(c).is_none(), "不应命中：{c}");
        }
    }

    #[test]
    fn filtered_env_keeps_allowlist_and_drops_tokens() {
        let parent = vec![
            ("PATH".to_string(), r"C:\Windows\System32".to_string()),
            ("SystemRoot".to_string(), r"C:\Windows".to_string()),
            ("ComSpec".to_string(), r"C:\Windows\system32\cmd.exe".to_string()),
            ("PATHEXT".to_string(), ".COM;.EXE".to_string()),
            ("GITHUB_TOKEN".to_string(), "secret".to_string()),
            ("OPENAI_API_KEY".to_string(), "secret".to_string()),
            ("NODE_OPTIONS".to_string(), "--require evil".to_string()),
            ("EAGENT_CONFIG_DIR".to_string(), r"D:\Mica\config".to_string()),
        ];
        let cfg = vec![("npm_config_cache".to_string(), r"D:\cache".to_string())];
        let out = filtered_env(&parent, &cfg);
        let get = |k: &str| {
            out.iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(k))
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("PATH"), Some(r"C:\Windows\System32"));
        assert_eq!(get("SystemRoot"), Some(r"C:\Windows"));
        assert_eq!(get("ComSpec"), Some(r"C:\Windows\system32\cmd.exe"));
        assert_eq!(get("npm_config_cache"), Some(r"D:\cache"));
        assert_eq!(get("npm_config_ignore_scripts"), Some("true"));
        assert!(get("GITHUB_TOKEN").is_none());
        assert!(get("OPENAI_API_KEY").is_none());
        assert!(get("NODE_OPTIONS").is_none());
        assert!(get("EAGENT_CONFIG_DIR").is_none());
    }

    #[test]
    fn filtered_env_cfg_overrides_parent() {
        let parent = vec![("PATH".to_string(), "old".to_string())];
        let cfg = vec![("PATH".to_string(), "new".to_string())];
        let out = filtered_env(&parent, &cfg);
        assert_eq!(out.iter().filter(|(k, _)| k == "PATH").count(), 1);
        assert_eq!(
            out.iter().find(|(k, _)| k == "PATH").map(|(_, v)| v.as_str()),
            Some("new")
        );
    }
}
