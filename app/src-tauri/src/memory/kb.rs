//! 知识库：文档抽取、分块、BM25 检索（策划案 2.0 · Phase 3.4）。
//!
//! 选型说明（与 v1.0 的偏差）：原案优先接 `repo-rag-mcp`，但该 crate 的真实性/
//! Windows 可用性/模型体积都未验证；本实现先落地**零外部进程、零模型下载**的
//! BM25 + 中文 bigram 方案，保证离线可用。后续若接入 RAG MCP，只需替换检索来源，
//! 注入接口不变（`context_block`）。

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use super::db::KbChunkRow;

/// 从文件抽取纯文本。支持 txt/md/csv/json/log/代码、docx、pdf、html。
pub fn extract_text(path: &Path) -> Result<String, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "docx" => extract_docx(path),
        "pdf" => {
            pdf_extract::extract_text(path).map_err(|e| format!("解析 PDF 失败：{e}"))
        }
        "html" | "htm" => {
            let raw = read_text_file(path)?;
            Ok(strip_html(&raw))
        }
        _ => read_text_file(path),
    }
}

fn read_text_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败（{}）：{e}", path.display()))?;
    // UTF-8 BOM
    let bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { &bytes[3..] } else { &bytes[..] };
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok(s.to_string());
    }
    let (cow, _, _) = encoding_rs::GBK.decode(bytes);
    Ok(cow.into_owned())
}

fn extract_docx(path: &Path) -> Result<String, String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let file = std::fs::File::open(path).map_err(|e| format!("打开 docx 失败：{e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("docx 不是合法 zip：{e}"))?;
    let mut xml = String::new();
    {
        let mut doc = zip
            .by_name("word/document.xml")
            .map_err(|e| format!("docx 缺少 word/document.xml：{e}"))?;
        doc.read_to_string(&mut xml)
            .map_err(|e| format!("读取 docx 正文失败：{e}"))?;
    }
    let mut reader = Reader::from_str(&xml);
    let mut out = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"w:p" => out.push('\n'),
            Ok(Event::End(e)) if e.name().as_ref() == b"w:p" => out.push('\n'),
            Ok(Event::Text(e)) => {
                if let Ok(t) = e.unescape() {
                    out.push_str(&t);
                }
            }
            Ok(Event::CData(e)) => {
                out.push_str(&String::from_utf8_lossy(e.as_ref()));
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("解析 docx XML 失败：{e}")),
            _ => {}
        }
    }
    Ok(out)
}

/// 极简 HTML → 文本：去 script/style，块级标签转换行，其余标签剥掉。
pub fn strip_html(html: &str) -> String {
    let mut s = html.to_string();
    for (open, close) in [
        ("<script", "</script>"),
        ("<style", "</style>"),
        ("<noscript", "</noscript>"),
    ] {
        loop {
            let lower = s.to_ascii_lowercase();
            let Some(a) = lower.find(open) else { break };
            let Some(b) = lower[a..].find(close) else { break };
            let end = a + b + close.len();
            s.replace_range(a..end, " ");
        }
    }
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in s.chars() {
        if ch == '<' {
            in_tag = true;
            tag.clear();
        } else if ch == '>' {
            in_tag = false;
            let t = tag.trim().to_ascii_lowercase();
            if t.starts_with("br")
                || t.starts_with("/p")
                || t.starts_with("/div")
                || t.starts_with("/li")
                || t.starts_with("/h")
                || t.starts_with("/tr")
            {
                out.push('\n');
            }
        } else if in_tag {
            tag.push(ch);
        } else {
            out.push(ch);
        }
    }
    // 实体（够用即可；不引入完整 HTML 解析器）
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 分块：按段落累积到 target 字符；超长段落按窗口切分（带 overlap）。
pub fn chunk_text(text: &str, target: usize, overlap: usize) -> Vec<String> {
    let target = target.max(100);
    let overlap = overlap.min(target / 2);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let paras: Vec<&str> = text
        .split('\n')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();

    for p in paras {
        let plen = p.chars().count();
        if plen > target {
            if !cur.trim().is_empty() {
                chunks.push(cur.trim().to_string());
                cur.clear();
            }
            let chars: Vec<char> = p.chars().collect();
            let mut start = 0;
            while start < chars.len() {
                let end = (start + target).min(chars.len());
                chunks.push(chars[start..end].iter().collect());
                if end == chars.len() {
                    break;
                }
                start = end.saturating_sub(overlap);
                if start == 0 && end == 0 {
                    break;
                }
            }
            continue;
        }
        if cur.chars().count() + plen + 1 > target && !cur.trim().is_empty() {
            chunks.push(cur.trim().to_string());
            cur.clear();
        }
        cur.push_str(p);
        cur.push('\n');
    }
    if !cur.trim().is_empty() {
        chunks.push(cur.trim().to_string());
    }
    chunks
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF
    )
}

