// 窄窗降级：窗口过窄时自动折叠浏览器面板，变宽后自动恢复 —— 拆分自 Shell.tsx（S-01）
import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { Toast } from "../types";

export function useNarrow(
  setBrowserCollapsed: Dispatch<SetStateAction<boolean>>,
  pushToast: (kind: Toast["kind"], text: string) => void,
) {
  const [narrow, setNarrow] = useState(false);
  // 记录「是不是自动折叠的」，避免和用户手动开关互相打架
  const autoCollapsed = useRef(false);

  useEffect(() => {
    const on = () => {
      const w = window.innerWidth;
      setNarrow(w < 1200);
      if (w < 1120 && !autoCollapsed.current) {
        autoCollapsed.current = true;
        setBrowserCollapsed(true);
        pushToast("info", "窗口较窄，已折叠浏览器面板（Ctrl+J 展开）");
      } else if (w >= 1240 && autoCollapsed.current) {
        autoCollapsed.current = false;
        setBrowserCollapsed(false);
      }
    };
    on();
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, [pushToast, setBrowserCollapsed]);

  return narrow;
}
