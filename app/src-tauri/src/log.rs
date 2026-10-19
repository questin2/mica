//! 日志与时间戳（chain / audit / 控制台回显）。
//!
//! 拆分自原单文件 `lib.rs`（可扩展性分析报告 v2 · S1）。
//! `use super::*` 继承根模块的导入与共享项，避免重复声明。

use super::*;

pub(crate) static SEQ: AtomicU64 = AtomicU64::new(1);

pub(crate) fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub(crate) fn append_log(path: &str, msg: &str) {
    let _ = std::fs::create_dir_all(data_dir());
    let line = format!("[{}] {msg}\n", now_ms());
    print!("LOG {line}");
    let _ = std::io::stdout().flush();
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

pub(crate) fn chain_log(msg: &str) {
    append_log(&chain_log_path(), msg);
}

pub(crate) fn audit_log(msg: &str) {
    append_log(&audit_log_path(), msg);
    // 【Phase 3】并行落库（文件审计仍是权威副本；数据库未初始化时静默跳过）。
    if let Some(db) = crate::memory::try_db() {
        let _ = db.append_audit("audit", msg);
    }
}

/// 读文件**尾部**：先按字节截取尾部，再取最后 `max_lines` 行。
///
/// 抽成纯文本函数是为了可测；`read_audit_tail` 只是它 + 磁盘读取。
/// 设计取舍：审计日志会无限增长，**绝不整文件读入内存**；只回传尾部窗口。
pub(crate) fn tail_lines(text: &str, max_bytes: usize, max_lines: usize) -> String {
    // 先按字节截尾（注意不能切坏 UTF-8：从合法边界起切）
    let mut start = text.len().saturating_sub(max_bytes);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    let window = &text[start..];

    let lines: Vec<&str> = window.split_inclusive('\n').collect();
    let tail = if lines.len() > max_lines {
        &lines[lines.len() - max_lines..]
    } else {
        &lines[..]
    };
    let mut s = tail.concat();
    // 若确实截断过，明确告知（不静默丢内容）
    if start > 0 || lines.len() > max_lines {
        s = format!("[仅显示日志尾部：最多 {max_lines} 行 / {max_bytes} 字节]\n{s}");
    }
    s
}

/// 人类可读的字节数（纯函数，便于测试）。
///
/// 用途：审计日志**无自动清理**，需要让用户看得见它在长 ——
/// 但"要不要自动清理旧审计"是会影响审计完整性的**产品决定**，不在此处擅自实现。
pub(crate) fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let f = n as f64;
    if f >= MB {
        format!("{:.2} MB", f / MB)
    } else if f >= KB {
        format!("{:.1} KB", f / KB)
    } else {
        format!("{n} B")
    }
}

/// 审计日志当前大小（字节）；文件不存在返回 0。
pub(crate) fn audit_log_size() -> u64 {
    std::fs::metadata(audit_log_path()).map(|m| m.len()).unwrap_or(0)
}

/// 审计日志大小的人类可读描述（供 UI 展示，提示"这个文件不会自动清理"）。
pub(crate) fn audit_log_size_human() -> String {
    format_bytes(audit_log_size())
}

/// 丢弃首个**不完整行**（从文件中间起读时，第一行往往是被切掉一半的）。
pub(crate) fn trim_leading_partial_line(s: &str) -> String {
    match s.find('\n') {
        Some(i) => s[i + 1..].to_string(),
        None => String::new(), // 整段都没有换行 → 说明它本身就是半行
    }
}

/// 读取磁盘审计日志的尾部（供 UI 展示审批来源留痕）。
///
/// **从文件末尾 seek 读取**，不把整个文件读进内存 —— 审计日志无上限增长，
/// 实测用户环境已 160KB+，整读在大日志上会越来越慢。
pub(crate) fn read_audit_tail(max_bytes: usize, max_lines: usize) -> String {
    use std::io::{Seek, SeekFrom};

    let path = audit_log_path();
    let Ok(mut f) = std::fs::File::open(&path) else {
        return String::new();
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return String::new();
    };
    if len == 0 {
        return String::new();
    }

    let start = len.saturating_sub(max_bytes as u64);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return String::new();
    }

    // 从中间切开可能落在 UTF-8 字符内部，也可能切出半行 → 用 lossy 兜底 + 丢掉首个半行
    let s = String::from_utf8_lossy(&buf).to_string();
    let s = if start > 0 { trim_leading_partial_line(&s) } else { s };
    tail_lines(&s, max_bytes, max_lines)
}

