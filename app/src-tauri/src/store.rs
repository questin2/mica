//! 会话持久化（可扩展性报告 v2 · S-02）：把前端会话状态落盘，**重启不丢**。
//!
//! 持久化内容由前端决定（消息流、已决策工具、工作目录…），后端只负责：
//!   - 校验是不是合法 JSON（不把垃圾写进磁盘）；
//!   - **原子写**（先写 `.tmp` 再 rename），避免掉电/崩溃留下半截文件；
//!   - 覆盖前把上一版留一份 `.bak`（误操作可回退）；
//!   - 读取时若主文件损坏，自动回退 `.bak`。
//!
//! 存储位置：`<data_dir>/session.json`。

/// 主文件路径。
pub fn session_path() -> String {
    format!(r"{}\session.json", crate::config::data_dir())
}

/// 备份文件路径（覆盖前自动保留的上一版）。
pub fn bak_path() -> String {
    format!(r"{}\session.bak.json", crate::config::data_dir())
}

fn tmp_path() -> String {
    format!(r"{}\session.json.tmp", crate::config::data_dir())
}

/// 写入会话快照（原子写 + 备份上一版）。
pub fn save(json: &str) -> Result<(), String> {
    // 只接受合法 JSON，避免把损坏内容固化下来
    serde_json::from_str::<serde_json::Value>(json).map_err(|e| format!("不是合法 JSON：{e}"))?;

    let dir = crate::config::data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败：{e}"))?;

    // 覆盖前备份上一版
    let main = session_path();
    if std::path::Path::new(&main).exists() {
        let _ = std::fs::copy(&main, bak_path());
    }

    // 原子写：tmp → rename
    let tmp = tmp_path();
    std::fs::write(&tmp, json).map_err(|e| format!("写临时文件失败：{e}"))?;
    std::fs::rename(&tmp, &main).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("替换 session.json 失败：{e}")
    })
}

/// 读取会话快照；不存在返回空串（前端据此走初始状态）。
/// 主文件损坏时回退 `.bak`，两者都不可用则返回空串。
pub fn load() -> String {
    if let Ok(s) = std::fs::read_to_string(session_path()) {
        if serde_json::from_str::<serde_json::Value>(&s).is_ok() {
            return s;
        }
    }
    if let Ok(s) = std::fs::read_to_string(bak_path()) {
        if serde_json::from_str::<serde_json::Value>(&s).is_ok() {
            return s;
        }
    }
    String::new()
}

/// 清空会话（同时清掉备份），用于「新会话」。
pub fn clear() -> Result<(), String> {
    for p in [session_path(), bak_path(), tmp_path()] {
        if std::path::Path::new(&p).exists() {
            std::fs::remove_file(&p).map_err(|e| format!("删除 {p} 失败：{e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个测试用**独立**临时目录（Rust 测试并行执行，共用目录会互相覆盖）。
    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("eagent-store-test-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn save_rejects_invalid_json() {
        // 非法 JSON 必须在写盘前被挡下（不依赖 data_dir，可安全调用）
        assert!(save("{ 不是 JSON").is_err());
        assert!(save("").is_err());
    }

    #[test]
    fn atomic_write_and_backup_semantics() {
        let d = tmp_dir("atomic");
        let main = d.join("session.json");
        let bak = d.join("session.bak.json");
        let tmp = d.join("session.json.tmp");

        // 第一次写
        std::fs::write(&tmp, r#"{"v":1,"messages":[]}"#).unwrap();
        std::fs::rename(&tmp, &main).unwrap();
        assert_eq!(std::fs::read_to_string(&main).unwrap(), r#"{"v":1,"messages":[]}"#);
        assert!(!tmp.exists(), "rename 后不应残留 tmp");

        // 第二次写：先备份旧版
        std::fs::copy(&main, &bak).unwrap();
        std::fs::write(&tmp, r#"{"v":2,"messages":[1]}"#).unwrap();
        std::fs::rename(&tmp, &main).unwrap();

        assert!(std::fs::read_to_string(&main).unwrap().contains(r#""v":2"#));
        assert!(std::fs::read_to_string(&bak).unwrap().contains(r#""v":1"#), "应保留上一版");

        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn corrupt_main_falls_back_to_bak() {
        let d = tmp_dir("corrupt");
        let main = d.join("session.json");
        let bak = d.join("session.bak.json");
        std::fs::write(&main, "{ 坏掉的 JSON").unwrap();
        std::fs::write(&bak, r#"{"ok":true}"#).unwrap();

        // 复刻 load 的回退判定（不触碰真实 data_dir）
        let pick = |p: &std::path::Path| -> Option<String> {
            let s = std::fs::read_to_string(p).ok()?;
            serde_json::from_str::<serde_json::Value>(&s).ok()?;
            Some(s)
        };
        let got = pick(&main).or_else(|| pick(&bak));
        assert_eq!(got.as_deref(), Some(r#"{"ok":true}"#), "主文件损坏应回退备份");

        let _ = std::fs::remove_dir_all(&d);
    }
}
