/**
 * 滚动锚定（图钉）Hook —— 移植自 open-claude-ui（MIT, Copyright (c) Caho1）
 * 来源: code-examples/useClaudeScrollPinning.ts（Claude Desktop 逆向物理参数）
 *
 * 行为规范（保留原版物理常数）:
 *  - 150ms 用户输入冷却窗：刚滚轮/触摸过时不抢视口
 *  - 40px 底部边缘吸附容差：距底 ≤40px 视为「贴底」
 *  - 离开底部 >120px 显示「跳至最新」
 *  - behavior:"auto" 瞬时对齐（流式追加时防平滑动画排队抖动）
 */
import { useCallback, useEffect, useRef, useState, type RefObject } from "react";

export function useScrollPinning(deps: unknown[]): {
  containerRef: RefObject<HTMLDivElement | null>;
  isPinned: boolean;
  showJumpControl: boolean;
  markUserInput: () => void;
  handleScroll: () => void;
  scrollToBottom: (smooth?: boolean) => void;
} {
  const containerRef = useRef<HTMLDivElement>(null);
  const [isPinned, setIsPinned] = useState(true);
  const [showJumpControl, setShowJumpControl] = useState(false);
  const lastUserInputAt = useRef(0);
  const programmaticScroll = useRef(false);

  // 用户主动滚轮、触摸、键盘输入时立即释放锁定
  const markUserInput = useCallback(() => {
    lastUserInputAt.current = performance.now();
    setIsPinned(false);
  }, []);

  const handleScroll = useCallback(() => {
    if (programmaticScroll.current) {
      programmaticScroll.current = false;
      return;
    }
    const el = containerRef.current;
    if (!el) return;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    setIsPinned(distanceFromBottom <= 40);
    setShowJumpControl(distanceFromBottom > 120);
  }, []);

  const scrollToBottom = useCallback((smooth = false) => {
    const el = containerRef.current;
    if (!el) return;
    programmaticScroll.current = true;
    el.scrollTo({ top: el.scrollHeight, behavior: smooth ? "smooth" : "auto" });
    setIsPinned(true);
    setShowJumpControl(false);
  }, []);

  // 依赖项变化（新消息/流式追加）时执行图钉重锚定
  useEffect(() => {
    if (performance.now() - lastUserInputAt.current <= 150) return;
    if (isPinned) {
      scrollToBottom(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  return { containerRef, isPinned, showJumpControl, markUserInput, handleScroll, scrollToBottom };
}
