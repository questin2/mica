// 外观：主题 / 密度 / 动效 / 玻璃 —— 拆分自 Shell.tsx（S-01）
//
// 状态自带 localStorage 持久化，并把结果写到 <html> 的 data-* 属性上（CSS 据此切换）。
import { useEffect, useState } from "react";
import { lsGet, lsSet } from "../format";
import type { MotionMode, ThemeMode } from "../types";

export function useAppearance() {
  const [theme, setTheme] = useState<ThemeMode>((lsGet("theme", "dark") as ThemeMode) || "dark");
  const [density, setDensity] = useState(lsGet("density", "cozy"));
  const [motion, setMotion] = useState<MotionMode>(() => {
    const v = lsGet("motion", "full");
    // 旧值兼容：曾用 normal 表示"完整"
    return (v === "normal" ? "full" : v) as MotionMode;
  });
  const [glass, setGlass] = useState(lsGet("glass", "1"));
  const [sysLight, setSysLight] = useState(false);

  // 跟随系统深浅色
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const sync = () => setSysLight(mq.matches);
    sync();
    mq.addEventListener("change", sync);
    return () => mq.removeEventListener("change", sync);
  }, []);

  // 窗口不可见 / 失焦时暂停所有循环动画 —— 否则 WebView2 在后台白白耗电。
  // 只切一个属性，循环动画统一读 --motion-play，因此不需要逐个动画处理。
  useEffect(() => {
    const sync = () => {
      const paused = document.visibilityState === "hidden" || !document.hasFocus();
      document.documentElement.setAttribute("data-window", paused ? "blurred" : "focused");
    };
    sync();
    document.addEventListener("visibilitychange", sync);
    window.addEventListener("focus", sync);
    window.addEventListener("blur", sync);
    return () => {
      document.removeEventListener("visibilitychange", sync);
      window.removeEventListener("focus", sync);
      window.removeEventListener("blur", sync);
    };
  }, []);

  // 应用到 DOM + 落盘
  useEffect(() => {
    const resolved = theme === "system" ? (sysLight ? "light" : "dark") : theme;
    const root = document.documentElement;
    root.setAttribute("data-theme", resolved);
    root.setAttribute("data-theme-mode", theme);
    root.setAttribute("data-density", density);
    root.setAttribute("data-motion", motion);
    root.setAttribute("data-glass", glass);
    lsSet("theme", theme);
    lsSet("density", density);
    lsSet("motion", motion);
    lsSet("glass", glass);
  }, [theme, density, motion, glass, sysLight]);

  return { theme, setTheme, density, setDensity, motion, setMotion, glass, setGlass };
}
