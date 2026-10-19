// 引擎事件订阅（engine://state 与 engine://tool）—— 拆分自 Shell.tsx（S-01）
//
// 这段是「引擎 → UI」的唯一入口，包含**会话信任自动批准**分支：
// 用户此前对只读/普通类显式选择过「本会话内此类全部允许」时直接批准，不再逐条打断；
// 危险类永不进入该分支（classify 判定 + 入集校验双重保证）。
import { useEffect, type Dispatch, type SetStateAction } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { hm } from "../format";
import { auditAutoApproved, auditConfigAutoApproved } from "../audit";
import { decideApprovalForMode } from "../autoApprove";
import type { DoneTool, EngineEvent, LogRow, PermissionMode, Step, Toast, ToolReq } from "../types";

type Opts = {
  setPhase: Dispatch<SetStateAction<string>>;
  setDetail: Dispatch<SetStateAction<string>>;
  setLogs: Dispatch<SetStateAction<LogRow[]>>;
  setToolReq: Dispatch<SetStateAction<ToolReq | null>>;
  setDoneTools: Dispatch<SetStateAction<DoneTool[]>>;
  /** 当前权限模式：决定哪些风险类别不再逐条询问 */
  permissionMode: PermissionMode;
  /** 内联步骤时间线（思考 / 工具 / 阶段），按发生顺序 */
  setSteps: Dispatch<SetStateAction<Step[]>>;
  /** 思考过程正文（DeepSeek 的 THINK 片段），空串表示本轮无思考 */
  setThinking: Dispatch<SetStateAction<string>>;
  pushToast: (kind: Toast["kind"], text: string) => void;
};

/** 命令摘要：取第一行非空内容，截断到 60 字 —— 时间线里不要铺满整段脚本。 */
function summarize(code: string): string {
  const line = code.split(/\r?\n/).map((s) => s.trim()).find((s) => s.length > 0) ?? code.trim();
  return line.length > 60 ? `${line.slice(0, 60)}…` : line;
}

export function useEngineEvents({
  setPhase,
  setDetail,
  setLogs,
  setToolReq,
  setDoneTools,
  permissionMode,
  setThinking,
  setSteps,
  pushToast,
}: Opts) {
  useEffect(() => {
    const unState = listen<EngineEvent>("engine://state", (e) => {
      setPhase(e.payload.phase);
      setDetail(e.payload.detail);
      // 上次运行崩溃：启动时引擎会发 crash 事件，直接弹一条可见告警。
      // 自动放行的工具调用（不经审批）→ 也进"已执行"面板，便于核对模型说的对不对
      // 思考过程：DeepSeek 每个回合会把 THINK 片段随 SSE 一起推上来，
      // 后端提取后以 phase="think" 广播。保留最后一次（不做累加，后端已给全量）。
      if (e.payload.phase === "think") {
        setThinking(e.payload.detail);
        // ★ 必须**同时**进时间线：否则思考只活在"底部忙碌行"里，
        //   回合结束（busy=false）那一行就消失 → 用户看到"闪一下就没有了"。
        setSteps((prev) => [
          ...prev.slice(-29),
          { id: e.payload.seq, kind: "think", text: e.payload.detail, ts: hm() },
        ]);
      }
      // 工具执行（自动放行）也进时间线
      if (e.payload.phase === "auto_tool") {
        setSteps((prev) => [...prev.slice(-29), { id: e.payload.seq, kind: "tool", text: e.payload.detail, ts: hm() }]);
      }
      if (e.payload.phase === "auto_tool") {
        setDoneTools((d) => [
          ...d,
          { id: e.payload.seq, code: `${e.payload.detail}（自动放行·未审批）`, allow: true, ts: hm() },
        ]);
      }
      if (e.payload.phase === "crash") {
        pushToast("danger", e.payload.detail);
      }
      setLogs((l) => [
        ...l.slice(-399),
        { seq: e.payload.seq, t: hm(), phase: e.payload.phase, detail: e.payload.detail },
      ]);
    });

    const unTool = listen<EngineEvent>("engine://tool", (e) => {
      if (e.payload.phase !== "request") return;
      const i = e.payload.detail.indexOf("|");
      if (i < 0) return;
      const id = Number(e.payload.detail.slice(0, i));
      const code = e.payload.detail.slice(i + 1);

      // ── 放行决策（顺序在 decideApproval 内固定：危险类永远最先，一律问用户）──
      const decision = decideApprovalForMode(code, permissionMode);
      if (decision.kind !== "ask") {
        const patternHint = decision.kind === "auto-config" ? decision.pattern : "";
        if (decision.kind === "auto-session") {
          // 【审计】自动放行必须留痕 —— 否则磁盘审计分不出「人工允许」与「自动放行」
          auditAutoApproved(id, decision.clsLabel);
        } else {
          // 【审计】记录命中的规则原文（配置规则跨重启生效，事后要知道"当时是哪条规则放行的"）
          auditConfigAutoApproved(id, decision.pattern);
        }
        void invoke("tool_decision", { id, allow: true }).catch((err) =>
          pushToast("danger", `自动批准失败 #${id}：${String(err)}`),
        );
        setDoneTools((d) => [...d, { id, code, allow: true, ts: hm() }]);
        setSteps((prev) => [
          ...prev.slice(-29),
          { id, kind: "tool", text: `#${id} ${summarize(code)}（自动放行）`, ts: hm() },
        ]);
        setLogs((l) => [
          ...l.slice(-399),
          {
            seq: e.payload.seq,
            t: hm(),
            phase: "tool",
            detail:
              decision.kind === "auto-session"
                ? `#${id} 按会话信任自动允许（${decision.clsLabel}类）`
                : `#${id} 按配置规则自动放行（你未看到该命令；规则：${patternHint}）`,
          },
        ]);
        if (decision.kind === "auto-config") {
          pushToast("warn", `已按配置规则自动放行 #${id}（你未看到该命令，审计页可查）`);
        }
        return;
      }

      setToolReq({ id, code });
      setSteps((prev) => [
        ...prev.slice(-29),
        { id, kind: "phase", text: `#${id} 等待审批 · ${summarize(code)}`, ts: hm() },
      ]);
      setLogs((l) => [
        ...l.slice(-399),
        { seq: e.payload.seq, t: hm(), phase: "tool", detail: `请求执行 #${id}（等待审批）` },
      ]);
    });

    return () => {
      unState.then((f) => f()).catch(() => {});
      unTool.then((f) => f()).catch(() => {});
    };
    // 依赖为空是**刻意**的：监听器只注册一次，回调内通过 setState 的函数式更新与 ref 读最新值
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