/// 分词：ASCII 字母数字 → 词；CJK → 单字 + 相邻 bigram（无需词典，召回优先）。
pub fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut ascii = String::new();
    let mut prev_cjk: Option<char> = None;
    let flush_ascii = |buf: &mut String, out: &mut Vec<String>| {
        if buf.len() >= 2 {
            out.push(buf.clone());
        } else if buf.len() == 1 {
            out.push(buf.clone());
        }
        buf.clear();
    };
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            ascii.push(ch.to_ascii_lowercase());
            prev_cjk = None;
        } else {
            flush_ascii(&mut ascii, &mut out);
            if is_cjk(ch) {
                out.push(ch.to_string());
                if let Some(p) = prev_cjk {
                    out.push(format!("{p}{ch}"));
                }
                prev_cjk = Some(ch);
            } else {
                prev_cjk = None;
            }
        }
    }
    flush_ascii(&mut ascii, &mut out);
    out
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct KbHit {
    pub doc_id: i64,
    pub ord: i64,
    pub title: String,
    pub path: String,
    pub text: String,
    pub score: f64,
}

/// BM25 检索（k1=1.2, b=0.75）。
pub fn search(chunks: &[KbChunkRow], query: &str, limit: usize) -> Vec<KbHit> {
    let mut q: Vec<String> = tokenize(query);
    q.sort();
    q.dedup();
    if q.is_empty() || chunks.is_empty() {
        return Vec::new();
    }
    let docs: Vec<Vec<String>> = chunks.iter().map(|c| tokenize(&c.text)).collect();
    let n = chunks.len() as f64;
    let avgdl = docs.iter().map(|d| d.len() as f64).sum::<f64>() / n;
    let mut df: HashMap<&str, usize> = HashMap::new();
    for term in &q {
        let count = docs.iter().filter(|d| d.iter().any(|t| t == term)).count();
        df.insert(term, count);
    }
    const K1: f64 = 1.2;
    const B: f64 = 0.75;
    let mut hits = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        let dl = doc.len() as f64;
        let mut tf: HashMap<&str, usize> = HashMap::new();
        for t in doc {
            *tf.entry(t.as_str()).or_insert(0) += 1;
        }
        let mut score = 0.0;
        for term in &q {
            let f = *tf.get(term.as_str()).unwrap_or(&0) as f64;
            if f == 0.0 {
                continue;
            }
            let dfi = *df.get(term.as_str()).unwrap_or(&0) as f64;
            let idf = (1.0 + (n - dfi + 0.5) / (dfi + 0.5)).ln();
            score += idf * (f * (K1 + 1.0)) / (f + K1 * (1.0 - B + B * dl / avgdl.max(1.0)));
        }
        if score > 0.0 {
            let c = &chunks[i];
            hits.push(KbHit {
                doc_id: c.doc_id,
                ord: c.ord,
                title: c.title.clone(),
                path: c.path.clone(),
                text: c.text.clone(),
                score,
            });
        }
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit.max(1));
    hits
}

