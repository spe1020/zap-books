import { createContext, useContext, useEffect, useRef } from "react";
export const SaveErrorContext = createContext("");
import type { ReactNode } from "react";
import { X, FileText, ArrowUpRight } from "lucide-react";
import type { BusinessData, ExpenseDraft } from "../domain/types";
import { duplicates, missingDetails } from "../domain/ledger";
export function Modal({
  title,
  children,
  onClose,
  wide = false,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  wide?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const saveError = useContext(SaveErrorContext);
  useEffect(() => {
    const el = ref.current!;
    const restoreFocus = document.activeElement as HTMLElement | null;
    const trapFocus = (event: KeyboardEvent) => {
      if (event.key !== "Tab") return;
      const controls = [
        ...el.querySelectorAll<HTMLElement>(
          'button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), summary, [tabindex]:not([tabindex="-1"])',
        ),
      ].filter(
        (node) =>
          node.getClientRects().length &&
          getComputedStyle(node).visibility !== "hidden",
      );
      const first = controls[0],
        last = controls.at(-1);
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    };
    el.showModal();
    el.addEventListener("keydown", trapFocus);
    return () => {
      el.removeEventListener("keydown", trapFocus);
      el.close();
      if (restoreFocus?.isConnected) restoreFocus.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      className={`modal ${wide ? "wide" : ""}`}
      aria-label={title}
    >
      <header className="modal-head">
        <h2>{title}</h2>
        <button
          className="icon-button"
          onClick={onClose}
          aria-label="Close dialog"
        >
          <X size={20} />
        </button>
      </header>
      {saveError && (
        <div className="alert error global-error" role="alert">
          {saveError}
        </div>
      )}
      {children}
    </dialog>
  );
}
export function Status({ b, d }: { b: BusinessData; d: ExpenseDraft }) {
  if (d.status === "corrected")
    return <span className="badge neutral">Corrected</span>;
  if (d.status === "posted")
    return <span className="badge success">Posted</span>;
  if (duplicates(b, d).length && !d.duplicateReason)
    return <span className="badge warning">Possible duplicate</span>;
  if (missingDetails(b, d).length)
    return <span className="badge warning">Needs details</span>;
  if (d.paymentId) return <span className="badge blue">Payment matched</span>;
  return <span className="badge success">Ready to review</span>;
}
export function Empty({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="empty">
      <FileText size={30} />
      <h3>{title}</h3>
      <p>{children}</p>
    </div>
  );
}
export function VendorIcon({ name }: { name: string }) {
  const colors = ["lilac", "sand", "sage", "blue"];
  const i = [...name].reduce((a, c) => a + c.charCodeAt(0), 0) % 4;
  return (
    <span className={`vendor-icon ${colors[i]}`}>
      {name.slice(0, 1).toUpperCase() || "?"}
    </span>
  );
}
export function ArrowLink({
  children,
  onClick,
}: {
  children: ReactNode;
  onClick: () => void;
}) {
  return (
    <button className="text-button" onClick={onClick}>
      {children}
      <ArrowUpRight size={15} />
    </button>
  );
}
export function shortDate(date: string) {
  return new Date(`${date}T12:00:00`).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  });
}
