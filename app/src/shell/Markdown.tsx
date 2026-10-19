/**
 * 轻量 Markdown 渲染器（Claude 风格的消息排版）。
 *
 * 设计取舍：
 * - **零新依赖**：项目一贯避免新增运行时依赖（图标都是自绘的），故手写解析；
 * - **不注入 HTML**：全部产出 React 元素，绝不使用 `dangerouslySetInnerHTML`，
 *   模型输出属于不可信内容，跨站脚本面必须为零；
 * - **够用即可**：覆盖对话里真正会出现的语法——标题 / 段落 / 粗体 / 斜体 / 行内代码 /
 *   围栏代码块 / 有序·无序列表 / 引用 / 分割线 / 表格 / 链接。
 */
import { type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

/* ---------------- 块级解析 ---------------- */

type Block =
  | { t: "h"; level: number; text: string }
  | { t: "p"; text: string }
  | { t: "code"; lang: string; code: string }
  | { t: "ul"; items: string[] }
  | { t: "ol"; items: string[] }
  | { t: "quote"; lines: string[] }
  | { t: "table"; head: string[]; rows: string[][] }
  | { t: "hr" };

const RE_FENCE = /^\s*```\s*([\w+-]*)\s*$/;
const RE_HEADING = /^(#{1,6})\s+(.*)$/;
const RE_UL = /^\s*[-*+]\s+(.*)$/;
const RE_OL = /^\s*\d+[.)]\s+(.*)$/;
const RE_HR = /^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/;
const RE_QUOTE = /^\s*>\s?(.*)$/;
/** 表格分隔行：|---|:--:|---|（允许单列：|---|）
 *  用 `*` 而非 `+`：单列表格是合法 Markdown，早期版本漏判会把它降级成段落。
 *  安全性：分割线在本判断**之前**处理（见 parseBlocks 顺序），因此 `---` 不会被误当表头分隔。 */
const RE_TBL_SEP = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

function splitRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|")) s = s.slice(0, -1);
  return s.split("|").map((c) => c.trim());
}

export function parseBlocks(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const out: Block[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // 空行
    if (!line.trim()) {
      i++;
      continue;
    }

    // 围栏代码块
    const fence = RE_FENCE.exec(line);
    if (fence) {
      const lang = fence[1] || "";
      const buf: string[] = [];
      i++;
      while (i < lines.length && !/^\s*```/.test(lines[i])) {
        buf.push(lines[i]);
        i++;
      }
      i++; // 跳过结束围栏
      out.push({ t: "code", lang, code: buf.join("\n") });
      continue;
    }

    // 标题
    const h = RE_HEADING.exec(line);
    if (h) {
      out.push({ t: "h", level: h[1].length, text: h[2].trim() });
      i++;
      continue;
    }

    // 分割线（必须在列表之前判断，避免 `---` 被当成 ul）
    if (RE_HR.test(line)) {
      out.push({ t: "hr" });
      i++;
      continue;
    }

    // 表格：当前行含 `|` 且下一行是分隔行
    if (line.includes("|") && i + 1 < lines.length && RE_TBL_SEP.test(lines[i + 1])) {
      const head = splitRow(line);
      i += 2;
      const rows: string[][] = [];
      while (i < lines.length && lines[i].includes("|") && lines[i].trim()) {
        rows.push(splitRow(lines[i]));
        i++;
      }
      out.push({ t: "table", head, rows });
      continue;
    }

    // 引用
    if (RE_QUOTE.test(line)) {
      const buf: string[] = [];
      while (i < lines.length && RE_QUOTE.test(lines[i])) {
        buf.push(RE_QUOTE.exec(lines[i])![1]);
        i++;
      }
      out.push({ t: "quote", lines: buf });
      continue;
    }

    // 无序列表
    if (RE_UL.test(line)) {
      const items: string[] = [];
      while (i < lines.length && RE_UL.test(lines[i])) {
        items.push(RE_UL.exec(lines[i])![1]);
        i++;
      }
      out.push({ t: "ul", items });
      continue;
    }

    // 有序列表
    if (RE_OL.test(line)) {
      const items: string[] = [];
      while (i < lines.length && RE_OL.test(lines[i])) {
        items.push(RE_OL.exec(lines[i])![1]);
        i++;
      }
      out.push({ t: "ol", items });
      continue;
    }

    // 段落：连续非空且不是其他块起始
    const buf: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() &&
      !RE_FENCE.test(lines[i]) &&
      !RE_HEADING.test(lines[i]) &&
      !RE_HR.test(lines[i]) &&
      !RE_UL.test(lines[i]) &&
      !RE_OL.test(lines[i]) &&
      !RE_QUOTE.test(lines[i]) &&
      !(lines[i].includes("|") && i + 1 < lines.length && RE_TBL_SEP.test(lines[i + 1]))
    ) {
      buf.push(lines[i].trim());
      i++;
    }
    out.push({ t: "p", text: buf.join(" ") });
  }

  return out;
}

