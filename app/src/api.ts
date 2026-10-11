// Typed wrappers around the Rust commands in src-tauri/src/commands.rs.
// The shapes mirror the Rust structs (serde keeps their snake_case names).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Meeting {
  id: string;
  title: string;
  started_at: string; // RFC 3339
  duration_ms: number;
  has_transcript: boolean;
  has_summary: boolean;
}

export interface Segment {
  start_ms: number;
  end_ms: number;
  text: string;
  speaker?: string | null;
}

export interface StoredSummary {
  markdown: string;
  model: string;
  prompt_version: string;
  prompt_tokens: number;
  completion_tokens: number;
  cost_usd: number | null;
  latency_ms: number;
  created_at: string;
}

export interface MeetingDetail {
  meeting: Meeting;
  segments: Segment[];
  summaries: StoredSummary[]; // newest first
}

export interface SearchHit {
  meeting_id: string;
  title: string;
  start_ms: number;
  speaker: string | null;
  snippet: string; // matches wrapped in [ ]
}

export interface Settings {
  stt_model: string;
  language: string | null;
  vad: boolean;
  summary_model: string;
  zdr: boolean;
  auto_summarize: boolean;
  /** Changed only via chooseRecordingsDir/resetRecordingsDir. */
  recordings_dir: string | null;
  onboarded: boolean;
  dictation: DictationSettings;
}

export interface Replacement {
  from: string;
  to: string;
}

export interface DictationSettings {
  enabled: boolean;
  /** e.g. "Alt+Space" */
  hotkey: string;
  /** Language code, "auto", or null for the system language. */
  language: string | null;
  polish: boolean;
  polish_model: string;
  vocabulary: string[];
  replacements: Replacement[];
  keep_history: boolean;
  app_styles: AppStyle[];
}

export type Style = "default" | "casual" | "formal" | "technical";

export interface AppStyle {
  app_id: string;
  name: string;
  style: Style;
}

/** An app dictated into, with the style in effect there. */
export interface AppStyleInfo extends AppStyle {
  /** What Quill uses when the user hasn't chosen. */
  built_in: Style;
  words: number;
}

export interface Insights {
  dictation: {
    count: number;
    words: number;
    words_last_7_days: number;
    speaking_wpm: number | null;
    minutes_saved: number;
    median_latency_ms: number | null;
    polished_share: number;
  };
  meetings: {
    count: number;
    total_ms: number;
    count_last_7_days: number;
    ms_last_7_days: number;
    average_ms: number | null;
    my_talk_share: number | null;
    summaries: number;
    summary_cost_usd: number;
  };
  streak_days: number;
  /** Last 30 days, oldest first; `date` is a local YYYY-MM-DD. */
  days: { date: string; words: number; dictations: number; meeting_ms: number }[];
  /** Dictations by local hour, 0–23. */
  hours: number[];
  typing_wpm: number;
  /** Apps dictated into, most words first. */
  apps: { app_id: string; name: string; dictations: number; words: number }[];
}

export type DictationPhase = "idle" | "listening" | "processing";

export interface DictationState {
  phase: DictationPhase;
  error: string | null;
}

export interface DictationEntry {
  id: number;
  created_at: string;
  text: string;
  /** Whisper's text, when the polish step changed it. */
  raw: string | null;
  polish_model: string | null;
  audio_ms: number;
  latency_ms: number;
  /** False: left on the clipboard (no permission to paste). */
  pasted: boolean;
  /** The app it was typed into. */
  app: string | null;
}

export interface StorageInfo {
  path: string;
  is_default: boolean;
  /** False if the folder is missing (e.g. a disconnected drive). */
  available: boolean;
  bytes: number;
}

export interface TrackAccess {
  device: string;
  heard: boolean;
  peak_db: number | null;
}

export interface AccessReport {
  microphone: TrackAccess;
  system_audio: TrackAccess;
}

export interface RecordingState {
  recording: boolean;
  meeting_id: string | null;
  elapsed_ms: number;
}

export type Stage = "downloading" | "transcribing" | "summarizing" | "done" | "failed";

/** Background processing progress for one meeting. */
export interface Progress {
  meeting_id: string;
  stage: Stage;
  fraction: number | null;
  message: string | null;
}

