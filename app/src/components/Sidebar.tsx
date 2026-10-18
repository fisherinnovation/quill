import { type ReactNode } from "react";
import { type DictationPhase, isMac } from "../api";
import { Icon, type IconName } from "./Icon";

/** The pages the sidebar leads to. */
export type Section = "home" | "meetings" | "dictation" | "help" | "settings";

/** What a section is busy with, shown as a badge (a dot when collapsed). */
interface Status {
  text: string;
  tone: "red" | "accent";
}

/** The shortcut that collapses and expands the sidebar, as shown in its tooltip. */
export const TOGGLE_SHORTCUT = isMac ? "⌘\\" : "Ctrl+\\";

interface Props {
  /** The page showing (or the list the open item belongs to), to highlight it. */
  current: Section | null;
  onNavigate: (section: Section) => void;
  /** A meeting is being recorded. */
  recording: boolean;
  /** Meetings being transcribed or summarized. */
  working: number;
  dictation: DictationPhase;
  /** Icons only, each named by a tooltip. */
  collapsed: boolean;
  onToggle: () => void;
  /** Shown above the navigation (the record button). */
  header: ReactNode;
}

/** Navigation only: the meetings and dictations are listed on their own pages. */
export function Sidebar({ current, onNavigate, recording, working, dictation, collapsed, onToggle, header }: Props) {
  const item = (section: Section, icon: IconName, label: string, status?: Status) => {
    const name = status ? `${label} · ${status.text}` : label;
    return (
      <button
        className={`nav-item${current === section ? " selected" : ""}`}
        aria-current={current === section ? "page" : undefined}
        aria-label={collapsed ? name : undefined}
        data-tooltip={collapsed ? name : undefined}
        onClick={() => onNavigate(section)}
      >
        <Icon name={icon} size={16} />
        <span className="nav-label">{label}</span>
        {status &&
          (collapsed ? (
            <span className={`nav-dot ${status.tone}`} />
          ) : (
            <span className={`badge pill ${status.tone}`}>
              {status.tone === "red" ? (
                <span className="rec-dot pulsing" />
              ) : (
                <span className="spinner" style={{ width: 10, height: 10 }} />
              )}{" "}
              {status.text}
            </span>
          ))}
      </button>
    );
  };

  const toggleLabel = `${collapsed ? "Expand" : "Collapse"} sidebar (${TOGGLE_SHORTCUT})`;
  return (
    <aside className={`sidebar${collapsed ? " collapsed" : ""}`}>
      <div className="drag" data-tauri-drag-region />
      <div className="sidebar-head">{header}</div>

      <nav className="sidebar-nav">
        {item("home", "home", "Home")}
        {item(
          "meetings",
          "users",
          "Meetings",
          recording
            ? { text: "Recording", tone: "red" }
            : working > 0
              ? { text: "Writing up", tone: "accent" }
              : undefined,
        )}
        {item(
          "dictation",
          "mic",
          "Dictations",
          dictation === "listening"
            ? { text: "Listening", tone: "red" }
            : dictation === "processing"
              ? { text: "Typing", tone: "accent" }
              : undefined,
        )}
      </nav>

      <nav className="sidebar-nav sidebar-foot">
        {item("help", "help", "Help")}
        {item("settings", "settings", "Settings")}
        <button
          className="nav-item sidebar-toggle"
          aria-label={toggleLabel}
          aria-expanded={!collapsed}
          data-tooltip={toggleLabel}
          onClick={onToggle}
        >
          <Icon name="sidebar" size={16} />
        </button>
      </nav>
    </aside>
  );
}
