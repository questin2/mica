// 技能页：**数据驱动、可增删**（拆分自 Shell.tsx 的 renderPage + 可扩展性报告 v2 · S-01/C）
//
// 与旧版的区别：技能不再是前端硬编码数组，而是走 Tauri 命令读写 `<data_dir>/skills.json`；
// 用户可「添加 / 删除 / 恢复内置」。会话初始化时引擎会把这份清单注入提示词，
// 因此这里加的技能，模型下一轮会话就能看到。
import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "../icons";

export type Skill = {
  name: string;
  desc: string;
  cat: string;
  /** 许可证来源；空串 = 未知 */
  licensed: string;
  /** 是否来自内置清单 */
  builtin: boolean;
  /** 来源标注；空串 = 本地技能库，非空如「插件：xxx」（只读，不能从本地库删除） */
  source: string;
};

/** 插件清单（对应后端 plugins::Plugin） */
export type PluginInfo = {
  dir: string;
  name: string;
  version: string;
  description: string;
  skill_count: number;
  /** 该插件贡献的工具数（M-01 工具抽象后的新扩展点） */
  tool_count: number;
  error: string | null;
};

type Props = {
  /** 技能数变化时回报给容器（用于左栏导航徽标） */
  onCount?: (n: number) => void;
};

/**
 * 技能来源徽标文案。
 *
 * 直接渲染裸的 `source`（如 `claude`）有两个问题：
 * ① 看不出这是"来源"还是"分类"；② 与相邻的「许可：未知」徽标都是暖色，视觉上糊成一块。
 * 因此：插件来源自带「插件：」前缀，原样显示；其余统一补「来源：」。
 */
export function sourceBadgeLabel(source: string): string {
  const s = source.trim();
  if (!s) return "";
  return s.startsWith("插件") ? s : `来源：${s}`;
}

