// 随机语音条：只测"不连出同一句"这一条硬约束，其余是纯展示。
import { describe, expect, it } from "vitest";
import { PHASE_QUIPS, stepQuip, MODE_QUIPS } from "../quips";

describe("stepQuip", () => {
  it("空列表返回空串（调用方据此不渲染）", () => {
    expect(stepQuip([], null)).toBe("");
  });

  it("只有一句时直接返回它", () => {
    expect(stepQuip(["唯一"], "唯一")).toBe("唯一");
  });

  it("绝不连出同一句 —— 即使随机数一直取同一个索引", () => {
    // rand 恒为 0 → 一直取 list[0]；因为 prev 是 list[0]，必须换到别的
    const q = stepQuip(["A", "B", "C"], "A", () => 0);
    expect(q).not.toBe("A");
  });

  it("返回的句子一定来自列表", () => {
    const list = ["A", "B", "C"];
    for (let i = 0; i < 20; i++) {
      expect(list).toContain(stepQuip(list, null));
    }
  });
});

describe("短语表", () => {
  it("每个阶段都有候选，且每条非空", () => {
    for (const [phase, list] of Object.entries(PHASE_QUIPS)) {
      expect(list.length, phase).toBeGreaterThan(0);
      for (const s of list) expect(s.trim()).not.toBe("");
    }
  });

  it("四种权限模式都有提醒语", () => {
    for (const m of ["readonly", "workspace", "smart", "full"] as const) {
      expect(MODE_QUIPS[m].length).toBeGreaterThan(0);
    }
  });
});
