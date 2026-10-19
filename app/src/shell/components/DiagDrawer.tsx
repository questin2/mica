// 引擎日志抽屉 —— 拆分自 Shell.tsx（S-01）
import type { LogRow } from "../types";

type Props = {
  logs: LogRow[];
};

export function DiagDrawer({ logs }: Props) {
  return (
    <div className="diag-drawer" aria-label="引擎日志">
      {logs.length === 0 && <div className="log-line muted">暂无引擎事件</div>}
      {logs.slice(-40).map((l, i) => (
        <div className="log-line" key={`${l.seq}-${i}`}>
          <span className="t">{l.t}</span> [{l.phase}] {l.detail}
        </div>
      ))}
    </div>
  );
}
