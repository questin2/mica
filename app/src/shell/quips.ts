// 随机语音条：贴合当前工作状态的短语。
//
// 设计约束（来自需求与既有红线）：
// 1. **不覆盖真实状态文字** —— 顶栏仍显示 `空闲 / 生成中 / 工具执行`，语音条只作为并排的一句陪衬；
// 2. **不连出同一句** —— step() 会跳过上一次的那句，避免"随机"看起来像卡住；
// 3. **减弱动效时静态** —— 调用方传入 allowRotate=false 时只固定取第一句；
// 4. 短语必须是**中性、不承诺结果**的，不能让人误以为任务已完成或失败
//    （尤其 error/done，措辞不能抢在真实结论前宣布什么）。

import type { PermissionMode } from "./types";

/** phase → 候选短语。key 与 constants.ts 的 PHASE_TEXT 对齐。 */
export const PHASE_QUIPS: Record<string, string[]> = {
  idle: [
    "待命中，随时开工",
    "手是空的，脑子还热着",
    "要不要找点事做",
    "我在这儿",
    "随时可以开始",
  ],
  injecting: [
    "正在把上下文铺开",
    "先跟对方打个招呼",
    "准备工作中",
    "把规则先递过去",
  ],
  generating: [
    "琢磨中",
    "咬笔头",
    "写呢写呢",
    "一边想一边写",
    "推演中，别催",
    "思路在跑",
  ],
  finalizing: [
    "收个尾",
    "把最后一句捋顺",
    "检查一遍再交",
    "快好了",
  ],
  tool: [
    "动手了",
    "正在拧螺丝",
    "搬砖中",
    "跑一步看一步",
    "工具在转",
    "让子弹飞一会儿",
  ],
  done: [
    "这一轮收工",
    "交付完毕",
    "看看合不合意",
    "做完了，请过目",
  ],
  timeout: [
    "等太久了，先停一下",
    "这一轮超时了",
    "时间到了",
  ],
  error: [
    "卡了一下",
    "这里出岔子了",
    "我看看是什么问题",
    "重来一次",
  ],
};

/** 权限模式对应的一句提醒 —— 让"当前不会被打断"这件事一直可见。 */
export const MODE_QUIPS: Record<PermissionMode, string[]> = {
  readonly: ["只读模式", "只看不改"],
  workspace: ["工作区内畅行", "写文件不再问你"],
  smart: ["有风险会问你", "该问的时候才问"],
  full: ["全权限，我盯着", "不问直接干，注意风险"],
};

/**
 * 取下一句：尽量不重复上一句。
 *
 * `rand` 可注入，便于测试（默认 Math.random）。
 */
export function stepQuip(
  list: string[],
  prev: string | null,
  rand: () => number = Math.random,
): string {
  if (list.length === 0) return "";
  if (list.length === 1) return list[0];
  // 最多试 6 次换一句
  for (let i = 0; i < 6; i++) {
    const cand = list[Math.floor(rand() * list.length)] ?? list[0];
    if (cand !== prev) return cand;
  }
  // 兜底：随机源退化（例如测试里 rand 恒为 0）时，**确定性地取下一句**，
  // 保证「不连出同一句」这条约束在任何随机源下都成立。
  const idx = prev === null ? -1 : list.indexOf(prev);
  return list[(idx + 1) % list.length] ?? list[0];
}

/* 顶栏随机句子：诗词 / 玩笑 / 干活碎嘴，20s 换一条，不连出同一句。 */
export const TOP_LINES: string[] = [
  "路虽远，行则将至。",
  "不积跬步，无以至千里。",
  "纸上得来终觉浅，绝知此事要躬行。",
  "山重水复疑无路，柳暗花明又一村。",
  "莫愁前路无知己，天下谁人不识君。",
  "长风破浪会有时，直挂云帆济沧海。",
  "工欲善其事，必先利其器。",
  "天下大事，必作于细。",
  "慢就是快，快就是慢。",
  "先把目录看清楚，再动手。",
  "不知道该干嘛的时候，先跑个 dir。",
  "能一步做完的，别分三步。",
  "报错不可怕，可怕的是不读报错。",
  "数据不会说谎，口径会。",
  "先怀疑数据，再怀疑自己。",
  "这个数为什么是这个数？",
  "空值不是零，跨月不是漏。",
  "老板要的是答案，不是过程。",
  "凌晨三点写的代码，白天会报仇。",
  "今天又是和分号搏斗的一天。",
  "不要相信没有验证过的\"已完成\"。",
  "工具是用来干活的，不是用来看的。",
  "慢一点，想清楚，再按回车。",
  "写代码 1 分钟，排查 1 小时，正常。",
  "记得存盘。真的。",
  "咖啡是燃料，睡眠是保养。",
  "别急着优化，先让它跑起来。",
  "能自动化的，就别手动。",
  "所有\"应该没问题\"，都值得再跑一遍。",
  "稳住，我们能赢。",
];

/**
 * 联网取一条随机句子（一言 hitokoto，无需 key）。
 * 失败 / 超时 / 内容异常 → 回落到本地 TOP_LINES，保证标题栏永远有内容。
 */
export async function fetchQuote(prev: string | null = null): Promise<string> {
  try {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), 4000);
    const res = await fetch("https://v1.hitokoto.cn/?encode=json&c=d&c=i&c=k&c=j", {
      signal: ctrl.signal,
    });
    clearTimeout(timer);
    if (!res.ok) throw new Error(String(res.status));
    const j = (await res.json()) as { hitokoto?: string };
    const text = String(j?.hitokoto ?? "").trim();
    if (text.length < 4 || text.length > 56) throw new Error("bad length");
    return text;
  } catch {
    return stepQuip(TOP_LINES, prev);
  }
}
