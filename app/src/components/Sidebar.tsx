import { type ReactNode } from "react";
import { type DictationPhase } from "../api";
import { Icon, type IconName } from "./Icon";

/** The pages the sidebar leads to. */
export type Section = "home" | "meetings" | "dictation" | "insights" | "settings";

interface Props {
  /** The page showing (or the list the open item belongs to), to highlight it. */
  current: Section | null;
  onNavigate: (section: Section) => void;
  /** A meeting is being recorded. */
  recording: boolean;
  /** Meetings being transcribed or summarized. */
  working: number;
  dictation: DictationPhase;
  /** Shown above the navigation (the record button). */
  header: ReactNode;
}

/** Navigation only: the meetings and dictations are listed on their own pages. */
export function Sidebar({ current, onNavigate, recording, working, dictation, header }: Props) {
  const item = (section: Section, icon: IconName, label: string, badge?: ReactNode) => (
    <button
      className={`nav-item${current === section ? " selected" : ""}`}
      aria-current={current === section ? "page" : undefined}
      onClick={() => onNavigate(section)}
    >
      <Icon name={icon} size={16} /> {label}
      {badge}
    </button>
  );

  return (
    <aside className="sidebar">
      <div className="drag" data-tauri-drag-region />
      <div className="sidebar-head">{header}</div>

      <nav className="sidebar-nav">
        {item("home", "home", "Home")}
        {item(
          "meetings",
          "users",
          "Meetings",
          recording ? (
            <span className="badge pill red">
              <span className="rec-dot pulsing" /> Recording
            </span>
          ) : working > 0 ? (
            <span className="badge pill accent">
              <span className="spinner" style={{ width: 10, height: 10 }} /> Working
            </span>
          ) : undefined,
        )}
        {item(
          "dictation",
          "mic",
          "Dictations",
          dictation === "listening" ? (
            <span className="badge pill red">
              <span className="rec-dot pulsing" /> Listening
            </span>
          ) : dictation === "processing" ? (
            <span className="badge pill accent">
              <span className="spinner" style={{ width: 10, height: 10 }} /> Typing
            </span>
          ) : undefined,
        )}
        {item("insights", "chart", "Insights")}
      </nav>

      <nav className="sidebar-nav sidebar-foot">{item("settings", "settings", "Settings")}</nav>
    </aside>
  );
}
