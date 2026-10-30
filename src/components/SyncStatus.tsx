import { useState } from "react";
import { CloudOff, RefreshCw, TriangleAlert, CheckCircle2 } from "lucide-react";
import type { RemoteView } from "../remote";
import { Modal } from "./common";

/**
 * The small persistent indicator of sync with the ledger service: synced,
 * syncing, offline (with how many changes wait), or an error. It opens the
 * list of changes waiting to sync, where a refused one can be discarded or
 * redone from the current books.
 */
export function SyncStatus({
  view,
  onDiscard,
  onRedo,
}: {
  view: RemoteView;
  onDiscard: (id: number) => void;
  onRedo: (id: number) => void;
}) {
  const [open, setOpen] = useState(false);
  const { state, queue, importing } = view;
  const waiting = queue.length;
  const label =
    state.status === "synced"
      ? waiting
        ? `${waiting} waiting to sync`
        : "Synced"
      : state.status === "syncing"
        ? importing
          ? `Importing: batch ${importing.batch} of ${importing.of}`
          : "Syncing…"
        : state.status === "offline"
          ? `Offline · ${state.waiting} waiting`
          : "Sync error";
  const Icon =
    state.status === "error"
      ? TriangleAlert
      : state.status === "offline"
        ? CloudOff
        : state.status === "syncing"
          ? RefreshCw
          : CheckCircle2;
  return (
    <>
      <button
        className={`sync-status ${state.status}`}
        onClick={() => setOpen(true)}
        aria-label={`Sync status: ${label}`}
      >
        <Icon size={15} />
        <span>{label}</span>
      </button>
      {open && (
        <Modal title="Sync status" onClose={() => setOpen(false)}>
          <div className="modal-form">
            {state.status === "error" && (
              <p role="alert" className="alert error">
                {state.message}
              </p>
            )}
            {importing && (
              <p role="status">
                Importing: batch {importing.batch} of {importing.of}
              </p>
            )}
            <h3>Waiting to sync</h3>
            {queue.length ? (
              <ul className="waiting-list">
                {queue.map((q) => (
                  <li key={q.id}>
                    <span>
                      <strong>{q.label}</strong>
                      <small>
                        {q.rejected
                          ? `Refused: ${q.rejected}`
                          : "Waiting to sync"}
                      </small>
                    </span>
                    {q.rejected && (
                      <span className="waiting-actions">
                        <button
                          className="button secondary small"
                          onClick={() => onDiscard(q.id!)}
                        >
                          Discard
                        </button>
                        <button
                          className="button primary small"
                          onClick={() => onRedo(q.id!)}
                        >
                          Redo from current books
                        </button>
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="muted">
                Nothing is waiting. Every change was sent.
              </p>
            )}
            <p className="small muted">
              The books shown are the ones the service accepted and this browser
              verified. Changes waiting here aren't in them yet.
            </p>
          </div>
        </Modal>
      )}
    </>
  );
}
