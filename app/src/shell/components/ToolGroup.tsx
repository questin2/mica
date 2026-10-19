// 已决策命令的「折叠汇总行 + 明细」—— 拆分自 Shell.tsx（可扩展性报告 v2 · S-01）
// 默认只占一行；点开看明细，单条明细可再展开命令全文。纯展示组件。
import { Icon } from "../icons";
import { classify, fmt, highlight, summarizeCode } from "../format";
import { SEND_LIMIT } from "../constants";
import type { DoneTool, Risk } from "../types";

type Props = {
  tools: DoneTool[];
  expanded: Set<number>;
  groupOpen: boolean;
  onToggleGroup: () => void;
  onToggleTool: (id: number) => void;
  onClear: () => void;
  onCopy: (text: string) => void;
};

export function ToolGroup({
  tools,
  expanded,
  groupOpen,
  onToggleGroup,
  onToggleTool,
  onClear,
  onCopy,
}: Props) {
  if (tools.length === 0) return null;

  const allowed = tools.filter((d) => d.allow).length;
  const denied = tools.length - allowed;
  const first = tools[0].ts;
  const last = tools[tools.length - 1].ts;
  const span = first === last ? first : `${first}–${last}`;

  return (
    <div className="toolgroup">
      <div className="tg-head">
        <button
          className="tg-toggle"
          onClick={onToggleGroup}
          aria-expanded={groupOpen}
          aria-label={`${groupOpen ? "收起" : "展开"}已决策的 ${tools.length} 条命令`}
        >
          <Icon name={groupOpen ? "chevron-down" : "chevron-right"} size={14} />
          <Icon name="terminal" size={14} />
          <span className="tg-title">已决策 {tools.length} 条命令</span>
          <span className="tg-stat">
            <span className="tg-ok">{allowed} 允许</span>
            {denied > 0 && <span className="tg-denied">{denied} 拒绝</span>}
          </span>
          <span className="tg-meta">{span}</span>
        </button>
        <button
          className="tg-clear"
          title="从对话流中清空这些执行记录（仍保留在「审计」页与磁盘日志）"
          aria-label="清空已决策命令记录"
          onClick={onClear}
        >
          <Icon name="x" size={14} />
        </button>
      </div>

      {groupOpen && (
        <div className="tg-body">
          {tools.map((d) => {
            const r: Risk = classify(d.code);
            const open = expanded.has(d.id);
            return (
              <div className={`toolrow ${d.allow ? "ok" : "denied"} risk-${r}`} key={d.id}>
                <button
                  className="tr-main"
                  onClick={() => onToggleTool(d.id)}
                  aria-expanded={open}
                  aria-label={`${open ? "收起" : "展开"}已${d.allow ? "执行" : "拒绝"}的命令 #${d.id}`}
                  title={summarizeCode(d.code)}
                >
                  <Icon name={open ? "chevron-down" : "chevron-right"} size={14} />
                  <Icon name="terminal" size={14} />
                  <span className={`tr-status ${d.allow ? "ok" : "danger"}`}>
                    {d.allow ? "已执行" : "已拒绝"}
                  </span>
                  <span className="tr-summary">{summarizeCode(d.code)}</span>
                  <span className="tr-meta">
                    #{d.id} · {fmt(d.code.length)} 字符 · {d.ts}
                  </span>
                </button>
                {open && (
                  <div className="tr-body">
                    <div className="code-head">
                      <Icon name="terminal" size={14} />
                      <span>命令全文</span>
                      <span className={`char-count ${d.code.length > SEND_LIMIT * 0.9 ? "warn" : ""}`}>
                        {fmt(d.code.length)} / 4,000 字符
                      </span>
                      <span className="spacer" />
                      <button className="btn ghost sm" onClick={() => onCopy(d.code)}>
                        <Icon name="copy" size={14} />
                        复制
                      </button>
                    </div>
                    <pre className="code-body" dangerouslySetInnerHTML={{ __html: highlight(d.code) }} />
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
