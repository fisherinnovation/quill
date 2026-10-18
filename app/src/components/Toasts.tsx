// Small cards at the bottom right of the window, for news from the
// background: a meeting written up while you were elsewhere in the app.
import { useEffect, useRef } from "react";
import type { Outcome } from "../api";
import { Icon } from "./Icon";

export interface Toast {
  /** The meeting it's about (one toast per meeting). */
  id: string;
  kind: "done" | "failed";
  title: string;
  outcome: Outcome | null;
  message: string | null;
}

/** How long a toast stays, unless the pointer is on it. */
const STAY_MS = 9_000;

export function Toasts({
  toasts,
  onOpen,
  onDismiss,
}: {
  toasts: Toast[];
  onOpen: (id: string) => void;
  onDismiss: (id: string) => void;
}) {
  return (
    <div className="toasts" role="region" aria-label="Notifications">
      {toasts.map((t) => (
        <ToastCard key={t.id} toast={t} onOpen={() => onOpen(t.id)} onDismiss={() => onDismiss(t.id)} />
      ))}
    </div>
  );
}

function ToastCard({ toast, onOpen, onDismiss }: { toast: Toast; onOpen: () => void; onDismiss: () => void }) {
  const timer = useRef<number | undefined>(undefined);
  const arm = () => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(onDismiss, STAY_MS);
  };
  useEffect(() => {
    // Armed once when it appears; the pointer on it pauses it.
    arm();
    return () => window.clearTimeout(timer.current);
  }, []);

  const o = toast.outcome;
  const detail =
    toast.kind === "failed"
      ? (toast.message ?? "Something went wrong.")
      : o && o.notes
        ? `${o.action_items} action ${o.action_items === 1 ? "item" : "items"} · ${o.decisions} ${o.decisions === 1 ? "decision" : "decisions"}`
        : "The transcript is ready.";
  return (
    <div
      className={`toast ${toast.kind}`}
      role="status"
      onMouseEnter={() => window.clearTimeout(timer.current)}
      onMouseLeave={arm}
    >
      <span className="toast-icon" aria-hidden>
        <Icon name={toast.kind === "failed" ? "alert" : "check"} size={14} />
      </span>
      <div className="toast-body">
        <strong>{toast.kind === "failed" ? `Couldn't finish ${toast.title}` : `Notes ready: ${toast.title}`}</strong>
        <span>{detail}</span>
      </div>
      <button className="toast-open" onClick={onOpen}>
        Open
      </button>
      <button className="ghost icon-btn" aria-label="Dismiss" onClick={onDismiss}>
        <Icon name="x" size={12} />
      </button>
    </div>
  );
}
