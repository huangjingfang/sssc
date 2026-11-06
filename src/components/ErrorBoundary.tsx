import React from "react";

/** 全局错误边界：渲染崩溃不再白屏，界面直接显示真实错误与重试按钮 */
export default class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[SSSC] 渲染崩溃:", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="empty-state">
          <div className="empty-card">
            <h2 style={{ color: "var(--ds-danger)" }}>页面渲染出错</h2>
            <p className="mono" style={{ fontSize: 12, textAlign: "left", wordBreak: "break-all", maxHeight: 160, overflow: "auto" }}>
              {this.state.error.message}
              {"\n"}
              {this.state.error.stack?.split("\n").slice(1, 5).join("\n")}
            </p>
            <button className="btn btn-primary" onClick={() => this.setState({ error: null })}>
              重试
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}
