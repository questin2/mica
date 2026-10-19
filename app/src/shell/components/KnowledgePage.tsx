// 知识库（策划案 2.0 · Phase 3.4）
//
// 本地 BM25 检索：导入文件/目录 → 分块入库 → 每轮用户消息自动检索并注入上下文。
// 不依赖外部 RAG 服务或 embedding 模型，离线可用。
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "../icons";
import type { KbDoc, KbHit } from "../types";

type Status = { docs: number; chunks: number };
type ImportResult = {
  imported_docs: number;
  imported_chunks: number;
  skipped: number;
  errors: string[];
};

type Props = { onCount?: (n: number) => void };

export function KnowledgePage({ onCount }: Props) {
  const [docs, setDocs] = useState<KbDoc[]>([]);
  const [status, setStatus] = useState<Status>({ docs: 0, chunks: 0 });
  const [path, setPath] = useState("");
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<KbHit[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [msg, setMsg] = useState("");

  const load = useCallback(async () => {
    try {
      const [d, s] = await Promise.all([
        invoke<KbDoc[]>("kb_list"),
        invoke<Status>("kb_status"),
      ]);
      setDocs(d);
      setStatus(s);
      onCount?.(s.docs);
    } catch (e) {
      setErr(`读取知识库失败：${String(e)}`);
    }
  }, [onCount]);

  useEffect(() => {
    void load();
  }, [load]);

  const importPath = async () => {
    if (!path.trim()) return;
    setBusy(true);
    setErr("");
    setMsg("");
    try {
      const r = await invoke<ImportResult>("kb_import", { path: path.trim() });
      setMsg(
        `导入 ${r.imported_docs} 篇 / ${r.imported_chunks} 块` +
          (r.skipped ? `，跳过 ${r.skipped}` : "") +
          (r.errors.length ? `；${r.errors.slice(0, 3).join("；")}` : ""),
      );
      await load();
    } catch (e) {
      setErr(`导入失败：${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const search = async () => {
    if (!query.trim()) return;
    setBusy(true);
    setErr("");
    try {
      setHits(await invoke<KbHit[]>("kb_search", { query: query.trim(), limit: 5 }));
    } catch (e) {
      setErr(`检索失败：${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const remove = async (d: KbDoc) => {
    if (!window.confirm(`删除《${d.title}》及其 ${d.chunk_count} 个分块？`)) return;
    try {
      await invoke("kb_delete", { docId: d.id });
      await load();
      setMsg(`已删除《${d.title}》`);
    } catch (e) {
      setErr(`删除失败：${String(e)}`);
    }
  };

  return (
    <div className="page-inner">
      <h1 className="page-title">知识库</h1>
      <p className="page-sub">
        已索引 <b>{status.docs}</b> 篇文档 / <b>{status.chunks}</b> 个分块。
        每轮消息会自动检索最相关的 3 个片段并注入上下文；<b>文档只保存在本机</b>。
      </p>

      {err && (
        <div className="banner warn" role="alert">
          <Icon name="info" size={16} />
          <span className="banner-text">{err}</span>
        </div>
      )}
      {msg && !err && (
        <div className="banner" role="status">
          <Icon name="check" size={16} />
          <span className="banner-text">{msg}</span>
        </div>
      )}

      <div className="section">
        <h2 className="section-title">导入</h2>
        <div className="card">
          <div className="field">
            <div className="field-label">
              <b>文件或目录路径</b>
              <span>支持 txt / md / csv / json / 代码 / html / docx / pdf；目录会递归导入（最多 500 个文件）</span>
            </div>
            <div className="field-control">
              <input
                className="input"
                style={{ minWidth: 380 }}
                placeholder="D:\资料\手册.pdf 或 D:\资料"
                value={path}
                spellCheck={false}
                onChange={(e) => setPath(e.target.value)}
              />
              <button className="btn primary" disabled={busy || !path.trim()} onClick={() => void importPath()}>
                <Icon name="upload" size={14} />
                {busy ? "导入中…" : "导入"}
              </button>
            </div>
          </div>
          <div className="tc-hint">
            导入是幂等的：同一文件重复导入会产生重复分块，建议先删除旧文档。索引是本地 BM25，不需要联网。
          </div>
        </div>
      </div>

      <div className="section">
        <h2 className="section-title">检索测试</h2>
        <div className="card">
          <div className="row gap-2">
            <input
              className="input"
              style={{ flex: 1 }}
              placeholder="输入关键词，查看会注入哪些片段"
              value={query}
              spellCheck={false}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void search()}
            />
            <button className="btn" disabled={busy || !query.trim()} onClick={() => void search()}>
              <Icon name="search" size={14} />
              检索
            </button>
          </div>
          {hits !== null && (
            <div style={{ marginTop: "var(--s-3)" }}>
              {hits.length === 0 ? (
                <p className="page-sub">没有命中。先导入文档，或换一个关键词。</p>
              ) : (
                hits.map((h, i) => (
                  <div className="plugin-card" key={`${h.doc_id}-${h.ord}-${i}`} style={{ marginBottom: "var(--s-2)" }}>
                    <div className="pc-head">
                      <div className="pc-title">
                        <Icon name="book" size={16} />
                        <b>{h.title}</b>
                        <span className="badge accent">片段 {h.ord + 1}</span>
                        <span className="badge">score {h.score.toFixed(2)}</span>
                      </div>
                    </div>
                    <div className="pc-dir mono">{h.path}</div>
                    <div className="pc-desc" style={{ whiteSpace: "pre-wrap" }}>
                      {h.text.slice(0, 500)}
                      {h.text.length > 500 ? "…" : ""}
                    </div>
                  </div>
                ))
              )}
            </div>
          )}
        </div>
      </div>

      <div className="section">
        <div className="skill-toolbar">
          <h2 className="section-title" style={{ margin: 0 }}>
            已索引文档
          </h2>
          <span className="spacer" />
          <button className="btn ghost sm" onClick={() => void load()}>
            <Icon name="refresh" size={14} />
            刷新
          </button>
        </div>
        {docs.length === 0 ? (
          <p className="page-sub">还没有文档。在上方输入路径导入。</p>
        ) : (
          <div className="card flush" style={{ marginTop: "var(--s-2)" }}>
            <table className="table">
              <thead>
                <tr>
                  <th>文档</th>
                  <th>路径</th>
                  <th className="num">分块</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {docs.map((d) => (
                  <tr key={d.id}>
                    <td>{d.title}</td>
                    <td className="mono muted ellipsis" title={d.path}>
                      {d.path}
                    </td>
                    <td className="num tabular">{d.chunk_count}</td>
                    <td>
                      <button className="btn ghost sm" onClick={() => void remove(d)}>
                        <Icon name="x" size={14} />
                        删除
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
