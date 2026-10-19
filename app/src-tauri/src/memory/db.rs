//! SQLite 记忆层（策划案 2.0 · Phase 3）。
//!
//! 单一 `eagent.db`：会话 / 消息 / 工具调用 / 审计 / 长期记忆 / 流程 run / 知识库。
//! 同步 API + `Mutex<Connection>`，与现有阻塞式回合模型一致。
//! 写模式 WAL + busy_timeout；schema 用 `PRAGMA user_version` 顺序迁移。

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, Connection};

use crate::now_ms;
use crate::SEQ;

/// 当前 schema 版本。每次结构性变更 +1，并在 `migrate` 里追加分支。
const SCHEMA_VERSION: i64 = 1;

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SessionRow {
    pub id: String,
    pub provider: String,
    pub title: String,
    pub workdir: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct MessageRow {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub ts: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct MemoryRow {
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct KbDoc {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub added_at: String,
    pub chunk_count: i64,
}

#[derive(Clone, Debug)]
pub struct KbChunkRow {
    pub doc_id: i64,
    pub ord: i64,
    pub text: String,
    pub title: String,
    pub path: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: String,
    pub status: String,
    pub current_step: i64,
    pub total_steps: i64,
    pub created_at: String,
    pub updated_at: String,
    pub error: Option<String>,
}

fn ts() -> String {
    now_ms().to_string()
}

fn open_conn(path: &Path) -> Result<Connection, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建数据库目录失败：{e}"))?;
    }
    let conn = Connection::open(path).map_err(|e| format!("打开数据库失败（{}）：{e}", path.display()))?;
    conn.busy_timeout(Duration::from_secs(3)).map_err(|e| e.to_string())?;
    let _: String = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
        .map_err(|e| format!("设置 WAL 失败：{e}"))?;
    conn.pragma_update(None, "foreign_keys", "ON").map_err(|e| e.to_string())?;
    Ok(conn)
}

impl Db {
    /// 打开/创建数据库并迁移到当前 schema。
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = open_conn(path)?;
        migrate(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// 测试用内存库。
    #[cfg(test)]
    pub fn open_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(|e| e.to_string())?;
        migrate(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.conn.lock().map_err(|e| format!("数据库锁失败：{e}"))
    }

    // ---------- 会话 / 消息 ----------

    pub fn create_session(&self, provider: &str, workdir: &str) -> Result<String, String> {
        let conn = self.lock()?;
        let id = format!("s{}-{}", now_ms(), SEQ.fetch_add(1, Ordering::Relaxed));
        let now = ts();
        conn.execute(
            "INSERT INTO sessions (id, provider, title, workdir, created_at, updated_at) VALUES (?1,?2,'',?3,?4,?4)",
            params![id, provider, workdir, now],
        )
        .map_err(|e| format!("创建会话失败：{e}"))?;
        Ok(id)
    }

    pub fn session_exists(&self, id: &str) -> Result<bool, String> {
        let conn = self.lock()?;
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions WHERE id=?1", params![id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    pub fn set_title_if_empty(&self, id: &str, title: &str) -> Result<(), String> {
        let conn = self.lock()?;
        let title: String = title.trim().chars().take(60).collect();
        if title.is_empty() {
            return Ok(());
        }
        conn.execute(
            "UPDATE sessions SET title=?1, updated_at=?2 WHERE id=?3 AND (title IS NULL OR title='')",
            params![title, ts(), id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn append_message(&self, session_id: &str, role: &str, content: &str) -> Result<(), String> {
        let conn = self.lock()?;
        let now = ts();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, ts) VALUES (?1,?2,?3,?4)",
            params![session_id, role, content, now],
        )
        .map_err(|e| format!("写入消息失败：{e}"))?;
        conn.execute(
            "UPDATE sessions SET updated_at=?1 WHERE id=?2",
            params![now, session_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load_messages(&self, session_id: &str) -> Result<Vec<MessageRow>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT id, role, content, ts FROM messages WHERE session_id=?1 ORDER BY id ASC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![session_id], |r| {
                Ok(MessageRow {
                    id: r.get(0)?,
                    role: r.get(1)?,
                    content: r.get(2)?,
                    ts: r.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionRow>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.provider, s.title, s.workdir, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id=s.id)
                 FROM sessions s ORDER BY s.updated_at DESC LIMIT 200",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SessionRow {
                    id: r.get(0)?,
                    provider: r.get(1)?,
                    title: r.get(2)?,
                    workdir: r.get(3)?,
                    created_at: r.get(4)?,
                    updated_at: r.get(5)?,
                    message_count: r.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn delete_session(&self, id: &str) -> Result<(), String> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM messages WHERE session_id=?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tool_calls WHERE session_id=?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM sessions WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---------- 工具调用 / 审计 ----------

    pub fn append_tool_call(
        &self,
        session_id: &str,
        name: &str,
        args: &str,
        result: &str,
        approved: bool,
    ) -> Result<(), String> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO tool_calls (session_id, name, args, result, approved, ts) VALUES (?1,?2,?3,?4,?5,?6)",
            params![session_id, name, args, result, if approved { 1 } else { 0 }, ts()],
        )
        .map_err(|e| format!("写入工具调用失败：{e}"))?;
        Ok(())
    }

    pub fn append_audit(&self, event: &str, detail: &str) -> Result<(), String> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO audit (ts, event, detail) VALUES (?1,?2,?3)",
            params![ts(), event, detail],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---------- 长期记忆 KV ----------

    pub fn memory_list(&self) -> Result<Vec<MemoryRow>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT key, value, updated_at FROM memory ORDER BY updated_at DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(MemoryRow {
                    key: r.get(0)?,
                    value: r.get(1)?,
                    updated_at: r.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn memory_set(&self, key: &str, value: &str) -> Result<(), String> {
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() {
            return Err("记忆的 key 不能为空".into());
        }
        if value.is_empty() {
            return Err("记忆的 value 不能为空".into());
        }
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO memory (key, value, updated_at) VALUES (?1,?2,?3)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
            params![key, value, ts()],
        )
        .map_err(|e| format!("写入记忆失败：{e}"))?;
        Ok(())
    }

    pub fn memory_delete(&self, key: &str) -> Result<(), String> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM memory WHERE key=?1", params![key])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 注入提示词的长期记忆块（按字符上限裁剪）。
    pub fn memory_prompt_block(&self, max_chars: usize) -> Result<String, String> {
        let rows = self.memory_list()?;
        if rows.is_empty() {
            return Ok(String::new());
        }
        let mut s = String::from("\n\n---\n\n## 长期记忆（用户明确要求记住的内容）\n\n");
        for r in rows {
            let line = format!("- {}：{}\n", r.key, r.value.replace('\n', " "));
            if s.chars().count() + line.chars().count() > max_chars {
                s.push_str("- （更多记忆已省略）\n");
                break;
            }
            s.push_str(&line);
        }
        Ok(s)
    }

    // ---------- 流程 run ----------

    pub fn create_workflow_run(&self, id: &str, workflow_id: &str, total: usize) -> Result<(), String> {
        let conn = self.lock()?;
        let now = ts();
        conn.execute(
            "INSERT INTO workflow_runs (id, workflow_id, status, current_step, total_steps, created_at, updated_at, error)
             VALUES (?1,?2,'running',0,?3,?4,?4,NULL)",
            params![id, workflow_id, total as i64, now],
        )
        .map_err(|e| format!("创建流程记录失败：{e}"))?;
        Ok(())
    }

    pub fn update_workflow_run(
        &self,
        id: &str,
        status: &str,
        current_step: usize,
        error: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.lock()?;
        // 成功才把进度推到 total；失败/取消保留最后一次已完成的步骤。
        conn.execute(
            "UPDATE workflow_runs
             SET status=?1,
                 current_step=CASE WHEN ?1='done' THEN ?2 ELSE current_step END,
                 updated_at=?3,
                 error=?4
             WHERE id=?5",
            params![status, current_step as i64, ts(), error, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_workflow_runs(&self) -> Result<Vec<WorkflowRun>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, workflow_id, status, current_step, total_steps, created_at, updated_at, error
                 FROM workflow_runs ORDER BY created_at DESC LIMIT 50",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(WorkflowRun {
                    id: r.get(0)?,
                    workflow_id: r.get(1)?,
                    status: r.get(2)?,
                    current_step: r.get(3)?,
                    total_steps: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                    error: r.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    // ---------- 知识库 ----------

    pub fn kb_insert_doc(&self, path: &str, title: &str, chunks: &[String]) -> Result<i64, String> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO kb_docs (path, title, added_at, chunk_count) VALUES (?1,?2,?3,?4)",
            params![path, title, ts(), chunks.len() as i64],
        )
        .map_err(|e| format!("写入知识库文档失败：{e}"))?;
        let doc_id = tx.last_insert_rowid();
        for (i, c) in chunks.iter().enumerate() {
            tx.execute(
                "INSERT INTO kb_chunks (doc_id, ord, text) VALUES (?1,?2,?3)",
                params![doc_id, i as i64, c],
            )
            .map_err(|e| format!("写入知识库分块失败：{e}"))?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(doc_id)
    }

    pub fn kb_list_docs(&self) -> Result<Vec<KbDoc>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT id, path, title, added_at, chunk_count FROM kb_docs ORDER BY id DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(KbDoc {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    title: r.get(2)?,
                    added_at: r.get(3)?,
                    chunk_count: r.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn kb_delete_doc(&self, doc_id: i64) -> Result<(), String> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM kb_chunks WHERE doc_id=?1", params![doc_id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM kb_docs WHERE id=?1", params![doc_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn kb_all_chunks(&self) -> Result<Vec<KbChunkRow>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT c.doc_id, c.ord, c.text, d.title, d.path
                 FROM kb_chunks c JOIN kb_docs d ON d.id=c.doc_id
                 ORDER BY c.doc_id ASC, c.ord ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(KbChunkRow {
                    doc_id: r.get(0)?,
                    ord: r.get(1)?,
                    text: r.get(2)?,
                    title: r.get(3)?,
                    path: r.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn kb_status(&self) -> Result<(i64, i64), String> {
        let conn = self.lock()?;
        let docs: i64 = conn
            .query_row("SELECT COUNT(*) FROM kb_docs", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let chunks: i64 = conn
            .query_row("SELECT COUNT(*) FROM kb_chunks", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok((docs, chunks))
    }
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if version < 1 {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
              id TEXT PRIMARY KEY,
              provider TEXT NOT NULL,
              title TEXT NOT NULL DEFAULT '',
              workdir TEXT NOT NULL DEFAULT '',
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              role TEXT NOT NULL,
              content TEXT NOT NULL,
              ts TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, id);
            CREATE TABLE IF NOT EXISTS tool_calls (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              name TEXT NOT NULL,
              args TEXT NOT NULL,
              result TEXT,
              approved INTEGER NOT NULL,
              ts TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memory (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS audit (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              ts TEXT NOT NULL,
              event TEXT NOT NULL,
              detail TEXT
            );
            CREATE TABLE IF NOT EXISTS workflow_runs (
              id TEXT PRIMARY KEY,
              workflow_id TEXT NOT NULL,
              status TEXT NOT NULL,
              current_step INTEGER NOT NULL DEFAULT 0,
              total_steps INTEGER NOT NULL DEFAULT 0,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              error TEXT
            );
            CREATE TABLE IF NOT EXISTS kb_docs (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              path TEXT NOT NULL,
              title TEXT NOT NULL,
              added_at TEXT NOT NULL,
              chunk_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS kb_chunks (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              doc_id INTEGER NOT NULL,
              ord INTEGER NOT NULL,
              text TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_kb_chunks_doc ON kb_chunks(doc_id);
            "#,
        )
        .map_err(|e| format!("初始化数据库 schema 失败：{e}"))?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_and_message_roundtrip() {
        let db = Db::open_memory().unwrap();
        let sid = db.create_session("deepseek-web", r"D:\Mica").unwrap();
        db.append_message(&sid, "user", "你好").unwrap();
        db.append_message(&sid, "assistant", "你好，有什么可以帮你？").unwrap();
        db.set_title_if_empty(&sid, "你好").unwrap();

        let sessions = db.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "你好");
        assert_eq!(sessions[0].message_count, 2);
        let msgs = db.load_messages(&sid).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn deleting_session_removes_messages() {
        let db = Db::open_memory().unwrap();
        let sid = db.create_session("ollama", "").unwrap();
        db.append_message(&sid, "user", "x").unwrap();
        db.delete_session(&sid).unwrap();
        assert!(db.load_messages(&sid).unwrap().is_empty());
        assert!(db.list_sessions().unwrap().is_empty());
    }

    #[test]
    fn memory_upsert_and_block() {
        let db = Db::open_memory().unwrap();
        db.memory_set("name", "老王").unwrap();
        db.memory_set("name", "老王（改）").unwrap();
        let rows = db.memory_list().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "老王（改）");
        let block = db.memory_prompt_block(1000).unwrap();
        assert!(block.contains("老王（改）"));
        db.memory_delete("name").unwrap();
        assert!(db.memory_list().unwrap().is_empty());
    }

    #[test]
    fn kb_doc_insert_and_delete() {
        let db = Db::open_memory().unwrap();
        let id = db
            .kb_insert_doc(r"D:\a.md", "a.md", &["第一段".into(), "第二段".into()])
            .unwrap();
        assert_eq!(db.kb_status().unwrap(), (1, 2));
        assert_eq!(db.kb_all_chunks().unwrap().len(), 2);
        db.kb_delete_doc(id).unwrap();
        assert_eq!(db.kb_status().unwrap(), (0, 0));
    }

    #[test]
    fn workflow_run_status_updates() {
        let db = Db::open_memory().unwrap();
        db.create_workflow_run("r1", "wf-demo", 3).unwrap();
        db.update_workflow_run("r1", "done", 3, None).unwrap();
        let runs = db.list_workflow_runs().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "done");
        assert_eq!(runs[0].current_step, 3);
    }

    #[test]
    fn tool_call_and_audit_are_queryable() {
        let db = Db::open_memory().unwrap();
        let sid = db.create_session("deepseek-web", "").unwrap();
        db.append_tool_call(&sid, "calc", r#"{"expr":"1+1"}"#, "2", true)
            .unwrap();
        db.append_audit("test", "detail").unwrap();
        let conn = db.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM tool_calls", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let m: i64 = conn.query_row("SELECT COUNT(*) FROM audit", [], |r| r.get(0)).unwrap();
        assert_eq!(m, 1);
    }
}