/// 把检索结果拼成注入提示词的上下文块（带长度上限）。
pub fn context_block(hits: &[KbHit], max_chars: usize) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "\n\n[知识库检索结果（仅作参考；与问题无关时请忽略，不要编造引用）]\n",
    );
    for (i, h) in hits.iter().enumerate() {
        let head = format!("{}. 《{}》{}（片段 {}）\n", i + 1, h.title, h.path, h.ord + 1);
        let body: String = h.text.chars().take(800).collect();
        if s.chars().count() + head.chars().count() + body.chars().count() > max_chars {
            s.push_str("\n（更多片段已省略）\n");
            break;
        }
        s.push_str(&head);
        s.push_str(&body);
        s.push_str("\n\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_ascii_and_cjk() {
        let t = tokenize("Mica 读取 README 文件");
        assert!(t.contains(&"eagent".to_string()));
        assert!(t.contains(&"readme".to_string()));
        assert!(t.contains(&"文".to_string()));
        assert!(t.contains(&"文件".to_string()));
    }

    #[test]
    fn chunking_respects_target() {
        let text = "第一段。\n第二段。\n".repeat(200);
        let chunks = chunk_text(&text, 200, 20);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| c.chars().count() <= 200));
        assert!(chunks.iter().all(|c| !c.trim().is_empty()));
    }

    #[test]
    fn bm25_prefers_relevant_chunk() {
        let chunks = vec![
            KbChunkRow {
                doc_id: 1,
                ord: 0,
                text: "今天天气很好，适合散步".into(),
                title: "a".into(),
                path: "a".into(),
            },
            KbChunkRow {
                doc_id: 2,
                ord: 0,
                text: "Mica 的 MCP 工具审批门是 fail-closed".into(),
                title: "b".into(),
                path: "b".into(),
            },
        ];
        let hits = search(&chunks, "MCP 审批", 5);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].doc_id, 2);
    }

    #[test]
    fn empty_query_returns_no_hits() {
        let chunks = vec![KbChunkRow {
            doc_id: 1,
            ord: 0,
            text: "x".into(),
            title: "x".into(),
            path: "x".into(),
        }];
        assert!(search(&chunks, "，。！", 5).is_empty());
    }

    #[test]
    fn html_strip_removes_tags_and_scripts() {
        let s = strip_html("<html><script>bad()</script><p>你好</p><br/>世界</html>");
        assert!(s.contains("你好"));
        assert!(s.contains("世界"));
        assert!(!s.contains("bad()"));
        assert!(!s.contains("<p>"));
    }

    #[test]
    fn text_file_falls_back_to_gbk() {
        let dir = std::env::temp_dir().join(format!("eagent-kb-gbk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("gbk.txt");
        let (bytes, _, _) = encoding_rs::GBK.encode("中文测试");
        std::fs::write(&p, bytes.as_ref()).unwrap();
        let got = extract_text(&p).unwrap();
        assert!(got.contains("中文测试"), "got: {got}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn docx_extraction_reads_document_xml() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("eagent-kb-docx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.docx");
        {
            let file = std::fs::File::create(&p).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("word/document.xml", opts).unwrap();
            zip.write_all(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                  <w:body><w:p><w:r><w:t>你好，文档</w:t></w:r></w:p></w:body>
                </w:document>"#
                    .as_bytes(),
            )
            .unwrap();
            zip.finish().unwrap();
        }
        let got = extract_text(&p).unwrap();
        assert!(got.contains("你好，文档"), "got: {got}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn context_block_is_capped() {
        let hits = vec![KbHit {
            doc_id: 1,
            ord: 0,
            title: "t".into(),
            path: "p".into(),
            text: "长".repeat(5000),
            score: 1.0,
        }];
        let b = context_block(&hits, 500);
        assert!(b.chars().count() <= 520);
    }
}
