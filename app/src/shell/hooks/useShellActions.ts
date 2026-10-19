// Shell 的动作层：回合执行、自检、审批决策、急停、复制、工具展开 —— 拆分自 Shell.tsx（S-01）
//
// 这是「UI 意图 → 引擎 IPC」的唯一出口。所有 state 由容器传入，本 Hook 只返回处理器。
import { useCallback, type Dispatch, type RefObject, type SetStateAction } from "react";
import { invoke } from "@tauri-apps/api/core";
import { classify, hm } from "../format";
import { auditManualDecision, logAudit } from "../audit";
import type { DoneTool, LogRow, Msg, Step, Toast, ToolReq } from "../types";

export type ShellActionsOpts = {
  workdir: string;
  msg: string;
  setMsg: Dispatch<SetStateAction<string>>;
  setBusy: Dispatch<SetStateAction<boolean>>;
  setMessages: Dispatch<SetStateAction<Msg[]>>;
  setLogs: Dispatch<SetStateAction<LogRow[]>>;
  setChannelRaw: Dispatch<SetStateAction<string>>;
  setChannel: Dispatch<SetStateAction<{ text: string; ok: boolean | null }>>;
  toolReq: ToolReq | null;
  setToolReq: Dispatch<SetStateAction<ToolReq | null>>;
  setDoneTools: Dispatch<SetStateAction<DoneTool[]>>;
  /** 当前回合的运行步骤：回复落库时一并带上（一个对话一组步骤） */
  stepsRef: RefObject<Step[]>;
  /** 新一轮开始时清空步骤缓冲 */
  setSteps: Dispatch<SetStateAction<Step[]>>;
  setExpandedTools: Dispatch<SetStateAction<Set<number>>>;
  /** 递增的消息 id 源 */
  seq: { seq: number };
  beginTimer: () => void;
  endTimer: () => void;
  pushToast: (kind: Toast["kind"], text: string) => void;
};

