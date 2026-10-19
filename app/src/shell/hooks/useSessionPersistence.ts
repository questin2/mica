// 会话持久化（S-02）：启动恢复 + 变更防抖落盘 —— 拆分自 Shell.tsx（S-01）
//
// 恢复用 state（而非 ref）标记完成，以便恢复完成时能触发一次落盘 ——
// 新用户也能立刻看到存档文件生成。
import { useEffect, useState, type Dispatch, type SetStateAction } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DoneTool, Msg } from "../types";

type Opts = {
  workdir: string;
  messages: Msg[];
  doneTools: DoneTool[];
  setWorkdir: Dispatch<SetStateAction<string>>;
  setMessages: Dispatch<SetStateAction<Msg[]>>;
  setDoneTools: Dispatch<SetStateAction<DoneTool[]>>;
  /** 恢复成功后回调（用于提示用户恢复了多少条） */
  onRestored: (count: number) => void;
};

export function useSessionPersistence({
  workdir,
  messages,
  doneTools,
  setWorkdir,
  setMessages,
  setDoneTools,
  onRestored,
}: Opts) {
  const [restored, setRestored] = useState(false);

  // 启动恢复
  useEffect(() => {
    invoke<string>("load_session")
      .then((raw) => {
        if (!raw) return;
        const s = JSON.parse(raw) as {
          workdir?: string;
          messages?: Msg[];
          doneTools?: DoneTool[];
        };
        if (typeof s.workdir === "string" && s.workdir) setWorkdir(s.workdir);
        if (Array.isArray(s.messages)) setMessages(s.messages);
        if (Array.isArray(s.doneTools)) setDoneTools(s.doneTools);
        const n = s.messages?.length ?? 0;
        if (n > 0) onRestored(n);
      })
      .catch(() => {
        /* 无存档或损坏 → 走初始状态 */
      })
      .finally(() => setRestored(true));
    // onRestored 由容器用 useCallback 保证稳定；此处只关心首次挂载
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 变更后延迟落盘：800ms 防抖，避免每来一条事件都写盘
  useEffect(() => {
    if (!restored) return;
    const t = window.setTimeout(() => {
      void invoke("save_session", {
        state: JSON.stringify({
          version: 1,
          savedAt: new Date().toISOString(),
          workdir,
          messages,
          doneTools,
        }),
      }).catch(() => {});
    }, 800);
    return () => window.clearTimeout(t);
  }, [restored, workdir, messages, doneTools]);
}
