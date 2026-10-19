// 「MCP」页：预置库 + 勾选即用（对标豆包桌面端）+ 高级用法（手动/导入）
//
// 主视图是**预置库**：每个 server 一行，右侧一个开关，勾上就写进 config/mcp.json 并启用。
// 下方列出已启用的 server 及其实时连接状态与工具清单。
// 「添加 server」「从 JSON 导入」收进高级区，默认折叠。
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "../icons";

export type McpToolInfo = { name: string; description: string };
export type McpServerStatus = {
  name: string;
  command: string;
  args: string[];
  tools: McpToolInfo[];
  error: string | null;
};
export type CatalogEntry = {
  id: string;
  name: string;
  description: string;
  category: string;
  command: string;
  needs: string[];
  note: string;
  installed: boolean;
  enabled: boolean;
};

type Props = { onCount?: (n: number) => void };

const EMPTY_FORM = { name: "", command: "", args: "", env: "", enabled: true };

export function McpPage({ onCount }: Props) {
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
  const [list, setList] = useState<McpServerStatus[]>([]);
  const [err, setErr] = useState("");
  const [msg, setMsg] = useState("");
  const [busy, setBusy] = useState(false);
  const [advanced, setAdvanced] = useState<"none" | "add" | "import">("none");
  const [form, setForm] = useState(EMPTY_FORM);
  const [importText, setImportText] = useState("");

  const load = useCallback(async () => {
    setBusy(true);
    try {
      const [c, s] = await Promise.all([
        invoke<CatalogEntry[]>("mcp_catalog"),
        invoke<McpServerStatus[]>("mcp_status"),
      ]);
      setCatalog(c);
      setList(s);
      setErr("");
      onCount?.(s.length);
    } catch (e) {
      setErr(`读取失败：${String(e)}`);
    } finally {
      setBusy(false);
    }
  }, [onCount]);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (label: string, fn: () => Promise<string>) => {
    setBusy(true);
    try {
      const m = await fn();
      setMsg(`${m}　重启应用或重新注入提示词后生效。`);
      await load();
      setErr("");
    } catch (e) {
      setErr(`${label}失败：${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const toggle = (c: CatalogEntry) => {
    // 【P4.5】启用 = 允许应用启动该子进程。审批门只覆盖"工具调用"，不覆盖"server 启动"，
    // 因此这里必须让用户看到将执行的完整命令。
    if (
      !c.enabled &&
      !window.confirm(
        `启用「${c.name}」将允许 Mica 启动以下子进程：\n\n${c.command}\n\n` +
          `MCP server 本身是第三方代码，启动即执行。确定继续？`,
      )
    ) {
      return;
    }
    void run(c.enabled ? "停用" : "启用", () =>
      invoke<string>("mcp_set_catalog", { id: c.id, enabled: !c.enabled }),
    );
  };

  const doSave = () =>
    run("保存", async () => {
      const args = form.args.trim() ? form.args.split(/\r?\n/).map((s) => s.trim()).filter(Boolean) : [];
      const env: Record<string, string> = {};
      for (const line of form.env.split(/\r?\n/)) {
        const i = line.indexOf("=");
        if (i > 0) env[line.slice(0, i).trim()] = line.slice(i + 1).trim();
      }
      if (
        form.enabled &&
        !window.confirm(
          `保存并启用将允许启动子进程：\n\n${form.command.trim()} ${args.join(" ")}\n\n` +
            `MCP server 是第三方代码，启动即执行。继续？`,
        )
      ) {
        return "已取消（未保存）";
      }
      const r = await invoke<string>("mcp_save_server", {
        name: form.name, command: form.command, args, env, enabled: form.enabled,
      });
      setForm(EMPTY_FORM);
      setAdvanced("none");
      return r;
    });

  const doImport = () => {
    if (
      !window.confirm(
        "导入的 server 会默认启用；它们启动时会执行配置里的 command。\n\n" +
          "请确认 JSON 来自可信来源。继续导入？",
      )
    ) {
      return;
    }
    void run("导入", async () => {
      const names = await invoke<string[]>("mcp_import", { text: importText });
      setImportText("");
      setAdvanced("none");
      return `已导入：${names.join("、")}`;
    });
  };

  const toolTotal = list.reduce((n, s) => n + s.tools.length, 0);
  const badCount = list.filter((s) => s.error).length;
  const enabledCount = catalog.filter((c) => c.enabled).length;

  return (
    <div className="page-inner">
      <h1 className="page-title">MCP</h1>
      <p className="page-sub">
        已启用 <b>{enabledCount}</b> 个扩展能力，共提供 <b>{toolTotal}</b> 个工具
        {badCount > 0 && ` ｜ ${badCount} 个连接异常`}。工具以{" "}
        <span className="mono">mcp:&lt;名称&gt;.&lt;工具&gt;</span> 注入提示词，
        <b>调用必须经审批</b>。
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

      {/* ---------- 预置库：勾选即用 ---------- */}
      <div className="section">
        <h2 className="section-title">能力库</h2>
        <div className="plugin-list">
          {catalog.map((c) => (
            <label className={`plugin-card catalog-row${c.enabled ? " on" : ""}`} key={c.id}>
              <div className="pc-head">
                <div className="pc-title">
                  <Icon name="zap" size={16} />
                  <b>{c.name}</b>
                  <span className="badge">{c.category}</span>
                  {c.needs.length > 0 && <span className="badge danger">需凭据</span>}
                </div>
                <input
                  type="checkbox"
                  className="catalog-toggle"
                  checked={c.enabled}
                  disabled={busy}
                  onChange={() => toggle(c)}
                />
              </div>
              <div className="pc-desc">{c.description}</div>
              <div className="pc-dir mono">{c.command}</div>
              {c.needs.length > 0 && (
                <div className="pc-err">
                  <Icon name="lock" size={14} />
                  勾选后需在 config\mcp.json 的 env 里填：{c.needs.join("、")}
                </div>
              )}
              {c.note && <div className="tc-hint">{c.note}</div>}
            </label>
          ))}
        </div>
      </div>

      {/* ---------- 已启用：实时状态 ---------- */}
      <div className="section">
        <div className="skill-toolbar">
          <h2 className="section-title" style={{ margin: 0 }}>
            已启用
          </h2>
          <span className="spacer" />
          <button className="btn ghost sm" disabled={busy} onClick={() => void load()}>
            <Icon name="refresh" size={14} />
            {busy ? "读取中…" : "刷新状态"}
          </button>
        </div>

        {list.length === 0 ? (
          <p className="page-sub">还没有启用任何能力。在上面的能力库里勾选即可。</p>
        ) : (
          <div className="plugin-list" style={{ marginTop: "var(--s-3)" }}>
            {list.map((s) => (
              <div className={`plugin-card${s.error ? " has-error" : ""}`} key={s.name}>
                <div className="pc-head">
                  <div className="pc-title">
                    <Icon name="zap" size={16} />
                    <b>{s.name}</b>
                    {s.error ? (
                      <span className="badge danger">连接异常</span>
                    ) : (
                      <span className="badge accent">{s.tools.length} 个工具</span>
                    )}
                  </div>
                  <button
                    className="btn ghost sm"
                    disabled={busy}
                    title="删除这条配置（不只是停用）"
                    onClick={() => {
                      if (!window.confirm(`删除「${s.name}」的配置？`)) return;
                      void run("删除", () => invoke<string>("mcp_remove_server", { name: s.name }));
                    }}
                  >
                    <Icon name="x" size={14} />
                    删除
                  </button>
                </div>
                <div className="pc-dir mono">
                  {s.command} {s.args.join(" ")}
                </div>
                {s.error ? (
                  <div className="pc-err">
                    <Icon name="shield-alert" size={14} />
                    {s.error}
                  </div>
                ) : s.tools.length === 0 ? (
                  <div className="muted">没有提供工具</div>
                ) : (
                  <div className="pc-contrib">
                    {s.tools.map((t) => (
                      <div className="pc-sec" key={t.name}>
                        <span className="badge accent">
                          mcp:{s.name}.{t.name}
                        </span>
                        <span className="pc-desc">{t.description}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* ---------- 高级：手动 / 导入 ---------- */}
      <div className="section">
        <div className="skill-toolbar">
          <button className="btn ghost sm" onClick={() => setAdvanced(advanced === "add" ? "none" : "add")}>
            <Icon name={advanced === "add" ? "x" : "plus"} size={14} />
            {advanced === "add" ? "取消" : "添加自定义 server"}
          </button>
          <button className="btn ghost sm" onClick={() => setAdvanced(advanced === "import" ? "none" : "import")}>
            <Icon name={advanced === "import" ? "x" : "download"} size={14} />
            {advanced === "import" ? "取消" : "从 JSON 导入"}
          </button>
        </div>

        {advanced === "add" && (
          <div className="skill-add plugin-add">
            <input className="input" placeholder="名称（如 my-fs）" value={form.name} spellCheck={false}
              onChange={(e) => setForm({ ...form, name: e.target.value })} />
            <input className="input" placeholder="command（如 npx）" value={form.command} spellCheck={false}
              onChange={(e) => setForm({ ...form, command: e.target.value })} />
            <textarea className="input" rows={3} spellCheck={false} placeholder={"args（每行一个）"}
              value={form.args} onChange={(e) => setForm({ ...form, args: e.target.value })} />
            <textarea className="input" rows={3} spellCheck={false} placeholder={"env（每行 KEY=VALUE）"}
              value={form.env} onChange={(e) => setForm({ ...form, env: e.target.value })} />
            <label className="tc-hint" style={{ display: "flex", gap: 4, alignItems: "center" }}>
              <input type="checkbox" checked={form.enabled} onChange={(e) => setForm({ ...form, enabled: e.target.checked })} />
              立即启用
            </label>
            <button className="btn primary" disabled={busy || !form.name.trim() || !form.command.trim()} onClick={() => void doSave()}>
              <Icon name="check" size={14} />
              保存
            </button>
          </div>
        )}

        {advanced === "import" && (
          <div className="card" style={{ marginTop: "var(--s-3)" }}>
            <div className="tc-hint" style={{ marginBottom: "var(--s-2)" }}>
              粘贴 JSON，兼容 Claude Desktop 的 <span className="mono">{"{\"mcpServers\":{…}}"}</span>、
              本项目的 <span className="mono">{"{\"servers\":{…}}"}</span>，或单个{" "}
              <span className="mono">{"{\"name\":…,\"command\":…}"}</span>。<b>只合并，不清空。</b>
            </div>
            <textarea className="input" rows={8} spellCheck={false} style={{ width: "100%" }}
              value={importText} onChange={(e) => setImportText(e.target.value)} />
            <div style={{ marginTop: "var(--s-2)" }}>
              <button className="btn primary" disabled={busy || !importText.trim()} onClick={() => void doImport()}>
                <Icon name="check" size={14} />
                导入
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