export const api = {
  listMeetings: () => invoke<Meeting[]>("list_meetings"),
  getMeeting: (id: string) => invoke<MeetingDetail>("get_meeting", { id }),
  search: (query: string) => invoke<SearchHit[]>("search_transcripts", { query }),
  renameMeeting: (id: string, title: string) => invoke<void>("rename_meeting", { id, title }),
  deleteMeeting: (id: string) => invoke<void>("delete_meeting", { id }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  recordingState: () => invoke<RecordingState>("recording_state"),
  startRecording: () => invoke<Meeting>("start_recording"),
  stopRecording: () => invoke<string>("stop_recording"),
  /** transcribe=true: transcribe (+ summarize per settings); false: summarize only. */
  processMeeting: (id: string, transcribe: boolean) => invoke<void>("process_meeting", { id, transcribe }),
  /** "keychain" | "environment", or null if no key is configured. Never reads the key. */
  openrouterKeyStatus: () => invoke<string | null>("openrouter_key_status"),
  setOpenrouterKey: (key: string) => invoke<void>("set_openrouter_key", { key }),
  deleteOpenrouterKey: () => invoke<void>("delete_openrouter_key"),
  storageInfo: () => invoke<StorageInfo>("storage_info"),
  /** Opens a folder picker and moves all recordings there; null if cancelled. */
  chooseRecordingsDir: () => invoke<StorageInfo | null>("choose_recordings_dir"),
  resetRecordingsDir: () => invoke<StorageInfo>("reset_recordings_dir"),
  showRecordingsDir: () => invoke<void>("show_recordings_dir"),
  showLogs: () => invoke<void>("show_logs"),
  getInsights: () => invoke<Insights>("get_insights"),
  resetDictationStats: () => invoke<void>("reset_dictation_stats"),
  /** Opens a save dialog; resolves to the saved path, or null if cancelled. */
  exportSummary: (id: string) => invoke<string | null>("export_summary", { id }),
  revealFile: (path: string) => invoke<void>("reveal_file", { path }),
  /** Records ~4 s from mic + system audio while playing a chime. */
  checkAudioAccess: () => invoke<AccessReport>("check_audio_access"),
  openPrivacySettings: (pane: "microphone" | "system_audio") => invoke<void>("open_privacy_settings", { pane }),
  finishOnboarding: () => invoke<void>("finish_onboarding"),
  dictationState: () => invoke<DictationState>("dictation_state"),
  /** Saves and rebinds the hotkey; rejects (keeping the old settings) if it can't be bound. */
  saveDictationSettings: (dictation: DictationSettings) => invoke<void>("save_dictation_settings", { dictation }),
  listDictations: () => invoke<DictationEntry[]>("list_dictations"),
  deleteDictation: (id: number) => invoke<void>("delete_dictation", { id }),
  clearDictations: () => invoke<void>("clear_dictations"),
  /** macOS Accessibility: needed to simulate the paste keystroke. */
  pastePermission: () => invoke<boolean>("paste_permission"),
  requestPastePermission: () => invoke<boolean>("request_paste_permission"),
  systemLanguage: () => invoke<string | null>("system_language"),
  dictationApps: () => invoke<AppStyleInfo[]>("dictation_apps"),
};

export const isMac = navigator.userAgent.includes("Mac");
/** The OS file manager's name, for "Show in …" buttons. */
export const fileManager = isMac ? "Finder" : "Explorer";

// Events emitted by the Rust side (see commands.rs / pipeline.rs).
export const events = {
  onRecording: (cb: (s: RecordingState) => void): Promise<UnlistenFn> =>
    listen<RecordingState>("recording-state", (e) => cb(e.payload)),
  onProgress: (cb: (p: Progress) => void): Promise<UnlistenFn> =>
    listen<Progress>("meeting-progress", (e) => cb(e.payload)),
  onDictation: (cb: (s: DictationState) => void): Promise<UnlistenFn> =>
    listen<DictationState>("dictation-state", (e) => cb(e.payload)),
  onDictationEntry: (cb: (entry: DictationEntry) => void): Promise<UnlistenFn> =>
    listen<DictationEntry>("dictation-entry", (e) => cb(e.payload)),
  /** Mic peak 0..1, sent only to the indicator window while listening. */
  onLevel: (cb: (level: number) => void): Promise<UnlistenFn> =>
    listen<number>("dictation-level", (e) => cb(e.payload)),
};

/** "Alt+Space" → "⌥ Space" on macOS; unchanged elsewhere. */
export function hotkeyLabel(hotkey: string): string {
  if (!isMac) return hotkey.replace(/\+/g, " + ");
  const symbols: Record<string, string> = {
    alt: "⌥",
    option: "⌥",
    shift: "⇧",
    ctrl: "⌃",
    control: "⌃",
    cmd: "⌘",
    command: "⌘",
    super: "⌘",
    meta: "⌘",
    commandorcontrol: "⌘",
    cmdorctrl: "⌘",
  };
  return hotkey
    .split("+")
    .map((part) => symbols[part.trim().toLowerCase()] ?? part.trim())
    .join(" ");
}

/** `mm:ss`, or `h:mm:ss` past an hour. */
export function clock(ms: number): string {
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

/** "Oct 11, 2:15 PM" in the user's locale. */
export function shortDate(rfc3339: string): string {
  return new Date(rfc3339).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function minutes(ms: number): string {
  const m = Math.round(ms / 60000);
  return m < 1 ? "<1 min" : m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
}

/** "1.2 GB", "340 MB". */
export function fileSize(bytes: number): string {
  const units = ["bytes", "KB", "MB", "GB", "TB"];
  let n = bytes;
  let i = 0;
  while (n >= 1000 && i < units.length - 1) {
    n /= 1000;
    i++;
  }
  return `${i === 0 ? n : n.toFixed(n < 10 ? 1 : 0)} ${units[i]}`;
}

/** Tauri command errors arrive as plain strings. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

// Same rules as quill_core::meeting::turns (keep in sync).
const TURN_MAX_GAP_MS = 5_000;
const TURN_MAX_LEN_MS = 60_000;

/** Joins consecutive segments by the same speaker into readable turns. */
export function turns(segments: Segment[]): Segment[] {
  const out: Segment[] = [];
  for (const s of segments) {
    const last = out[out.length - 1];
    if (
      last &&
      last.speaker === s.speaker &&
      s.start_ms <= last.end_ms + TURN_MAX_GAP_MS &&
      s.end_ms - last.start_ms <= TURN_MAX_LEN_MS
    ) {
      last.text += " " + s.text;
      last.end_ms = Math.max(last.end_ms, s.end_ms);
    } else {
      out.push({ ...s });
    }
  }
  return out;
}
