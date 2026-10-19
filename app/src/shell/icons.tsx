/* Mica 图标集（策划案 §4.4）
   规范：24 网格、stroke=currentColor、stroke-width=1.5、圆角端点、不填充（stop/moon 例外）。
   零依赖实现：同规格线性图元；若后续引入 lucide-react（同网格/同线宽）可无缝替换。 */
import type { CSSProperties } from "react";

const PATHS: Record<string, string> = {
  plus: '<path d="M12 5v14M5 12h14"/>',
  terminal:
    '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9.5l2.5 2.5L7 14.5M12.5 15H17"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7.5V12l3 2"/>',
  sparkle:
    '<path d="M12 3.2l1.9 4.9 4.9 1.9-4.9 1.9L12 16.8l-1.9-4.9L5.2 10l4.9-1.9z"/><path d="M18.5 16.5l.8 2 2 .8-2 .8-.8 2-.8-2-2-.8 2-.8z"/>',
  shield: '<path d="M12 3l7 3v5c0 4.4-2.9 7.7-7 10-4.1-2.3-7-5.6-7-10V6z"/>',
  "shield-alert":
    '<path d="M12 3l7 3v5c0 4.4-2.9 7.7-7 10-4.1-2.3-7-5.6-7-10V6z"/><path d="M12 8.8v4.4M12 16.2v.01"/>',
  gear:
    '<circle cx="12" cy="12" r="3.2"/><path d="M18 12h2.5M16.24 16.24l1.77 1.77M12 18v2.5M7.76 16.24l-1.77 1.77M6 12H3.5M7.76 7.76L5.99 5.99M12 6V3.5M16.24 7.76l1.77-1.77"/>',
  folder:
    '<path d="M3 7a2 2 0 0 1 2-2h3.2l1.8 2h9a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  sun:
    '<circle cx="12" cy="12" r="4"/><path d="M19 12h2.5M12 19v2.5M5 12H2.5M12 5V2.5M16.95 16.95l1.77 1.77M16.95 7.05l1.77-1.77M7.05 16.95l-1.77 1.77M7.05 7.05L5.28 5.28"/>',
  moon: '<path d="M20 14.5A8.5 8.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5z"/>',
  monitor: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8.5 20h7M12 16v4"/>',
  attach:
    '<path d="M12 16.5V4.5M8 8.5l4-4 4 4M4.5 16.5v3a1 1 0 0 0 1 1h13a1 1 0 0 0 1-1v-3"/>',
  send: '<path d="M12 19.5V4.5M5.5 11L12 4.5 18.5 11"/>',
  stop: '<rect x="7" y="7" width="10" height="10" rx="2" fill="currentColor" stroke="none"/>',
  camera:
    '<path d="M4 8.5A1.5 1.5 0 0 1 5.5 7h2L9 5h6l1.5 2h2A1.5 1.5 0 0 1 20 8.5v9A1.5 1.5 0 0 1 18.5 19h-13A1.5 1.5 0 0 1 4 17.5z"/><circle cx="12" cy="12.5" r="3.2"/>',
  keyboard:
    '<rect x="3" y="7" width="18" height="10" rx="2"/><path d="M7 11h.01M10.5 11h.01M14 11h.01M17.5 11h.01M8.5 14h7"/>',
  mouse: '<path d="M5.5 3.5l13 7.2-5.6 1.4 3.4 5.8-2.4 1.3-3.3-5.9-3.6 3z"/>',
  "alert-triangle": '<path d="M12 4.2l8.8 14.6H3.2z"/><path d="M12 9.8v4M12 16.4v.01"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
  x: '<path d="M6 6l12 12M18 6L6 18"/>',
  "chevron-down": '<path d="M6 9.5l6 6 6-6"/>',
  "chevron-right": '<path d="M9.5 6l6 6-6 6"/>',
  "chevron-left": '<path d="M14.5 6l-6 6 6 6"/>',
  refresh: '<path d="M20 12a8 8 0 1 1-3.2-6.4"/><path d="M20 4.5v5h-5"/>',
  link: '<path d="M14 4h6v6M20 4l-8.5 8.5M10 5H6a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2v-4"/>',
  copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5.5M12 7.8v.01"/>',
  zap: '<path d="M13 3.2L5.5 14H11l-1 6.8L17.5 10H12z"/>',
  lock: '<rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8.5 11V8.2a3.5 3.5 0 0 1 7 0V11"/>',
  plug: '<path d="M9 3v5M15 3v5M6.5 8h11v3.2a5.5 5.5 0 0 1-11 0z"/><path d="M12 16.7V21"/>',
  search: '<circle cx="11" cy="11" r="6.5"/><path d="M15.8 15.8L21 21"/>',
  bot: '<rect x="4" y="7" width="16" height="12" rx="3"/><path d="M9.5 12.5v.01M14.5 12.5v.01M12 3v4"/>',
  panel: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M15 4v16"/>',
  user: '<circle cx="12" cy="8.5" r="3.5"/><path d="M5.5 20a6.5 6.5 0 0 1 13 0"/>',
  upload:
    '<path d="M12 16V4.5M8 8.5l4-4 4 4M4.5 16.5v3a1 1 0 0 0 1 1h13a1 1 0 0 0 1-1v-3"/>',
  download: '<path d="M12 4v11.5M8 11.5l4 4 4-4M4.5 16.5v3a1 1 0 0 0 1 1h13a1 1 0 0 0 1-1v-3"/>',
  history: '<path d="M3.5 12a8.5 8.5 0 1 0 2.6-6.1M3.5 4.5v5h5"/><path d="M12 8v4.5l3 1.8"/>',
  list: '<path d="M8 6h13M8 12h13M8 18h13M3.5 6h.01M3.5 12h.01M3.5 18h.01"/>',
  book: '<path d="M4 5.5A2.5 2.5 0 0 1 6.5 3H20v18H6.5A2.5 2.5 0 0 1 4 18.5z"/><path d="M4 18.5A2.5 2.5 0 0 1 6.5 16H20"/>',
  flow: '<rect x="3" y="4" width="6" height="5" rx="1.5"/><rect x="15" y="15" width="6" height="5" rx="1.5"/><path d="M9 6.5h4a3 3 0 0 1 3 3v3a3 3 0 0 0 3 3"/>',
};

export function Icon({
  name,
  size = 18,
  className,
  style,
}: {
  name: string;
  size?: number;
  className?: string;
  style?: CSSProperties;
}) {
  const body = PATHS[name] ?? PATHS.info;
  return (
    <svg
      className={className ? `icon ${className}` : "icon"}
      style={style}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      dangerouslySetInnerHTML={{ __html: body }}
    />
  );
}

export const ICON_NAMES = Object.keys(PATHS);
