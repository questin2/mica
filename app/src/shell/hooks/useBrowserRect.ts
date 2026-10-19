// 右栏矩形同步：ResizeObserver → rAF → IPC —— 拆分自 Shell.tsx（S-01）
//
// 把 shell 侧给 DeepSeek WebView 预留的 slot 矩形同步给 Rust 引擎
// （引擎据此摆放 deepseek webview 的位置与尺寸）。
import { useCallback, useEffect, type RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";

type Opts = {
  slotRef: RefObject<HTMLDivElement | null>;
  /** 这些值变化会改变 slot 矩形 → 需要重新同步 */
  deps: unknown[];
};

export function useBrowserRect({ slotRef, deps }: Opts) {
  const pushRect = useCallback(() => {
    const el = slotRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    invoke("set_browser_rect", { x: r.x, y: r.y, width: r.width, height: r.height }).catch((e) =>
      console.error("set_browser_rect", e),
    );
  }, [slotRef]);

  // 观察 slot 尺寸变化（rAF 合并抖动）
  useEffect(() => {
    const el = slotRef.current;
    if (!el) return;
    let raf = 0;
    const schedule = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(pushRect);
    };
    const ro = new ResizeObserver(schedule);
    ro.observe(el);
    window.addEventListener("resize", schedule);
    const t0 = window.setTimeout(pushRect, 350);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", schedule);
      cancelAnimationFrame(raf);
      window.clearTimeout(t0);
    };
  }, [pushRect, slotRef]);

  // 折叠/展开、窄窗降级等会改变 slot 矩形 → 重新同步
  useEffect(() => {
    const t0 = window.setTimeout(pushRect, 80);
    return () => window.clearTimeout(t0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pushRect, ...deps]);

  return pushRect;
}
