/**
 * 边缘羽化滚动容器 —— 移植自 open-claude-ui（MIT, Copyright (c) Caho1）
 * 来源: code-examples/ScrollFadeContainer.tsx（7 档非线性 Alpha 遮罩，28px 羽化）
 *
 * 原理: 以 y≈x^1.5 感知曲线的 7 组 alpha 采样做 mask-image，消除高分辨率屏上的
 *       渐变色阶断层；触顶/触底时对应侧羽化自动归零（边缘恢复全清晰）。
 *       兼容暗/浅主题（mask 只用 alpha 通道，与前景色无关）。
 */
import { useCallback, useEffect, useState, type HTMLAttributes, type ReactNode, type RefObject } from "react";

const STOPS: [number, string][] = [
  [0.15, "#0000000d"],
  [0.3, "#00000026"],
  [0.45, "#0000004d"],
  [0.6, "#00000080"],
  [0.75, "#000000b3"],
  [0.87, "#000000d9"],
];

function buildMask(): string {
  const top = STOPS.map(([k, a]) => `${a} calc(var(--fade-top) * ${k})`).join(",\n    ");
  const bottom = STOPS.map(([k, a]) => `${a} calc(100% - var(--fade-bottom) * ${k})`)
    .reverse()
    .join(",\n    ");
  return `linear-gradient(to bottom,
    transparent,
    ${top},
    #000 var(--fade-top),
    #000 calc(100% - var(--fade-bottom)),
    ${bottom},
    transparent)`;
}

const MASK = buildMask();

export function ScrollFade({
  children,
  className = "",
  containerRef,
  onScroll,
  fadeSize = 28,
  ...rest
}: {
  children: ReactNode;
  className?: string;
  containerRef: RefObject<HTMLDivElement | null>;
  onScroll?: () => void;
  fadeSize?: number;
} & Omit<HTMLAttributes<HTMLDivElement>, "style">) {
  const [top, setTop] = useState(0);
  const [bottom, setBottom] = useState(0);

  const update = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const atTop = el.scrollTop <= 2;
    const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 2;
    setTop(atTop ? 0 : fadeSize);
    setBottom(atBottom ? 0 : fadeSize);
    onScroll?.();
  }, [containerRef, fadeSize, onScroll]);

  // 尺寸变化与内容变化后重算（内容增长/窗口缩放）
  useEffect(() => {
    update();
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [update]);
  useEffect(update);

  return (
    <div
      {...rest}
      ref={containerRef}
      className={className}
      onScroll={update}
      style={
        {
          maskImage: MASK,
          WebkitMaskImage: MASK,
          "--fade-top": `${top}px`,
          "--fade-bottom": `${bottom}px`,
        } as React.CSSProperties
      }
    >
      {children}
    </div>
  );
}
