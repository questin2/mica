// 左栏：注入入口 + 主导航 + 工作目录 / 本次运行 / 工具调用概览 —— 拆分自 Shell.tsx（S-01）
import { useState } from "react";
import { Icon } from "../icons";
import { baseName } from "../format";
import type { NavItem, View } from "../types";

type Props = {
  busy: boolean;
  view: View;
  navItems: NavItem[];
  workdir: string;
  turns: number;
  logCount: number;
  onRunSession: () => void;
  onSetView: (v: View) => void;
  onSetWorkdir: (v: string) => void;
  onCopy: (text: string) => void;
  onCycleTheme: () => void;
  /** 最近会话（切换用） */
  sessions: { id: string; title: string; updated_at: number }[];
  onOpenSession: (id: string) => void;
  /** 删除某个历史会话（不可恢复） */
  onDeleteSession: (id: string) => void;
};

export function LeftRail({
  busy,
  view,
  navItems,
  workdir,
  turns,
  logCount,
  onRunSession,
  onSetView,
  onCopy,
  onCycleTheme,
  sessions,
  onOpenSession,
  onDeleteSession,
}: Props) {
  // 复制反馈：点一下图标变对勾，1.2s 后复位（不改变复制行为本身）
  const [copied, setCopied] = useState(false);
  const doCopy = () => {
    onCopy(workdir);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  };
  return (
    <aside className="rail" aria-label="会话与导航">
      <div className="rail-top">
        <button className="btn primary block" disabled={busy} onClick={onRunSession} title="注入提示词">
          <Icon name="zap" size={16} />
          {/* 窄窗下这段文字会隐藏，按钮收成方形图标 —— 否则整宽按钮会把 rail 撑开 */}
          <span className="btn-text hide-on-collapse">注入提示词</span>
        </button>
      </div>

      <nav className="nav" aria-label="主导航">
        {navItems.map((n) => (
          <button
            className="nav-item"
            key={n[0]}
            /* 折叠后只剩图标，必须靠 title 提示名称 */
            title={n[3] != null ? `${n[2]}（${n[3]}）` : n[2]}
            aria-current={view === n[0] ? "page" : undefined}
            onClick={() => onSetView(n[0])}
          >
            <Icon name={n[1]} size={18} />
            <span className="nav-text">{n[2]}</span>
            {n[3] != null && <span className="nav-count">{n[3]}</span>}
          </button>
        ))}
      </nav>

      <div className="rail-mid">
        <div className="rail-label" style={{ display: "flex", alignItems: "center", gap: "var(--s-2)" }}>
          <span>工作目录</span>
          {/* 工作目录在左栏只展示这一处；复制完整路径的入口放在这里 */}
          <button
            className="btn ghost sm"
            style={{ marginLeft: "auto" }}
            onClick={doCopy}
            title={copied ? "已复制" : "复制完整路径"}
          >
            <Icon name={copied ? "check" : "copy"} size={12} />
            {copied ? "已复制" : "复制"}
          </button>
        </div>
        <p className="muted" style={{ font: "var(--t-meta)", marginTop: "var(--s-2)" }}>
          工作目录在对话区顶部设置，每个会话各自独立。
        </p>

        <div className="rail-label">本次运行</div>
        <button className="session" aria-current="true" onClick={() => onSetView("chat")}>
          <span className="session-title ellipsis">会话 · {baseName(workdir)}</span>
          <span className="session-meta">
            {turns} 个用户回合 · {logCount} 条引擎事件
          </span>
        </button>


      </div>

      {/* 最近会话：点一下切回去（后端 open_session 已存在，这里只是入口） */}
      {sessions.length > 0 && (
        <div className="rail-sessions">
          <div className="rail-label">最近会话</div>
          <div style={{ display: "grid", gap: 2 }}>
            {sessions.slice(0, 8).map((x) => (
              <div key={x.id} className="rail-session-row">
                <button
                  className="nav-item"
                  title={`${x.title || "(未命名)"} · 切换到此会话`}
                  onClick={() => onOpenSession(x.id)}
                  style={{ height: 28 }}
                >
                  <Icon name="history" size={14} />
                  <span className="nav-text ellipsis">{x.title || "(未命名)"}</span>
                </button>
                <button
                  className="btn ghost sm rail-session-del"
                  title="删除该会话（不可恢复）"
                  aria-label={`删除会话 ${x.title || "(未命名)"}`}
                  onClick={() => {
                    if (window.confirm(`删除会话「${x.title || "(未命名)"}」？此操作不可恢复。`)) {
                      onDeleteSession(x.id);
                    }
                  }}
                >
                  <Icon name="x" size={12} />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="rail-bottom">
        {/* 工作目录只在左栏上方出现一处（可编辑输入框），这里不再重复展示只读副本；
            复制完整路径改由上方输入框的 title 提示承担。 */}
        <div className="rail-meta">
          <span>v0.2.0</span>
          <button className="btn ghost sm" onClick={onCycleTheme}>
            主题
          </button>
        </div>
      </div>
    </aside>
  );
}
