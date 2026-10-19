// 历史页：SQLite 多会话列表 + 消息回放（策划案 2.0 · Phase 3.5）
//
// `session.json` 仍是前端崩溃恢复快照；本页读的是引擎侧 `eagent.db` 的**回合级**记录。
// 打开历史会话会把当前会话指针切过去；"载入到对话"只恢复显示，不伪造模型上下文。
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "../icons";
import { baseName } from "../format";
import type { MessageRow, Msg, SessionRow } from "../types";

type Props = {
  workdir: string;
  turns: number;
  busy: boolean;
  /** 把历史消息载入对话视图（仅显示；引擎上下文不会自动迁移） */
  onLoad?: (messages: Msg[], workdir: string) => void;
};

type Detail = { session: SessionRow; messages: MessageRow[] };

export function HistoryPage({ workdir, turns, busy, onLoad }: Props) {
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [detail, setDetail] = useState<Detail | null>(null);
  const [err, setErr] = useState("");
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setSessions(await invoke<SessionRow[]>("list_sessions"));
      setErr("");
    } catch (e) {
      setErr(`读取历史失败：${String(e)}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const open = async (s: SessionRow) => {
    try {
      setDetail(await invoke<Detail>("open_session", { id: s.id }));
      setErr("");
    } catch (e) {
      setErr(`打开会话失败：${String(e)}`);
    }
  };

  const remove = async (s: SessionRow) => {
    if (!window.confirm(`删除会话「${s.title || "未命名"}」及其消息记录？`)) return;
    try {
      await invoke("delete_session", { id: s.id });
      if (detail?.session.id === s.id) setDetail(null);
      await load();
    } catch (e) {
      setErr(`删除失败：${String(e)}`);
    }
  };

  const loadToChat = () => {
    if (!detail || !onLoad) return;
    const msgs: Msg[] = detail.messages
      .filter((m) => m.role === "user" || m.role === "assistant")
      .map((m) => ({
        id: m.id,
        role: m.role as "user" | "assistant",
        text: m.content,
        ts: formatTs(m.ts),
      }));
    onLoad(msgs, detail.session.workdir || workdir);
  };

  return (
    <div className="page-inner">
      <h1 className="page-title">历史</h1>
      <p className="page-sub">
        当前运行：<b>{baseName(workdir) || "未设置"}</b> · {turns} 个用户回合
        {busy ? " · 进行中" : ""}。以下为引擎侧 SQLite 记录的历史会话；
        <span className="mono">session.json</span> 仍负责前端崩溃恢复。
      </p>

      {err && (
        <div className="banner warn" role="alert">
          <Icon name="info" size={16} />
          <span className="banner-text">{err}</span>
        </div>
      )}

      <div className="section">
        <div className="skill-toolbar">
          <h2 className="section-title" style={{ margin: 0 }}>
            会话
          </h2>
          <span className="spacer" />
          <button className="btn ghost sm" disabled={loading} onClick={() => void load()}>
            <Icon name="refresh" size={14} />
            {loading ? "读取中…" : "刷新"}
          </button>
        </div>
        {sessions.length === 0 ? (
          <p className="page-sub">还没有历史会话。发送第一条消息后会自动创建。</p>
        ) : (
          <div className="card flush" style={{ marginTop: "var(--s-2)" }}>
            <table className="table">
              <thead>
                <tr>
                  <th>标题</th>
                  <th>通道</th>
                  <th>工作目录</th>
                  <th className="num">消息</th>
                  <th>更新时间</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {sessions.map((s) => (
                  <tr key={s.id}>
                    <td className="ellipsis" title={s.title || "(未命名)"}>
                      {s.title || "(未命名)"}
                    </td>
                    <td>
                      <span className="badge">{providerLabel(s.provider)}</span>
                    </td>
                    <td className="mono muted ellipsis" title={s.workdir}>
                      {s.workdir || "—"}
                    </td>
                    <td className="num tabular">{s.message_count}</td>
                    <td className="muted">{formatTs(s.updated_at)}</td>
                    <td>
                      <button className="btn ghost sm" onClick={() => void open(s)}>
                        <Icon name="search" size={14} />
                        查看
                      </button>
                      <button className="btn ghost sm" onClick={() => void remove(s)}>
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

      {detail && (
        <div className="section">
          <div className="skill-toolbar">
            <h2 className="section-title" style={{ margin: 0 }}>
              {detail.session.title || "(未命名会话)"}
            </h2>
            <span className="spacer" />
            {onLoad && (
              <button className="btn sm" onClick={loadToChat}>
                <Icon name="download" size={14} />
                载入到对话视图
              </button>
            )}
            <button className="btn ghost sm" onClick={() => setDetail(null)}>
              <Icon name="x" size={14} />
              关闭
            </button>
          </div>
          <p className="page-sub">
            {providerLabel(detail.session.provider)} · {detail.session.workdir || "—"} ·{" "}
            {detail.messages.length} 条记录
          </p>
          <div className="plugin-list">
            {detail.messages.map((m) => (
              <div className="plugin-card" key={m.id}>
                <div className="pc-head">
                  <div className="pc-title">
                    <span className={`badge ${m.role === "user" ? "accent" : ""}`}>{m.role}</span>
                    <span className="muted">{formatTs(m.ts)}</span>
                  </div>
                </div>
                <div className="pc-desc" style={{ whiteSpace: "pre-wrap" }}>
                  {m.content.length > 1200 ? `${m.content.slice(0, 1200)}…` : m.content}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function providerLabel(p: string): string {
  if (p === "ollama") return "Ollama";
  if (p === "deepseek-web") return "DeepSeek 网页";
  return p || "未知";
}

function formatTs(ms: string): string {
  const n = Number(ms);
  if (!Number.isFinite(n) || n <= 0) return ms;
  const d = new Date(n);
  const pad = (x: number) => String(x).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