/// 审计正文上限（字符数）。前端可能误传巨量内容，这里兜一层。
pub(crate) const AUDIT_TEXT_CAP: usize = 1000;

/// 规整前端回报的审计正文：去首尾空白 + 长度截断。
///
/// 抽成纯函数是为了**可测**：它是"外部输入进审计日志"的唯一关口。
/// 空内容一律拒绝（不往审计里写空行）。
pub(crate) fn prepare_audit_text(text: &str) -> Result<String, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("审计内容为空".into());
    }
    let n = t.chars().count();
    if n > AUDIT_TEXT_CAP {
        let body: String = t.chars().take(AUDIT_TEXT_CAP).collect();
        return Ok(format!("{body}\n[审计内容已截断：原 {n} 字符]"));
    }
    Ok(t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_text_rejects_empty_and_whitespace() {
        assert!(prepare_audit_text("").is_err());
        assert!(prepare_audit_text("   \n\t ").is_err());
    }

    #[test]
    fn audit_text_trims_surrounding_whitespace() {
        assert_eq!(prepare_audit_text("  hello  \n").unwrap(), "hello");
    }

    #[test]
    fn audit_text_keeps_content_at_exactly_cap() {
        let s = "x".repeat(AUDIT_TEXT_CAP);
        let out = prepare_audit_text(&s).unwrap();
        assert_eq!(out.chars().count(), AUDIT_TEXT_CAP);
        assert!(!out.contains("已截断"), "恰好等于上限不应报截断");
    }

    #[test]
    fn audit_text_truncates_beyond_cap_and_says_so() {
        let s = "x".repeat(AUDIT_TEXT_CAP + 50);
        let out = prepare_audit_text(&s).unwrap();
        assert!(out.starts_with(&"x".repeat(10)));
        assert!(out.contains("审计内容已截断"), "超限必须显式标注，不能静默丢内容");
        assert!(out.contains(&format!("原 {} 字符", AUDIT_TEXT_CAP + 50)));
    }

    #[test]
    fn audit_text_counts_chars_not_bytes() {
        // 中文按字符计：1000 个汉字不应被当成 3000 字节截断
        let s = "中".repeat(AUDIT_TEXT_CAP);
        let out = prepare_audit_text(&s).unwrap();
        assert_eq!(out.chars().count(), AUDIT_TEXT_CAP);
        assert!(!out.contains("已截断"));
    }

    #[test]
    fn audit_text_preserves_newlines_in_provenance_lines() {
        let t = "审批来源：会话信任自动放行 #7（类别 只读）\n说明：非人工逐条确认。";
        assert_eq!(prepare_audit_text(t).unwrap(), t);
    }

    // ---------- 尾部读取（供「审计」页展示留痕） ----------

    #[test]
    fn tail_returns_everything_when_under_limits() {
        let t = "line1\nline2\nline3\n";
        let out = tail_lines(t, 10_000, 100);
        assert_eq!(out, t, "未超限时不应加任何前缀、也不应丢内容");
    }

    #[test]
    fn tail_keeps_only_last_lines_and_says_so() {
        let t: String = (1..=100).map(|i| format!("L{i}\n")).collect();
        let out = tail_lines(&t, 1_000_000, 10);
        assert!(out.contains("仅显示日志尾部"), "截断必须显式告知，不能静默丢");
        assert!(out.contains("L100\n"), "应保留最后一行");
        assert!(out.contains("L91\n"), "应保留最后 10 行");
        assert!(!out.contains("L90\n"), "第 11 行应被丢弃");
    }

    #[test]
    fn tail_respects_byte_budget() {
        let t: String = (1..=1000).map(|i| format!("line-{i}\n")).collect();
        let out = tail_lines(&t, 200, 10_000);
        // 200 字节窗口内最多 10 行；前缀另算
        assert!(out.chars().count() < 400, "尾部窗口应按字节受控");
        assert!(out.contains("仅显示日志尾部"));
    }

    #[test]
    fn tail_never_splits_a_utf8_char() {
        // 全中文：按字节截尾时若切在字符中间会产生非法 UTF-8（Rust 的 &str 切片会 panic）
        let t: String = (1..=200).map(|i| format!("第{i}行：中文内容\n")).collect();
        let out = tail_lines(&t, 37, 5_000); // 故意用非对齐的字节数
        assert!(out.contains("仅显示日志尾部"));
        assert!(!out.is_empty());
    }

    #[test]
    fn tail_of_empty_text_is_empty() {
        assert_eq!(tail_lines("", 100, 10), "");
    }

    #[test]
    fn tail_handles_text_without_trailing_newline() {
        let out = tail_lines("a\nb\nc", 10_000, 10);
        assert_eq!(out, "a\nb\nc");
    }

    #[test]
    fn trim_partial_line_drops_only_the_first_incomplete_line() {
        let s = "半行被切断了\n完整行1\n完整行2\n";
        assert_eq!(trim_leading_partial_line(s), "完整行1\n完整行2\n");
    }

    #[test]
    fn trim_partial_line_without_any_newline_yields_empty() {
        // 整段没有换行 → 它本身就是半行，应当整个丢掉
        assert_eq!(trim_leading_partial_line("没有换行的半行"), "");
    }

    #[test]
    fn trim_partial_line_on_empty_is_empty() {
        assert_eq!(trim_leading_partial_line(""), "");
    }

    #[test]
    fn format_bytes_scales_and_rounds() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(166_471), "162.6 KB"); // 实测用户环境的量级
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 + 512 * 1024), "3.50 MB");
    }

    #[test]
    fn format_bytes_boundary_kb_to_mb() {
        // 刚好差 1 字节仍是 KB，跨过去才变 MB
        assert!(format_bytes(1024 * 1024 - 1).ends_with("KB"));
        assert!(format_bytes(1024 * 1024).ends_with("MB"));
    }

    /// 端到端读尾：写一个真实临时文件，验证「从末尾 seek」读到的内容正确。
    #[test]
    fn read_tail_from_file_end_matches_whole_file_tail() {
        use std::io::Write as _;
        let d = std::env::temp_dir().join(format!("eagent-tail-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("audit.log");
        let all: String = (1..=500).map(|i| format!("行{i}: 中文内容\n")).collect();
        std::fs::File::create(&p).unwrap().write_all(all.as_bytes()).unwrap();

        // 复刻 read_audit_tail 的读法（不触碰真实 data_dir）
        let read_tail = |max_bytes: usize, max_lines: usize| -> String {
            use std::io::{Seek, SeekFrom};
            let mut f = std::fs::File::open(&p).unwrap();
            let len = f.metadata().unwrap().len();
            let start = len.saturating_sub(max_bytes as u64);
            f.seek(SeekFrom::Start(start)).unwrap();
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            let s = String::from_utf8_lossy(&buf).to_string();
            let s = if start > 0 { trim_leading_partial_line(&s) } else { s };
            tail_lines(&s, max_bytes, max_lines)
        };

        // 全量读（起点为 0，不该丢首行）
        let full = read_tail(10 * 1024 * 1024, 1000);
        assert!(full.starts_with("行1: 中文内容"), "起点为文件头时不得丢首行");
        assert!(full.contains("行500: 中文内容"));

        // 只取尾部：最后一行的内容必须还在
        let small = read_tail(200, 5);
        assert!(small.contains("行500: 中文内容"), "尾部读取必须包含最后一行");
        assert!(!small.contains("行1: 中文内容"));
        // 且不得出现替换字符（说明没把 UTF-8 字符切坏并留下来）
        assert!(!small.contains('\u{FFFD}'), "不应残留 UTF-8 替换字符：{small:?}");

        let _ = std::fs::remove_dir_all(&d);
    }
}
