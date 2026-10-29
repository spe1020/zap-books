import { Component, type ReactNode } from "react";

/**
 * Shows a plain message in place of a part of the app that failed to
 * render, instead of unmounting everything. A damaged saved record then
 * breaks only the screen that shows it: navigation and the other screens
 * (Settings among them) keep working. Changing `resetKey`, by moving to
 * another screen or business, tries again.
 */
export class ErrorBoundary extends Component<
  {
    children: ReactNode;
    /** What failed, as the start of a sentence: "This screen". */
    label: string;
    resetKey?: string;
    /** Shown after the message; says where the user can go from here. */
    note?: string;
    /** Offers a Close button, for a dialog. */
    onClose?: () => void;
  },
  { failed: boolean; resetKey?: string }
> {
  state = { failed: false, resetKey: this.props.resetKey };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  static getDerivedStateFromProps(
    props: { resetKey?: string },
    state: { failed: boolean; resetKey?: string },
  ) {
    return props.resetKey === state.resetKey
      ? null
      : { failed: false, resetKey: props.resetKey };
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div className="alert error" role="alert">
        <p>
          {this.props.label} couldn’t be shown. A saved record may be damaged.{" "}
          {this.props.note ?? "The rest of the app still works."}
        </p>
        <button
          type="button"
          className="button secondary"
          onClick={() => this.setState({ failed: false })}
        >
          Try again
        </button>
        {this.props.onClose && (
          <button
            type="button"
            className="button secondary"
            onClick={this.props.onClose}
          >
            Close
          </button>
        )}
      </div>
    );
  }
}
