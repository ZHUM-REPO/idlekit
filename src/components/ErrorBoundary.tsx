import { Component, type ErrorInfo, type ReactNode } from "react";

interface State {
  error: Error | null;
}

/** Shows what broke instead of leaving a blank window. */
export class ErrorBoundary extends Component<{ children: ReactNode; resetKey?: string }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("IdleKit UI error:", error, info.componentStack);
  }

  componentDidUpdate(prev: { resetKey?: string }) {
    if (prev.resetKey !== this.props.resetKey && this.state.error) this.setState({ error: null });
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="max-w-2xl py-6">
        <h1 className="heading text-[22px] font-semibold">This page hit a problem</h1>
        <p className="mt-2 text-muted">
          The rest of IdleKit keeps running. Try another page, or reload the window. If it keeps happening, copy this
          message into a bug report:
        </p>
        <pre className="mt-4 overflow-auto rounded-lg border border-line bg-well p-3 font-mono text-[12px] text-bad whitespace-pre-wrap">
          {`${this.state.error.name}: ${this.state.error.message}\n\n${this.state.error.stack ?? ""}`}
        </pre>
        <button
          className="mt-4 h-8.5 rounded-md border border-line bg-raised px-3.5 text-[13px] hover:border-line-strong"
          onClick={() => window.location.reload()}
        >
          Reload window
        </button>
      </div>
    );
  }
}
