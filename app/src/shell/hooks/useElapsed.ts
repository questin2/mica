// 计时器（只显示，不做完成判定 —— 判定权在引擎）—— 拆分自 Shell.tsx（S-01）
//
// 暴露 begin/end 供回合执行方显式打点；phase 进入忙碌态时也会自动开始计时。
import { useCallback, useEffect, useState } from "react";
import { BUSY } from "../constants";

export function useElapsed(phase: string) {
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    if (startedAt == null) return;
    const id = window.setInterval(() => setElapsed(Date.now() - startedAt), 200);
    return () => window.clearInterval(id);
  }, [startedAt]);

  useEffect(() => {
    if (BUSY.includes(phase)) setStartedAt((prev) => prev ?? Date.now());
    else setStartedAt(null);
  }, [phase]);

  const begin = useCallback(() => setStartedAt(Date.now()), []);
  const end = useCallback(() => setStartedAt(null), []);

  return { elapsed, begin, end };
}