/* ---------------- 行内解析 ---------------- */

/** 行内语法：行内代码 / 粗体 / 链接 / 斜体（按此优先级匹配）。 */
const RE_INLINE = /(`[^`\n]+`)|(\*\*[^*\n]+\*\*)|(\[[^\]\n]+\]\([^)\s]+\))|(\*[^*\n]+\*)/g;

function openExternal(url: string) {
  // 仅放行 http(s)，其余（javascript: data: 等）一律忽略 —— 防注入
  if (!/^https?:\/\//i.test(url)) return;
  openUrl(url).catch(() => {
    /* 打不开就静默失败，不打断阅读 */
  });
}

export function renderInline(text: string, keyPrefix = ""): ReactNode[] {
  const nodes: ReactNode[] = [];
  let last = 0;
  let k = 0;
  RE_INLINE.lastIndex = 0;
  let m: RegExpExecArray | null;

  while ((m = RE_INLINE.exec(text)) !== null) {
    if (m.index > last) nodes.push(text.slice(last, m.index));
    const tok = m[0];
    const key = `${keyPrefix}i${k++}`;

    if (m[1]) {
      nodes.push(
        <code className="md-code" key={key}>
          {tok.slice(1, -1)}
        </code>,
      );
    } else if (m[2]) {
      nodes.push(<strong key={key}>{tok.slice(2, -2)}</strong>);
    } else if (m[3]) {
      const mm = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(tok);
      // 纵深防御：**只有 http(s) 才产出 <a>**。
      // 不把 javascript:/data: 等写进 href 交给 React 去拦 —— 渲染器自己就不产出，
      // 这样既不依赖下游保护，也不会出现"看着可点、点了没反应"的假链接。
      if (mm && /^https?:\/\//i.test(mm[2])) {
        nodes.push(
          <a
            className="md-link"
            key={key}
            href={mm[2]}
            title={mm[2]}
            onClick={(e) => {
              e.preventDefault();
              openExternal(mm[2]);
            }}
          >
            {mm[1]}
          </a>,
        );
      } else {
        nodes.push(tok);
      }
    } else if (m[4]) {
      nodes.push(<em key={key}>{tok.slice(1, -1)}</em>);
    }
    last = m.index + tok.length;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

/* ---------------- 渲染 ---------------- */

export default function Markdown({ text }: { text: string }) {
  const blocks = parseBlocks(text);
  return (
    <div className="md">
      {blocks.map((b, idx) => {
        const key = `b${idx}`;
        switch (b.t) {
          case "h": {
            const Tag = (`h${Math.min(6, b.level + 1)}`) as "h2";
            return (
              <Tag className={`md-h md-h${b.level}`} key={key}>
                {renderInline(b.text, key)}
              </Tag>
            );
          }
          case "p":
            return (
              <p className="md-p" key={key}>
                {renderInline(b.text, key)}
              </p>
            );
          case "code":
            return (
              <pre className="md-pre" key={key}>
                <code>{b.code}</code>
              </pre>
            );
          case "ul":
            return (
              <ul className="md-ul" key={key}>
                {b.items.map((it, j) => (
                  <li key={`${key}-${j}`}>{renderInline(it, `${key}-${j}`)}</li>
                ))}
              </ul>
            );
          case "ol":
            return (
              <ol className="md-ol" key={key}>
                {b.items.map((it, j) => (
                  <li key={`${key}-${j}`}>{renderInline(it, `${key}-${j}`)}</li>
                ))}
              </ol>
            );
          case "quote":
            return (
              <blockquote className="md-quote" key={key}>
                {b.lines.map((l, j) => (
                  <p key={`${key}-${j}`}>{renderInline(l, `${key}-${j}`)}</p>
                ))}
              </blockquote>
            );
          case "table":
            return (
              <div className="md-tablewrap" key={key}>
                <table className="md-table">
                  <thead>
                    <tr>
                      {b.head.map((c, j) => (
                        <th key={`${key}-h${j}`}>{renderInline(c, `${key}-h${j}`)}</th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {b.rows.map((r, ri) => (
                      <tr key={`${key}-r${ri}`}>
                        {r.map((c, ci) => (
                          <td key={`${key}-r${ri}c${ci}`}>{renderInline(c, `${key}-r${ri}c${ci}`)}</td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            );
          case "hr":
            return <hr className="md-hr" key={key} />;
        }
      })}
    </div>
  );
}
