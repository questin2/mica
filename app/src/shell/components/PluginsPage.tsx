// 「插件」页：插件能力的**管理入口**（阶段2-29）
//
// 此前插件能力只存在于后端，界面只有技能页上一行“插件 N 个”的计数 —— 用户既看不到
// 插件贡献了什么，也没有任何创建/删除入口，等于"能力不存在"。本页把它摊开：
//   列表（含贡献明细与加载错误）· 新建（可附示例技能/工具）· 删除 · 打开插件目录
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "../icons";

export type PluginSkill = { name: string; desc: string; cat: string };
export type PluginTool = { name: string; title: string; description: string; command: string };

export type PluginCatalogEntry = {
  id: string;
  name: string;
  description: string;
  category: string;
  tool_count: number;
  installed: boolean;
  enabled: boolean;
};

export type PluginDetail = {
  dir: string;
  name: string;
  version: string;
  description: string;
  skills: PluginSkill[];
  tools: PluginTool[];
  error: string | null;
};

type Props = {
  /** 插件集合变化时回报插件数（技能库/工具注册表会跟着变，容器可据此刷新徽标） */
  onChanged?: (count: number) => void;
};

export function PluginsPage({ onChanged }: Props) {
  const [list, setList] = useState<PluginDetail[]>([]);
  const [catalog, setCatalog] = useState<PluginCatalogEntry[]>([]);
  const [dir, setDir] = useState("");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState(false);
  const [form, setForm] = useState({ name: "", version: "1.0.0", description: "", skill: true, tool: true });

  const load = useCallback(async () => {
    try {
      const [d, p, c] = await Promise.all([
        invoke<PluginDetail[]>("plugin_details"),
        invoke<string>("plugin_dir_info"),
        invoke<PluginCatalogEntry[]>("plugin_catalog"),
      ]);
      setList(d);
      setDir(p);
      setCatalog(c);
      setErr("");
      onChanged?.(d.length);
    } catch (e) {
      setErr(`读取插件失败：${String(e)}`);
    }
  }, [onChanged]);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (label: string, fn: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await fn();
      await load();
      setErr("");
    } catch (e) {
      setErr(`${label}失败：${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const togglePreset = (c: PluginCatalogEntry) =>
    run(c.enabled ? "停用" : "启用", () =>
      invoke<string>("plugin_set_catalog", { id: c.id, enabled: !c.enabled }),
    );

  const doCreate = () =>
    run("新建插件", async () => {
      await invoke<string>("create_plugin", {
        name: form.name,
        version: form.version,
        description: form.description,
        withSkill: form.skill,
        withTool: form.tool,
      });
      setForm({ name: "", version: "1.0.0", description: "", skill: true, tool: true });
      setAdding(false);
    });

  const doDelete = (p: PluginDetail) => {
    const label = p.name || p.dir.split(/[\\/]/).pop() || p.dir;
    if (!window.confirm(`删除插件「${label}」？\n\n会删除整个插件目录（含其 plugin.json），不可撤销。`)) return;
    void run("删除插件", () => invoke("delete_plugin", { dir: p.dir }));
  };

  const skillTotal = list.reduce((n, p) => n + p.skills.length, 0);
  const toolTotal = list.reduce((n, p) => n + p.tools.length, 0);
  const badCount = list.filter((p) => p.error).length;

  return (
    <div className="page-inner">
      <h1 className="page-title">插件</h1>
      <p className="page-sub">
        共 <b>{list.length}</b> 个插件（贡献技能 {skillTotal} 项 · 工具 {toolTotal} 个）
        {badCount > 0 && ` ｜ ${badCount} 个加载失败`}。
        插件可以<b>贡献技能</b>（进入技能库并注入提示词）与<b>贡献工具</b>（进入工具注册表，
        与 PowerShell 工具走同一道审批门）。
      </p>

      {err && (
        <div className="banner warn" role="alert">
          <Icon name="info" size={16} />
          <span className="banner-text">{err}</span>
        </div>
      )}

      {/* ---------- 能力库：勾选即用（与 MCP 页同一套交互）---------- */}
      <div className="section">
        <h2 className="section-title">能力库</h2>
        <div className="plugin-list">
          {catalog.map((c) => (
            <label className={`plugin-card catalog-row${c.enabled ? " on" : ""}`} key={c.id}>
              <div className="pc-head">
                <div className="pc-title">
                  <Icon name="sparkle" size={16} />
                  <b>{c.name}</b>
                  <span className="badge">{c.category}</span>
                  <span className="badge accent">{c.tool_count} 个工具</span>
                </div>
                <input
                  type="checkbox"
                  className="catalog-toggle"
                  checked={c.enabled}
                  disabled={busy}
                  onChange={() => togglePreset(c)}
                />
              </div>
              <div className="pc-desc">{c.description}</div>
            </label>
          ))}
        </div>
      </div>

      <div className="section">
        <div className="skill-toolbar">
          <button className="btn primary" disabled={busy} onClick={() => setAdding((v) => !v)}>
            <Icon name={adding ? "x" : "plus"} size={14} />
            {adding ? "取消" : "新建插件"}
          </button>
          <button className="btn ghost" disabled={busy} onClick={() => void run("打开目录", () => invoke("open_plugins_dir"))}>
            <Icon name="folder" size={14} />
            打开插件目录
          </button>
          <button className="btn ghost" disabled={busy} onClick={() => void load()}>
            <Icon name="refresh" size={14} />
            刷新
          </button>
          <span className="spacer" />
          <span className="tc-hint" title={dir}>
            {dir || "app\\data\\plugins"}
          </span>
        </div>

        {adding && (
          <div className="skill-add plugin-add">
            <input
              className="input"
              placeholder="插件名（如 my-plugin）"
              value={form.name}
              spellCheck={false}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
            <input
              className="input"
              placeholder="版本（默认 1.0.0）"
              value={form.version}
              spellCheck={false}
              onChange={(e) => setForm({ ...form, version: e.target.value })}
            />
            <input
              className="input"
              placeholder="一句话描述"
              value={form.description}
              spellCheck={false}
              onChange={(e) => setForm({ ...form, description: e.target.value })}
            />
            <label className="tc-hint" style={{ display: "flex", gap: 4, alignItems: "center" }}>
              <input type="checkbox" checked={form.skill} onChange={(e) => setForm({ ...form, skill: e.target.checked })} />
              附示例技能
            </label>
            <label className="tc-hint" style={{ display: "flex", gap: 4, alignItems: "center" }}>
              <input type="checkbox" checked={form.tool} onChange={(e) => setForm({ ...form, tool: e.target.checked })} />
              附示例工具
            </label>
            <button className="btn primary" disabled={busy || !form.name.trim()} onClick={doCreate}>
              <Icon name="check" size={14} />
              创建
            </button>
          </div>
        )}
      </div>

      {list.length === 0 ? (
        <p className="page-sub">
          还没有插件。点「新建插件」快速创建一个（勾选示例技能/工具后立刻能在技能库与工具注册表里看到），
          或点「打开插件目录」把自己写的插件文件夹放进去。
        </p>
      ) : (
        <div className="plugin-list">
          {list.map((p) => (
            <div className={`plugin-card${p.error ? " has-error" : ""}`} key={p.dir}>
              <div className="pc-head">
                <div className="pc-title">
                  <Icon name="plug" size={16} />
                  <b>{p.name || "(未命名)"}</b>
                  {p.version && <span className="badge">{p.version}</span>}
                  {p.error && <span className="badge danger">加载失败</span>}
                </div>
                <button
                  className="btn ghost sm"
                  title="删除此插件（会删除整个插件目录）"
                  aria-label={`删除插件 ${p.name || p.dir}`}
                  disabled={busy}
                  onClick={() => doDelete(p)}
                >
                  <Icon name="x" size={14} />
                  删除
                </button>
              </div>

              {p.description && <div className="pc-desc">{p.description}</div>}
              <div className="pc-dir mono" title={p.dir}>
                {p.dir}
              </div>

              {p.error ? (
                <div className="pc-err">
                  <Icon name="shield-alert" size={14} />
                  {p.error}
                </div>
              ) : (
                <div className="pc-contrib">
                  <div className="pc-sec">
                    <span className="pc-label">技能 {p.skills.length}</span>
                    {p.skills.length === 0 ? (
                      <span className="muted">—</span>
                    ) : (
                      <span className="pc-tags">
                        {p.skills.map((s) => (
                          <span className="badge" key={s.name} title={s.desc}>
                            {s.name}
                            {s.cat ? ` · ${s.cat}` : ""}
                          </span>
                        ))}
                      </span>
                    )}
                  </div>
                  <div className="pc-sec">
                    <span className="pc-label">工具 {p.tools.length}</span>
                    {p.tools.length === 0 ? (
                      <span className="muted">—</span>
                    ) : (
                      <span className="pc-tags">
                        {p.tools.map((t) => (
                          <span className="badge accent" key={t.name} title={`${t.description}\n\n${t.command}`}>
                            {t.name}
                            {t.title ? ` · ${t.title}` : ""}
                          </span>
                        ))}
                      </span>
                    )}
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
