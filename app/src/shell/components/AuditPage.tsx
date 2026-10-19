// 审计页：引擎事件时间线 + 磁盘审批留痕 —— 拆分自 Shell.tsx（可扩展性报告 v2 · S-01）
//
// 两块内容的分工：
//   - **内存时间线**：本次运行的所有引擎事件（含 phase 过滤）；
//   - **磁盘审计尾部**：`audit.log` 的结尾若干行，重点是**审批来源留痕**
//     （人工允许 / 会话信任自动放行 / 信任集变更）—— 这是事后追责的唯一依据。
// 只读操作，不改动任何状态。
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "../icons";
import type { LogRow } from "../types";

type Props = {
  logs: LogRow[];
  /** 最近一次通道自检的原始输出（空串则不显示该区块） */
  channelRaw: string;
  onCopy: (text: string) => void;
};

const FILTERS = ["全部", "回合", "工具", "审批", "结果"];

/** 按筛选档位裁剪日志行。 */
function filterLogs(logs: LogRow[], f: string): LogRow[] {
  if (f === "全部") return logs;
  if (f === "回合") return logs.filter((l) => ["injecting", "generating", "finalizing", "done"].includes(l.phase));
  if (f === "工具") return logs.filter((l) => l.phase === "tool");
  if (f === "审批") return logs.filter((l) => l.phase === "tool" && /审批|允许|拒绝/.test(l.detail));
  return logs.filter((l) => l.phase === "error" || l.phase === "timeout");
}

/** 审计留痕的特征行：只有前端回报的「审批来源」与「信任集变更」带这些前缀。 */
const PROVENANCE_RE = /审批来源|信任集变更/;

/**
 * 从磁盘审计尾部里挑出**审批来源留痕**。
 *
 * 为什么需要它：`audit.log` 是混合内容（命令、执行结果、留痕都写在同一文件），
 * 实测已达 3000+ 行、160KB+，直接看尾部很难找到关键的那几行。
 */
export function pickProvenance(tail: string): string {
  return tail
    .split("\n")
    .filter((l) => PROVENANCE_RE.test(l))
    .join("\n")
    .trim();
}

export function AuditPage({ logs, channelRaw, onCopy }: Props) {
  const [filter, setFilter] = useState("全部");
  const rows = filterLogs(logs, filter);

  /* 磁盘审计尾部：重点是审批来源留痕（人工 / 自动放行 / 信任集变更） */
  const [diskLog, setDiskLog] = useState("");
  const [logPath, setLogPath] = useState("");
  const [loading, setLoading] = useState(false);
  const [onlyProv, setOnlyProv] = useState(true);

  const loadDisk = useCallback(async () => {
    setLoading(true);
    try {
      setDiskLog(await invoke<string>("read_audit_log"));
      setLogPath(await invoke<string>("audit_log_path_info"));
    } catch {
      /* 读不到就留空，不影响内存时间线 */
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadDisk();
  }, [loadDisk]);

  const provenance = pickProvenance(diskLog);
  const shown = onlyProv ? provenance : diskLog;

  return (
    <div className="page-inner">
      <h1 className="page-title">审计</h1>
      <p className="page-sub">
        引擎事件时间线（seq / 阶段 / 明细）。工具决策与执行同时落盘于 <span className="mono">app\data\audit.log</span>；
        本页为内存视图，保留 {logs.length} 条。
      </p>
      <div className="section">
        <div className="chip-row">
          {FILTERS.map((f) => (
            <button key={f} className="chip" aria-pressed={filter === f} onClick={() => setFilter(f)}>
              {f}
            </button>
          ))}
          <button
            className="btn sm ml-auto"
            onClick={() => onCopy(logs.map((l) => `${l.t} [${l.phase}] ${l.detail}`).join("\n"))}
          >
            <Icon name="download" size={14} />
            复制全部（诊断用）
          </button>
        </div>
      </div>

      {/* 磁盘留痕：审批来源（人工 / 会话信任自动放行 / 信任集变更）唯一的追责依据 */}
      <div className="section">
        <h2 className="section-title">
          磁盘审计日志
          <button className="chip" aria-pressed={onlyProv} onClick={() => setOnlyProv((v) => !v)}>
            {onlyProv ? "只看审批来源" : "显示全部内容"}
          </button>
          <span className="spacer" />
          <button className="btn ghost sm" disabled={loading} onClick={() => void loadDisk()}>
            <Icon name="refresh" size={14} />
            {loading ? "读取中…" : "刷新"}
          </button>
          <button className="btn ghost sm" disabled={!shown} onClick={() => onCopy(shown)}>
            <Icon name="copy" size={14} />
            复制
          </button>
        </h2>
        <p className="page-sub">
          日志：<span className="mono">{logPath || "app\\data\\audit.log"}</span>
          {" · "}
          该文件是混合内容（命令 / 执行结果 / 留痕），故默认只挑出审批留痕；
          <b>自动放行会写明「会话信任自动放行」与「非人工逐条确认」</b>，与人工确认可区分。
          {onlyProv && provenance === "" && diskLog !== "" && "（尾部窗口中暂无留痕行，可切「显示全部内容」或点刷新）"}
        </p>
        <div className="card flush">
          <pre className="code-body audit-tail" aria-label="磁盘审计日志尾部">
            {shown || "（暂无内容 —— 触发一次审批后这里会出现留痕）"}
          </pre>
        </div>
      </div>
      {channelRaw && (
        <div className="section">
          <h2 className="section-title">最近一次通道自检原始输出</h2>
          <div className="card flush">
            <pre className="code-body" style={{ maxHeight: 200 }}>
              {channelRaw}
            </pre>
          </div>
        </div>
      )}
      <div className="card flush">
        <table className="table">
          <thead>
            <tr>
              <th className="tabular">时间</th>
              <th>阶段</th>
              <th>明细</th>
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr>
                <td colSpan={3} className="muted">
                  暂无记录 —— 触发一次回合或自检后这里会出现时间线。
                </td>
              </tr>
            )}
            {rows
              .slice()
              .reverse()
              .map((l, i) => (
                <tr key={`${l.seq}-${i}`}>
                  <td className="tabular muted">{l.t}</td>
                  <td>
                    <span
                      className={`badge ${l.phase === "error" ? "danger" : l.phase === "tool" ? "accent" : l.phase === "timeout" ? "warn" : ""}`}
                    >
                      {l.phase}
                    </span>
                  </td>
                  <td>{l.detail}</td>
                </tr>
              ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
