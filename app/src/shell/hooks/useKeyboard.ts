// 全局键盘处理 —— 拆分自 Shell.tsx（可扩展性报告 v2 · S-01）
//
// 依赖全部通过参数传入；内部用 ref 持有最新的处理函数，因此监听器只注册一次，
// 不会因为闭包过期而用到旧的 state（原实现就是这套模式，行为逐字保留）。
import { useEffect, useRef, type Dispatch, type SetStateAction } from "react";
import type { ApprovalOption } from "../components/ApprovalCard";
import type { ToolReq } from "../types";

export type KeyboardOpts = {
  toolReq: ToolReq | null;
  approvalOptions: ApprovalOption[];
  approvalIdx: number;
  setApprovalIdx: Dispatch<SetStateAction<number>>;
  onDecide: (allow: boolean) => void;
  onChooseApproval: (key: ApprovalOption["key"]) => void;
  onEstop: () => void;
  onSelfcheck: () => void;
  onSend: () => void;
  onClearInput: () => void;
  toggleRail: () => void;
  toggleBrowser: () => void;
  enterSend: boolean;
};

export function useKeyboard(o: KeyboardOpts) {
  const ref = useRef<(e: KeyboardEvent) => void>(() => {});

  ref.current = (e: KeyboardEvent) => {
    // CJK 输入法合成守卫（采编自 open-claude-ui / Claude 官方断言）：
    // 合成中的 Enter（选词确认）与 keyCode 229 一律放行，绝不触发发送或快捷键
    if (e.isComposing || e.keyCode === 229) return;
    const mod = e.ctrlKey || e.metaKey;
    const inComposer = document.activeElement === document.getElementById("composer-input");

    if (e.key === "Escape") {
      if (o.toolReq) {
        e.preventDefault();
        o.onDecide(false);
      }
      return;
    }

    // 审批中：↑↓ 选择 · Enter 确认 · 数字键直选（DSH/Claude 式交互）
    if (o.toolReq && !mod) {
      const n = o.approvalOptions.length;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const d = e.key === "ArrowDown" ? 1 : -1;
        o.setApprovalIdx((i) => (i + d + n) % n);
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const opt = o.approvalOptions[o.approvalIdx];
        if (opt) o.onChooseApproval(opt.key);
        return;
      }
      if (/^[1-9]$/.test(e.key)) {
        const opt = o.approvalOptions[Number(e.key) - 1];
        if (opt) {
          e.preventDefault();
          o.onChooseApproval(opt.key);
          return;
        }
      }
    }

    if (mod && e.key.toLowerCase() === "b") {
      e.preventDefault();
      o.toggleRail();
      return;
    }
    if (mod && e.key.toLowerCase() === "j") {
      e.preventDefault();
      o.toggleBrowser();
      return;
    }
    if (mod && e.key === ".") {
      e.preventDefault();
      o.onEstop();
      return;
    }
    if (mod && e.shiftKey && e.key.toLowerCase() === "t") {
      e.preventDefault();
      o.onSelfcheck();
      return;
    }
    if (mod && e.key.toLowerCase() === "n") {
      e.preventDefault();
      o.onClearInput();
      return;
    }
    if (e.key === "Enter" && inComposer && !e.shiftKey) {
      if (mod) {
        e.preventDefault();
        o.onSend();
        return;
      }
      if (o.enterSend) {
        e.preventDefault();
        o.onSend();
      }
    }
  };

  useEffect(() => {
    const h = (e: KeyboardEvent) => ref.current(e);
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, []);
}
