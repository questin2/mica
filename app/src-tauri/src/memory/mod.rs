//! 记忆层门面（策划案 2.0 · Phase 3）。
//!
//! - `db`：SQLite（会话/消息/工具/审计/记忆/流程/知识库）
//! - `kb`：文档抽取、分块、BM25 检索
//!
//! 数据库懒建并缓存在 `EngineState.db`；同时登记到 `GLOBAL_DB`，
//! 供 `log::audit_log` 在不持有 AppHandle 的路径上并行落库。

pub mod db;
pub mod kb;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use tauri::Manager;

use crate::config;
use crate::state::EngineState;

pub use db::Db;

static GLOBAL_DB: OnceLock<Arc<Db>> = OnceLock::new();

/// 取（或懒建）数据库。路径：`data_dir()/eagent.db`。
pub fn db(app: &tauri::AppHandle) -> Result<Arc<Db>, String> {
    let st = app.state::<EngineState>();
    let mut guard = st.db.lock().map_err(|e| format!("数据库状态锁失败：{e}"))?;
    if let Some(d) = guard.as_ref() {
        return Ok(d.clone());
    }
    let path = PathBuf::from(config::data_dir()).join("eagent.db");
    let d = Arc::new(Db::open(&path)?);
    *guard = Some(d.clone());
    let _ = GLOBAL_DB.set(d.clone());
    crate::log::chain_log(&format!("memory: 数据库已就绪 → {}", path.display()));
    Ok(d)
}

/// 不依赖 AppHandle 的全局句柄（审计落库用）；未初始化时返回 None。
pub fn try_db() -> Option<Arc<Db>> {
    GLOBAL_DB.get().cloned()
}

// ---------- 会话 ----------

/// 开一个新会话并设为当前会话。返回 session id。
pub fn new_session(app: &tauri::AppHandle, provider: &str, workdir: &str) -> Result<String, String> {
    let d = db(app)?;
    let id = d.create_session(provider, workdir)?;
    let st = app.state::<EngineState>();
    if let Ok(mut cur) = st.session_id.lock() {
        *cur = Some(id.clone());
    }
    Ok(id)
}

/// 当前会话 id。
pub fn current_session(app: &tauri::AppHandle) -> Option<String> {
    app.state::<EngineState>()
        .session_id
        .lock()
        .ok()
        .and_then(|g| g.clone())
}

/// 确保有当前会话；没有就新建。
pub fn ensure_session(app: &tauri::AppHandle, provider: &str, workdir: &str) -> Result<String, String> {
    if let Some(id) = current_session(app) {
        let d = db(app)?;
        if d.session_exists(&id)? {
            return Ok(id);
        }
    }
    new_session(app, provider, workdir)
}

/// 设置当前会话（恢复历史时用）。
pub fn set_current_session(app: &tauri::AppHandle, id: Option<String>) {
    if let Ok(mut cur) = app.state::<EngineState>().session_id.lock() {
        *cur = id;
    }
}

#[derive(Clone, Debug)]
pub struct ToolCallRecord {
    pub name: String,
    pub args: String,
    pub result: String,
    pub approved: bool,
}

/// 落一个回合：用户消息 + 助手回复 + 工具调用。
pub fn persist_turn(
    app: &tauri::AppHandle,
    provider: &str,
    workdir: &str,
    user_message: &str,
    assistant_reply: &str,
    tool_calls: &[ToolCallRecord],
) -> Result<(), String> {
    let sid = ensure_session(app, provider, workdir)?;
    let d = db(app)?;
    d.append_message(&sid, "user", user_message)?;
    d.set_title_if_empty(&sid, user_message)?;
    if !assistant_reply.is_empty() {
        d.append_message(&sid, "assistant", assistant_reply)?;
    }
    for t in tool_calls {
        d.append_tool_call(&sid, &t.name, &t.args, &t.result, t.approved)?;
    }
    Ok(())
}

// ---------- 长期记忆 ----------

pub fn memory_prompt_block(app: &tauri::AppHandle, max_chars: usize) -> String {
    match db(app).and_then(|d| d.memory_prompt_block(max_chars)) {
        Ok(s) => s,
        Err(e) => {
            crate::log::chain_log(&format!("memory: 读取长期记忆失败：{e}"));
            String::new()
        }
    }
}

// ---------- 知识库 ----------

#[derive(serde::Serialize)]
pub struct KbImportResult {
    pub imported_docs: usize,
    pub imported_chunks: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

fn supported_ext(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "txt" | "md" | "markdown" | "csv" | "json" | "log" | "rs" | "ts" | "tsx" | "js"
            | "jsx" | "py" | "java" | "c" | "cpp" | "h" | "go" | "html" | "htm" | "docx" | "pdf"
    )
}

