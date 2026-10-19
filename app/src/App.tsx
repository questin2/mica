import { Component, type ErrorInfo, type ReactNode } from "react";
import Shell from "./shell/Shell";

/**
 * 应用入口。
 *
 * UI 已按 `docs/07-ui-design-plan.md` v1.0 重构为 **Shell**（三栏工作台 + 工具三级审批 +
 * 状态矩阵 + 设计 token），并保留原有全部引擎接线：
 *   `set_browser_rect` / `start_agent_session` / `agent_turn` / `test_roundtrip` /
 *   `tool_decision` / `selfcheck`，事件 `engine://state` 与 `engine://tool`。
 *
 * 旧版单面板实现（工作目录 + ①② + 审批 + 引擎日志平铺）见 git 历史；其样式 `App.css` 已随本接入删除。
 * 回滚方式：`git log -- app/src/App.tsx` 找到旧版提交后 `git checkout <commit> -- app/src/App.tsx app/src/App.css`。
 */

/**
 * 【F-05】全局错误边界：Shell 组件任何未捕获异常都不会导致整个 WebView 白屏，
 * 而是显示可恢复的错误页，提供"刷新页面"按钮。
 */
class AppErrorBoundary extends Component<{ children: ReactNode }, { hasError: boolean; error: string }> {
  constructor(props: { children: ReactNode }) {
    super(props);
    this.state = { hasError: false, error: "" };
  }

  static getDerivedStateFromError(error: Error) {
    return { hasError: true, error: error.message || String(error) };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[AppErrorBoundary] 组件异常:", error, info.componentStack);
  }

  handleReload = () => {
    window.location.reload();
  };

  render() {
    if (this.state.hasError) {
      return (
        <div style={{
          display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
          height: "100vh", gap: 16, padding: 32, fontFamily: "system-ui, sans-serif",
          background: "#1a1a2e", color: "#eee",
        }}>
          <div style={{ fontSize: 48 }}>⚠️</div>
          <h2 style={{ margin: 0 }}>界面异常</h2>
          <p style={{ margin: 0, maxWidth: 480, textAlign: "center", color: "#aaa", fontSize: 14, lineHeight: 1.6 }}>
            Shell 组件发生未捕获异常。这通常是引擎事件数据格式变化或状态不一致导致的。
          </p>
          <pre style={{
            maxWidth: 560, maxHeight: 160, overflow: "auto", padding: 12,
            background: "#0d0d1a", borderRadius: 8, fontSize: 12, color: "#f88",
          }}>
            {this.state.error}
          </pre>
          <button
            onClick={this.handleReload}
            style={{
              padding: "10px 24px", borderRadius: 8, border: "none",
              background: "#4f6ef7", color: "#fff", fontSize: 14, cursor: "pointer",
            }}
          >
            刷新页面
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

export default function App() {
  return (
    <AppErrorBoundary>
      <Shell />
    </AppErrorBoundary>
  );
}
