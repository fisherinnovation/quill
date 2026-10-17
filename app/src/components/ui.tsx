// Small shared building blocks for the settings-style pages.
import { useEffect, useRef, useState, type ReactNode } from "react";
import { errorText, hotkeyLabel } from "../api";
import { Icon } from "./Icon";

export function Switch({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
}) {
  // A label, so a click on the track (drawn over the hidden checkbox)
  // toggles it.
  return (
    <label className="switch">
      <input type="checkbox" role="switch" aria-label={label} checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span className="switch-track" />
    </label>
  );
}

/** A shortcut as key caps: "Alt+Space" → ⌥ + Space. */
export function Keys({ hotkey, large }: { hotkey: string; large?: boolean }) {
  return (
    <span className={`keys${large ? " large" : ""}`}>
      {hotkeyLabel(hotkey)
        .split(/ \+ | /)
        .filter(Boolean)
        .map((key, i) => (
          <kbd key={i}>{key}</kbd>
        ))}
    </span>
  );
}

export function Group({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <section className="group">
      {title && <h2 className="group-title">{title}</h2>}
      <div className="card">{children}</div>
    </section>
  );
}

/** One setting: label and hint on the left, the control on the right. */
export function Row({
  label,
  hint,
  children,
  className,
}: {
  label: ReactNode;
  hint?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div className={className ? `row ${className}` : "row"}>
      <div className="row-label">
        <span>{label}</span>
        {hint && <p className="row-hint">{hint}</p>}
      </div>
      {children && <div className="row-control">{children}</div>}
    </div>
  );
}

export type SaveState = { kind: "idle" } | { kind: "saving" } | { kind: "saved" } | { kind: "error"; message: string };

/**
 * Saves `value` a moment after it stops changing (so typing doesn't save on
 * every keystroke). The first value, as loaded, isn't saved back.
 */
export function useAutosave<T>(value: T | null, save: (value: T) => Promise<void>, delay = 600): SaveState {
  const [state, setState] = useState<SaveState>({ kind: "idle" });
  const loaded = useRef<T | null>(null);
  const saveRef = useRef(save);
  saveRef.current = save;

  useEffect(() => {
    if (value === null) return;
    if (loaded.current === null) {
      loaded.current = value;
      return;
    }
    const timer = setTimeout(() => {
      setState({ kind: "saving" });
      saveRef.current(value).then(
        () => setState({ kind: "saved" }),
        (e) => setState({ kind: "error", message: errorText(e) }),
      );
    }, delay);
    return () => clearTimeout(timer);
  }, [value, delay]);

  return state;
}

export function SaveIndicator({ state }: { state: SaveState }) {
  if (state.kind === "idle") return null;
  if (state.kind === "error") {
    return (
      <span className="save-state error-text">
        <Icon name="alert" size={14} /> {state.message}
      </span>
    );
  }
  return (
    <span className="save-state">
      {state.kind === "saving" ? (
        <>
          <span className="spinner" /> Saving…
        </>
      ) : (
        <>
          <Icon name="check" size={14} /> Saved
        </>
      )}
    </span>
  );
}

export function Banner({
  kind = "info",
  icon,
  children,
  action,
}: {
  kind?: "info" | "error" | "warning" | "recording";
  icon?: Parameters<typeof Icon>[0]["name"];
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className={`banner ${kind}`} role={kind === "error" ? "alert" : "status"}>
      {icon && <Icon name={icon} size={16} />}
      <div className="banner-text">{children}</div>
      {action}
    </div>
  );
}

/** Languages offered in the pickers (Whisper supports many more). */
export const LANGUAGES: [string, string][] = [
  ["en", "English"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["de", "German"],
  ["it", "Italian"],
  ["pt", "Portuguese"],
  ["nl", "Dutch"],
  ["sv", "Swedish"],
  ["pl", "Polish"],
  ["ru", "Russian"],
  ["uk", "Ukrainian"],
  ["ja", "Japanese"],
  ["zh", "Chinese"],
  ["ko", "Korean"],
  ["hi", "Hindi"],
];

/** A headline number: label, value and a short note under it. */
export function Stat({
  label,
  value,
  note,
  accent,
}: {
  label: string;
  value: string;
  note: string;
  accent?: boolean;
}) {
  return (
    <div className={`stat card${accent ? " accent" : ""}`}>
      <div className="stat-label">{label}</div>
      <div className="stat-value">{value}</div>
      <div className="stat-note">{note}</div>
    </div>
  );
}

/** A link back to the list a page was opened from: "← Meetings". */
export function BackLink({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button className="back-link" onClick={onClick}>
      <Icon name="back" size={14} /> {label}
    </button>
  );
}

/**
 * "Today", "Yesterday", weekday names this week, then "October 2026". The
 * third element says whether the group is a single day (so its items need
 * only a time).
 */
export function groupByDay<T>(items: T[], when: (item: T) => string): [string, T[], boolean][] {
  const groups = new Map<string, [T[], boolean]>();
  const startOfToday = new Date();
  startOfToday.setHours(0, 0, 0, 0);
  const day = 86_400_000;
  for (const m of items) {
    const date = new Date(when(m));
    const ago = startOfToday.getTime() - date.getTime();
    const oneDay = ago <= 6 * day;
    const label =
      ago <= 0
        ? "Today"
        : ago <= day
          ? "Yesterday"
          : oneDay
            ? date.toLocaleDateString(undefined, { weekday: "long" })
            : date.toLocaleDateString(undefined, { month: "long", year: "numeric" });
    const group = groups.get(label) ?? [[], oneDay];
    group[0].push(m);
    groups.set(label, group);
  }
  return [...groups].map(([label, [group, oneDay]]) => [label, group, oneDay]);
}

/** "2:15 PM" in the user's locale. */
export function timeOfDay(rfc3339: string): string {
  return new Date(rfc3339).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}
