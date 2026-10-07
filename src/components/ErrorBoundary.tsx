import { Component, type ErrorInfo, type ReactNode } from "react";
import { errorText } from "../api";

interface Props {
  /** Shown in the console log ("Review", "pane builder", …). */
  where: string;
  /** Sub-screens: offers "Close" (leave the screen / dialog). */
  onClose?(): void;
  /** "root" fills the window, "modal" floats over it, "pane"/"screen" fill their slot. */
  variant?: "root" | "screen" | "modal" | "pane";
  /** Changing this clears the error (e.g. a pane showing another agent). */
  resetKey?: unknown;
  children: ReactNode;
}

interface State {
  error: unknown;
  resetKey: unknown;
}

/** Catches render errors so one broken screen doesn't freeze the whole window. */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, resetKey: this.props.resetKey };

  static getDerivedStateFromError(error: unknown): Partial<State> {
    return { error: error ?? new Error("Unknown error") };
  }

  static getDerivedStateFromProps(props: Props, state: State): Partial<State> | null {
    return props.resetKey !== state.resetKey ? { resetKey: props.resetKey, error: null } : null;
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error(`pitwall: ${this.props.where} crashed:`, error, info.componentStack);
  }

  private close = () => {
    this.setState({ error: null });
    this.props.onClose?.();
  };

  render() {
    const { error } = this.state;
    if (error === null) return this.props.children;
    const { variant = "screen", onClose, where } = this.props;
    const detail = error instanceof Error && error.stack ? error.stack : errorText(error);
    const box = (
      <div className="crash" data-variant={variant} role="alert">
        <h2 className="crash-title">Something broke here</h2>
        <details className="crash-details">
          <summary className="muted-sm">{errorText(error) || "Error details"}</summary>
          <pre className="crash-text mono">{`${where}\n${detail}`}</pre>
        </details>
        <div className="row gap">
          <button className="primary-btn" onClick={() => window.location.reload()}>
            Reload
          </button>
          {onClose && (
            <button className="ghost-btn" onClick={this.close}>
              Close
            </button>
          )}
        </div>
      </div>
    );
    return variant === "modal" ? <div className="backdrop backdrop-dialog">{box}</div> : box;
  }
}
