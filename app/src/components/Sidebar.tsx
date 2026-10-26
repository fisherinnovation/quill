import { type ReactNode } from "react";
import { type DictationPhase, isMac } from "../api";
import { Icon, type IconName } from "./Icon";
import { SEARCH_SHORTCUT } from "./SearchPalette";

/** The pages the sidebar leads to. */
export type Section = "home" | "meetings" | "actions" | "dictation" | "help" | "settings";

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
  /** Your open action items, counted beside Action items. */
  openActions: number;
  /** Icons only, each named by a tooltip. */
  collapsed: boolean;
  onToggle: () => void;
  /** Opens search across everything. */
  onSearch: () => void;
  /** Shown above the navigation (the record button). */
  header: ReactNode;
}

/** Navigation only: the meetings and dictations are listed on their own pages. */
export function Sidebar({
  current,
  onNavigate,
  recording,
  working,
  dictation,
  openActions,
  collapsed,
  onToggle,
  onSearch,
  header,
}: Props) {
  const item = (section: Section, icon: IconName, label: string, status?: Status, count?: number) => {
    const name = status ? `${label} · ${status.text}` : count ? `${label} · ${count} open` : label;
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
        {!status && !collapsed && count ? <span className="nav-count">{count}</span> : null}
      </button>
    );
  };

  const toggleLabel = `${collapsed ? "Expand" : "Collapse"} sidebar (${TOGGLE_SHORTCUT})`;
  return (
    <aside className={`sidebar${collapsed ? " collapsed" : ""}`}>
      <div className="drag" data-tauri-drag-region />
      <div className="sidebar-head">{header}</div>

      {/* Search opens over whatever page is showing, so it sits apart from the pages. */}
      <div className="sidebar-nav sidebar-actions">
        <button
          className="nav-item"
          aria-label={collapsed ? `Search (${SEARCH_SHORTCUT})` : undefined}
          aria-keyshortcuts={isMac ? "Meta+K" : "Control+K"}
          data-tooltip={collapsed ? `Search (${SEARCH_SHORTCUT})` : undefined}
          onClick={onSearch}
        >
          <Icon name="search" size={16} />
          <span className="nav-label">Search</span>
          {!collapsed && <span className="nav-shortcut">{SEARCH_SHORTCUT}</span>}
        </button>
      </div>

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
        {item("actions", "checklist", "Action items", undefined, openActions)}
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