export function useShellActions(o: ShellActionsOpts) {
  const {
    workdir,
    msg,
    setMsg,
    setBusy,
    setMessages,
    setLogs,
    setChannelRaw,
    setChannel,
    toolReq,
    setToolReq,
    setDoneTools,
    stepsRef,
    setSteps,
    setExpandedTools,
    seq,
    beginTimer,
    endTimer,
    pushToast,
  } = o;

  /** 统一包裹：置忙 → 执行 → 把回复/错误写进消息流。 */
  const runGuarded = useCallback(
    async (label: string, fn: () => Promise<string>) => {
      setBusy(true);
      beginTimer();
      try {
        const text = await fn();
        setMessages((m) => [...m, { id: seq.seq++, role: "assistant", text: text || "（空回复）", ts: hm(), steps: stepsRef.current }]);
      } catch (e) {
        const text = `ERR: ${String(e)}`;
        setMessages((m) => [...m, { id: seq.seq++, role: "assistant", text, ts: hm(), steps: stepsRef.current }]);
        setLogs((l) => [...l.slice(-399), { seq: -1, t: hm(), phase: "error", detail: `${label}：${text}` }]);
      } finally {
        setBusy(false);
        endTimer();
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [seq],
  );

  const runSession = useCallback(() => {
    pushToast("info", "注入系统提示词（约 8k 字）…");
    return runGuarded("注入提示词", () => invoke<string>("start_agent_session", { workdir }));
  }, [pushToast, runGuarded, workdir]);

  const runTurn = useCallback(
    (text?: string) => {
      const body = (text ?? msg).trim();
      if (!body) {
        pushToast("warn", "先输入要交给 Agent 的指令");
        return;
      }
      setMsg("");
      setMessages((m) => [...m, { id: seq.seq++, role: "user", text: body, ts: hm() }]);
    // 新一轮开始：清空步骤缓冲，保证"一个对话一组步骤"
    setSteps([]);
      return runGuarded("发送指令", () => invoke<string>("agent_turn", { workdir, message: body }));
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [msg, pushToast, runGuarded, seq, workdir, setMsg, setMessages],
  );

  const runBareRound = useCallback(() => {
    const body = msg.trim() || "请只回复两个字母：OK";
    pushToast("info", "裸回合（不经 Agent 循环，诊断用）");
    return runGuarded("测试回合", () => invoke<string>("test_roundtrip", { message: body }));
  }, [msg, pushToast, runGuarded]);

  const runSelfcheck = useCallback(async () => {
    pushToast("info", "正在运行通道自检（只读，查询选择器命中）…");
    try {
      const raw = await invoke<string>("selfcheck");
      setChannelRaw(raw);
      let ok: boolean | null = null;
      let unhealthyList: string[] = [];
      try {
        const parsed: unknown = JSON.parse(raw);
        if (parsed && typeof parsed === "object" && "allHealthy" in parsed) {
          // 新格式：结构化 JSON {groups, unhealthy, allHealthy}
          const obj = parsed as { allHealthy: boolean; unhealthy?: string[] };
          ok = obj.allHealthy;
          unhealthyList = Array.isArray(obj.unhealthy) ? obj.unhealthy : [];
        } else if (Array.isArray(parsed)) {
          // 兼容旧格式：字符串数组 ["group | selector => count", ...]
          const counts = parsed
            .map((x) => (typeof x === "string" ? Number(x.split("=>").pop()?.trim()) : NaN))
            .filter((n) => !Number.isNaN(n));
          if (counts.length) ok = counts.every((n) => n > 0);
        }
      } catch {
        ok = null;
      }
      const text =
        ok === true
          ? "就绪"
          : ok === false
            ? `选择器告警：${unhealthyList.length > 0 ? unhealthyList.join(", ") : "有 0 命中组"}`
            : "已自检";
      setChannel({ text, ok });
      setLogs((l) => [...l.slice(-399), { seq: -1, t: hm(), phase: "selfcheck", detail: raw.slice(0, 400) }]);
      if (ok === false) {
        pushToast("warn", `通道自检：${unhealthyList.length > 0 ? unhealthyList.join("、") : "部分选择器"} 0 命中，回合可能返回空回复`);
      } else {
        pushToast("ok", "通道自检完成");
      }
    } catch (e) {
      setChannel({ text: `自检失败：${String(e)}`, ok: false });
      pushToast("danger", `通道自检失败：${String(e)}`);
    }
  }, [pushToast, setChannel, setChannelRaw, setLogs]);

  const decideTool = useCallback(
    async (allow: boolean) => {
      const req = toolReq;
      if (!req) return;
      // 【审计】人工决策留痕（与「会话信任自动放行」在 audit.log 里可区分）
      auditManualDecision(req.id, allow, classify(req.code));
      setToolReq(null);
      setDoneTools((d) => [...d, { id: req.id, code: req.code, allow, ts: hm() }]);
      setLogs((l) => [
        ...l.slice(-399),
        { seq: -1, t: hm(), phase: "tool", detail: `${allow ? "已允许" : "已拒绝"} #${req.id}` },
      ]);
      try {
        await invoke("tool_decision", { id: req.id, allow });
      } catch (e) {
        pushToast("danger", `决策下发失败：${String(e)}`);
      }
    },
    [pushToast, toolReq, setToolReq, setDoneTools, setLogs],
  );

  const estop = useCallback(() => {
    if (toolReq) {
      logAudit("审计来源：急停（Ctrl+.）—— 待审批工具按拒绝处理");
      void decideTool(false);
    }
    setBusy(false);
    endTimer();
    pushToast("danger", "急停：已停止本轮 UI、并按拒绝处理待审批工具（引擎侧终止子进程树随阶段 3 落地）");
  }, [decideTool, pushToast, toolReq, setBusy, endTimer]);

  const copy = useCallback(
    (text: string) => {
      if (!navigator.clipboard) {
        pushToast("warn", "剪贴板不可用");
        return;
      }
      navigator.clipboard
        .writeText(text)
        .then(() => pushToast("ok", "已复制"))
        .catch((e) => pushToast("warn", `复制失败：${String(e)}`));
    },
    [pushToast],
  );

  /** 切换某条已决策工具的命令全文展开状态 */
  const toggleTool = useCallback(
    (id: number) => {
      setExpandedTools((prev) => {
        const next = new Set(prev);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        return next;
      });
    },
    [setExpandedTools],
  );

  return { runGuarded, runSession, runTurn, runBareRound, runSelfcheck, decideTool, estop, copy, toggleTool };
}
