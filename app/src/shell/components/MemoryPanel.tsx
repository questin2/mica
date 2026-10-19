// 长期记忆（策划案 2.0 · Phase 3.3）
//
// 只允许用户显式写入；新会话初始化时注入提示词（上限 800 字符）。
// 不自动把模型输出写进记忆，避免把臆测固化。
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "../icons";

type MemoryRow = { key: string; value: string; updated_at: string };

export function MemoryPanel() {
  const [rows, setRows] = useState<MemoryRow[]>([]);
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  const [err, setErr] = useState("");
  const [msg, setMsg] = useState("");

  const load = useCallback(async () => {
    try {
      setRows(await invoke<MemoryRow[]>("memory_list"));
    } catch (e) {
      setErr(`读取长期记忆失败：${String(e)}`);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const save = async () => {
    if (!key.trim() || !value.trim()) return;
    try {
      await invoke("memory_set", { key: key.trim(), value: value.trim() });
      setKey("");
      setValue("");
      setMsg("已写入；下一次新会话生效");
      setErr("");
      await load();
    } catch (e) {
      setErr(`写入失败：${String(e)}`);
    }
  };

  const remove = async (k: string) => {
    if (!window.confirm(`删除长期记忆「${k}」？`)) return;
    try {
      await invoke("memory_delete", { key: k });
      await load();
    } catch (e) {
      setErr(`删除失败：${String(e)}`);
    }
  };

  return (
    <div className="section">
      <h2 className="section-title">长期记忆</h2>
      <div className="card">
        <p className="card-desc">
          只在你明确写入时保存；每次<b>新会话</b>初始化时注入提示词（最多 800 字符）。
          不会自动把模型输出写进来，避免把猜测固化成事实。
        </p>
        <div className="row gap-2 wrap" style={{ marginTop: "var(--s-2)" }}>
          <input
            className="input"
            style={{ width: 180 }}
            placeholder="key（如 用户名）"
            value={key}
            onChange={(e) => setKey(e.target.value)}
          />
          <input
            className="input"
            style={{ flex: 1, minWidth: 240 }}
            placeholder="value（如 老王；偏好简洁回答）"
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
          <button className="btn primary" disabled={!key.trim() || !value.trim()} onClick={() => void save()}>
            <Icon name="plus" size={14} />
            记住
          </button>
        </div>
        <div className="tc-hint">相同 key 再次写入会覆盖旧值。</div>

        {err && <div className="pc-err">{err}</div>}
        {msg && <div className="pc-desc">{msg}</div>}

        {rows.length > 0 && (
          <div className="card flush" style={{ marginTop: "var(--s-3)" }}>
            <table className="table">
              <thead>
                <tr>
                  <th>Key</th>
                  <th>Value</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={r.key}>
                    <td className="mono">{r.key}</td>
                    <td style={{ whiteSpace: "pre-wrap" }}>{r.value}</td>
                    <td>
                      <button className="btn ghost sm" onClick={() => void remove(r.key)}>
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