export function SkillsPage({ onCount }: Props) {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [cat, setCat] = useState("全部");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState(false);
  const [form, setForm] = useState({ name: "", desc: "", cat: "" });
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const [pluginDir, setPluginDir] = useState("");

  const apply = useCallback(
    (list: Skill[]) => {
      setSkills(list);
      onCount?.(list.length);
    },
    [onCount],
  );

  const load = useCallback(async () => {
    try {
      apply(await invoke<Skill[]>("list_skills"));
      setErr("");
    } catch (e) {
      setErr(`读取技能库失败：${String(e)}`);
    }
    // 插件清单（失败不影响技能页主功能）
    try {
      setPlugins(await invoke<PluginInfo[]>("list_plugins"));
      setPluginDir(await invoke<string>("plugin_dir_info"));
    } catch {
      /* 忽略 */
    }
  }, [apply]);

  useEffect(() => {
    void load();
  }, [load]);

  /** 分类标签：由实际数据推导，自定义分类会自动出现 */
  const cats = useMemo(() => {
    const set = new Set<string>();
    skills.forEach((s) => set.add(s.cat));
    return ["全部", ...Array.from(set)];
  }, [skills]);

  const filtered = cat === "全部" ? skills : skills.filter((s) => s.cat === cat);

  const run = async (label: string, fn: () => Promise<Skill[]>) => {
    setBusy(true);
    try {
      apply(await fn());
      setErr("");
    } catch (e) {
      setErr(`${label}失败：${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const doAdd = () =>
    run("添加技能", async () => {
      const list = await invoke<Skill[]>("add_skill", {
        name: form.name,
        desc: form.desc,
        cat: form.cat,
      });
      setForm({ name: "", desc: "", cat: "" });
      setAdding(false);
      return list;
    });

  const doRemove = (name: string) => {
    if (!window.confirm(`删除技能「${name}」？\n（仅从技能库移除，不影响磁盘上的任何文件）`)) return;
    void run("删除技能", () => invoke<Skill[]>("remove_skill", { name }));
  };

  const doRestore = () => void run("恢复内置", () => invoke<Skill[]>("restore_builtin_skills"));

  const builtinCount = skills.filter((s) => s.builtin).length;

  return (
    <div className="page-inner">
      <h1 className="page-title">技能</h1>
      <p className="page-sub">
        共 <b>{skills.length}</b> 项（内置 {builtinCount} · 自建 {skills.length - builtinCount}）。
        技能库保存在 <span className="mono">app\data\skills.json</span>，
        <b>新增的技能会在下一次会话初始化时注入提示词</b>，模型即可使用。
      </p>

      {err && (
        <div className="banner warn" role="alert">
          <Icon name="info" size={16} />
          <span className="banner-text">{err}</span>
        </div>
      )}

      <div className="section">
        <div className="skill-toolbar">
          <button className="btn primary" disabled={busy} onClick={() => setAdding((v) => !v)}>
            <Icon name={adding ? "x" : "plus"} size={14} />
            {adding ? "取消" : "添加技能"}
          </button>
          <button className="btn ghost" disabled={busy} onClick={doRestore} title="把缺失的内置技能补回来（保留自建项）">
            <Icon name="refresh" size={14} />
            恢复内置
          </button>
          <span className="spacer" />
          <span className="tc-hint">{busy ? "处理中…" : "改动即时落盘"}</span>
        </div>

        {adding && (
          <div className="skill-add">
            <input
              className="input"
              placeholder="技能名（唯一，如 my-skill）"
              value={form.name}
              spellCheck={false}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
            <input
              className="input"
              placeholder="一句话描述（告诉模型这技能能干什么）"
              value={form.desc}
              spellCheck={false}
              onChange={(e) => setForm({ ...form, desc: e.target.value })}
            />
            <input
              className="input"
              placeholder="分类（留空 = 自定义）"
              value={form.cat}
              spellCheck={false}
              onChange={(e) => setForm({ ...form, cat: e.target.value })}
            />
            <button
              className="btn primary"
              disabled={busy || !form.name.trim() || !form.desc.trim()}
              onClick={doAdd}
            >
              <Icon name="check" size={14} />
              保存
            </button>
          </div>
        )}

        <div className="chip-row">
          {cats.map((c) => (
            <button key={c} className="chip" aria-pressed={cat === c} onClick={() => setCat(c)}>
              {c}
            </button>
          ))}
        </div>
      </div>

      {/* 插件状态：最小插件系统只支持「贡献技能」；工具类插件待 M-01 工具抽象 */}
      <div className="plugin-strip">
        <Icon name="zap" size={14} />
        <span>
          插件 {plugins.length} 个
          {plugins.length > 0 &&
            `（贡献技能 ${plugins.reduce((n, p) => n + p.skill_count, 0)} 项 · 工具 ${plugins.reduce(
              (n, p) => n + p.tool_count,
              0,
            )} 个）`}
          {plugins.some((p) => p.error) && ` · ${plugins.filter((p) => p.error).length} 个加载失败`}
        </span>
        <span className="spacer" />
        <span className="tc-hint" title={pluginDir}>
          把插件文件夹放入 <span className="mono">{pluginDir || "app\\data\\plugins"}</span>（每个含 plugin.json）
        </span>
      </div>
      {plugins
        .filter((p) => p.error)
        .map((p) => (
          <div className="banner warn" role="alert" key={p.dir}>
            <Icon name="info" size={16} />
            <span className="banner-text">
              <span className="mono">{p.dir}</span> —— {p.error}
            </span>
          </div>
        ))}

      {filtered.length === 0 ? (
        <p className="page-sub">
          {skills.length === 0 ? "技能库为空 —— 点「添加技能」或「恢复内置」开始。" : "该分类下没有技能。"}
        </p>
      ) : (
        <div className="skill-grid">
          {filtered.map((s) => (
            <div className="skill-card" key={s.name}>
              <div className="skill-card-head">
                <div className="skill-name">{s.name}</div>
                {!s.source && (
                  <button
                    className="btn ghost sm skill-del"
                    title={`删除「${s.name}」`}
                    aria-label={`删除技能 ${s.name}`}
                    disabled={busy}
                    onClick={() => doRemove(s.name)}
                  >
                    <Icon name="x" size={13} />
                  </button>
                )}
              </div>
              <div className="skill-desc">{s.desc}</div>
              <div className="skill-tags">
                <span className="badge">{s.cat}</span>
                {s.source ? (
                  <span className="badge source" title={`来源：${s.source}`}>
                    {sourceBadgeLabel(s.source)}
                  </span>
                ) : (
                  <span className={`badge ${s.builtin ? "" : "accent"}`}>{s.builtin ? "内置" : "自建"}</span>
                )}
                {s.licensed ? (
                  <span className="badge ok">许可：{s.licensed}</span>
                ) : (
                  <span className="badge warn">许可：未知</span>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