fn collect_files(path: &Path, out: &mut Vec<PathBuf>, depth: usize) -> Result<(), String> {
    if depth > 8 || out.len() >= 500 {
        return Ok(());
    }
    if path.is_file() {
        if supported_ext(path) {
            out.push(path.to_path_buf());
        }
        return Ok(());
    }
    if !path.is_dir() {
        return Err(format!("路径不存在：{}", path.display()));
    }
    let entries = std::fs::read_dir(path).map_err(|e| format!("读取目录失败：{e}"))?;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_files(&p, out, depth + 1)?;
        } else if supported_ext(&p) {
            out.push(p);
        }
        if out.len() >= 500 {
            break;
        }
    }
    Ok(())
}

/// 导入文件或目录（目录递归，最多 500 个文件）。
pub fn kb_import_path(app: &tauri::AppHandle, path: &str) -> Result<KbImportResult, String> {
    let root = PathBuf::from(path.trim());
    if path.trim().is_empty() {
        return Err("路径不能为空".into());
    }
    let mut files = Vec::new();
    collect_files(&root, &mut files, 0)?;
    if files.is_empty() {
        return Err("没有找到可导入的文件（支持 txt/md/csv/json/代码/docx/pdf/html）".into());
    }
    let d = db(app)?;
    let mut result = KbImportResult {
        imported_docs: 0,
        imported_chunks: 0,
        skipped: 0,
        errors: Vec::new(),
    };
    for f in files {
        let meta = match std::fs::metadata(&f) {
            Ok(m) => m,
            Err(e) => {
                result.skipped += 1;
                result.errors.push(format!("{}：{e}", f.display()));
                continue;
            }
        };
        if meta.len() > 32 * 1024 * 1024 {
            result.skipped += 1;
            result.errors.push(format!("{}：超过 32MB，已跳过", f.display()));
            continue;
        }
        let text = match kb::extract_text(&f) {
            Ok(t) if !t.trim().is_empty() => t,
            Ok(_) => {
                result.skipped += 1;
                result.errors.push(format!("{}：没有可提取的文本", f.display()));
                continue;
            }
            Err(e) => {
                result.skipped += 1;
                result.errors.push(format!("{}：{e}", f.display()));
                continue;
            }
        };
        let chunks = kb::chunk_text(&text, 700, 80);
        if chunks.is_empty() {
            result.skipped += 1;
            continue;
        }
        let title = f
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("未命名")
            .to_string();
        match d.kb_insert_doc(&f.to_string_lossy(), &title, &chunks) {
            Ok(_) => {
                result.imported_docs += 1;
                result.imported_chunks += chunks.len();
            }
            Err(e) => {
                result.skipped += 1;
                result.errors.push(format!("{}：{e}", f.display()));
            }
        }
    }
    if result.imported_docs == 0 && result.errors.is_empty() {
        return Err("没有导入任何文档".into());
    }
    crate::log::chain_log(&format!(
        "kb: 导入 {} 篇 / {} 块，跳过 {}",
        result.imported_docs, result.imported_chunks, result.skipped
    ));
    Ok(result)
}

pub fn kb_search(app: &tauri::AppHandle, query: &str, limit: usize) -> Result<Vec<kb::KbHit>, String> {
    let d = db(app)?;
    let chunks = d.kb_all_chunks()?;
    Ok(kb::search(&chunks, query, limit))
}

/// 给模型的知识库上下文（无文档/无命中时返回空串）。
pub fn kb_context_for(app: &tauri::AppHandle, query: &str, max_chars: usize) -> String {
    match kb_search(app, query, 3) {
        Ok(hits) => kb::context_block(&hits, max_chars),
        Err(e) => {
            crate::log::chain_log(&format!("kb: 检索失败：{e}"));
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions_are_case_insensitive() {
        assert!(supported_ext(Path::new("a.MD")));
        assert!(supported_ext(Path::new("a.docx")));
        assert!(!supported_ext(Path::new("a.exe")));
    }

    #[test]
    fn chunk_and_tokenize_are_exported() {
        assert!(!kb::chunk_text("你好世界", 700, 80).is_empty());
        assert!(!kb::tokenize("你好").is_empty());
    }

    #[test]
    fn kb_pipeline_chunk_insert_search_context() {
        let db = Db::open_memory().unwrap();
        let text = "Mica 的 MCP 工具调用必须经过审批门，超时按拒绝处理。\n\
                    知识库使用本地 BM25 检索，不依赖外部服务。\n\
                    今天天气很好，适合散步。";
        let chunks = kb::chunk_text(text, 90, 20);
        db.kb_insert_doc("D:\\doc.md", "doc.md", &chunks).unwrap();
        let all = db.kb_all_chunks().unwrap();
        let hits = kb::search(&all, "MCP 审批门", 3);
        assert!(!hits.is_empty());
        assert!(hits[0].text.contains("MCP"), "top hit: {:?}", hits[0]);
        let ctx = kb::context_block(&hits, 2000);
        assert!(ctx.contains("知识库检索结果"));
        assert!(ctx.contains("doc.md"));
    }
}
