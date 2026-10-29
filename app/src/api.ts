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
  /** Transcribed, and nothing was said (unless the user chose to keep it). */
  empty: boolean;
}

/** What the Meetings page shows about a meeting beyond its title. */
export interface MeetingOverview {
  id: string;
  /**
   * Who spoke most in each slice of the meeting: 0 nobody, 1 you, 2 and up
   * the people in `people` (2 is people[0]). Empty without a transcript.
   */
  speaker: number[];
  /** How much of each slice had speech, 0–100. */
  level: number[];
  /** The people on the call, in order of first appearance (not you). */
  people: string[];
  /** Your share of the speaking time, 0–1. */
  my_share: number | null;
  /** The summary's first TL;DR point, as plain text. */
  gist: string | null;
  action_items: number;
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

/** An action item from a meeting's notes (quill_core::library::ActionItem). */
export interface ActionItem {
  id: number;
  meeting_id: string;
  meeting_title: string;
  meeting_started_at: string;
  /** "You", "Priya"…, or null if the notes named no one. */
  owner: string | null;
  /** The owner is the person recording ("You"). */
  mine: boolean;
  task: string;
  /** As written, e.g. "by Thursday". */
  due_text: string | null;
  /** YYYY-MM-DD, worked out from due_text and the meeting's day. */
  due_date: string | null;
  /** When it was ticked off; null while open. */
  done_at: string | null;
}

export interface SearchHit {
  meeting_id: string;
  title: string;
  start_ms: number;
  speaker: string | null;
  snippet: string; // matches wrapped in [ ]
  /** Found by its words, its meaning (search by meaning), or both. */
  matched: SearchMatch;
}

/** How a search result was found: its words, its meaning, or both. */
export type SearchMatch = "words" | "meaning" | "both";

/** What a search covers: everything, or one kind of thing. */
export type SearchScope = "all" | "meetings" | "notes" | "actions" | "dictations";

/** Up to the limit asked for, best first, and whether there were more. */
export interface SearchGroup<T> {
  hits: T[];
  more: boolean;
}

/** A meeting found by its title. */
export interface MeetingTitleHit {
  meeting_id: string;
  title: string;
  started_at: string;
  snippet: string; // matches wrapped in [ ]
}

/** A section of a meeting's notes. */
export interface NoteHit {
  meeting_id: string;
  title: string;
  started_at: string;
  /** The section or topic, e.g. "Decisions". */
  heading: string | null;
  snippet: string;
  matched: SearchMatch;
}

export interface ActionItemHit {
  action_id: number;
  meeting_id: string;
  /** Its meeting's title. */
  title: string;
  owner: string | null;
  task: string;
  done: boolean;
  snippet: string;
  matched: SearchMatch;
}

/** A dictation from the history. */
export interface DictationHit {
  dictation_id: number;
  created_at: string;
  app: string | null;
  app_id: string | null;
  snippet: string;
  matched: SearchMatch;
}

/** A day's meetings summarized from their notes (quill_core::daily). */
export interface DailySummary {
  /** The local day, YYYY-MM-DD. */
  day: string;
  markdown: string;
  model: string;
  prompt_version: string;
  /** When it was written (RFC 3339). */
  created_at: string;
  cost_usd: number | null;
  /** How many meetings' notes it was written from. */
  meetings: number;
  /** The day's notes changed since: rewritten notes, or another meeting's. */
  outdated: boolean;
}

/** What a search across the library found, by kind. */
export interface SearchResults {
  /** Meetings whose title matches. */
  meetings: SearchGroup<MeetingTitleHit>;
  /** Moments in transcripts. */
  moments: SearchGroup<SearchHit>;
  notes: SearchGroup<NoteHit>;
  actions: SearchGroup<ActionItemHit>;
  dictations: SearchGroup<DictationHit>;
}

export interface Settings {
  stt_model: string;
  language: string | null;
  vad: boolean;
  summary_model: string;
  zdr: boolean;
  auto_summarize: boolean;
  /** Split "Others" into Speaker 1, 2… by voice. */
  identify_speakers: boolean;
  /** Changed only via chooseRecordingsDir/resetRecordingsDir. */
  recordings_dir: string | null;
  onboarded: boolean;
  /** Look for a new version once a day (installing is always the user's call). */
  check_for_updates: boolean;
  theme: Theme;
  /** Offer to record when a call app starts using the mic (macOS for now). */
  offer_to_record_calls: boolean;
  /** Live notes in a side panel while recording. */
  live_notes: boolean;
  /** A system notification when a meeting's notes are ready, if Quill isn't in front. */
  notify_notes_ready: boolean;
  /** Remember the voices of people named on calls and name them in later meetings. */
  recognize_voices: boolean;
  /** Search by meaning as well as words, once the model is downloaded. */
  semantic_search: boolean;
  /** Let AI assistants such as Claude read meetings: served on a localhost URL while on. Off unless turned on. */
  assistant_access: boolean;
  /** The localhost port assistants connect to. */
  assistant_port: number;
  /** Which graphics card transcribes (Windows; quill_stt::GpuChoice). */
  gpu: GpuChoice;
  dictation: DictationSettings;
}

/** Which graphics card transcribes: a card is remembered by name. */
export type GpuChoice = { kind: "automatic" } | { kind: "cpu" } | { kind: "named"; name: string };

/** A graphics card speech recognition can run on (quill_stt::Gpu). */
export interface Gpu {
  name: string;
  integrated: boolean;
  /** 0 if unknown; for integrated graphics, the shared memory it can use. */
  memory_bytes: number;
}

/** What Settings → Models offers (src-tauri/src/models.rs). */
export interface GraphicsCards {
  /** False with Metal (macOS), where there's one GPU. */
  choosable: boolean;
  cards: Gpu[];
  /** The card Automatic picks, if any. */
  automatic: string | null;
}

/** A live note (quill_core::live_meeting::Item). */
export interface LiveItem {
  id: string;
  kind: "ask" | "follow_up" | "decision";
  text: string;
  who: string | null;
  done: boolean;
}

/** What the live notes panel shows (src-tauri/src/live_notes.rs). */
export interface LiveState {
  active: boolean;
  notes: { gist: string; items: LiveItem[] };
  /** Recording time (ms) of the last update and of the next. */
  updated_ms: number | null;
  next_ms: number | null;
  updating: boolean;
  /** The latest line heard. */
  heard: string | null;
  error: string | null;
  collapsed: boolean;
  /** After the recording stops: the meeting being written up, followed by a small card. */
  wrapping: { meeting_id: string; title: string } | null;
}

/** What the call prompt asks (src-tauri/src/calls.rs). */
export type CallPrompt = { kind: "record"; app: string } | { kind: "stop"; app: string };

/** What's been said so far while dictating (quill_core::live::Caption). */
export interface Caption {
  /** Locked in: won't change. */
  committed: string;
  /** The latest guess at the rest: may still change. */
  tentative: string;
}

/** "system" follows the OS, including when it switches. */
export type Theme = "system" | "light" | "dark";

/**
 * Shows this window's page in `theme`: styles.css keys the colours off
 * `data-theme`, and without it follows the OS. (Rust sets the native window
 * chrome to match; see theme.rs.)
 */
export function applyTheme(theme: Theme) {
  const root = document.documentElement;
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
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
  /** Speech model for dictation; null uses the meetings' (Settings.stt_model). */
  stt_model: string | null;
  polish: boolean;
  polish_model: string;
  /** What the user tells the clean-up model about themselves. */
  about: string;
  vocabulary: string[];
  replacements: Replacement[];
  keep_history: boolean;
  /** Keep each dictation's audio and typed text, to measure accuracy. */
  save_samples: boolean;
  /** Show what's being said, above the dictation pill, while speaking. */
  live_captions: boolean;
  /** The sounds dictation plays to start, stop and cancel. */
  sound: Sounds;
  app_styles: AppStyle[];
}

/** A set of dictation sounds (quill_audio::cue::Sounds). */
export type Sounds = "classic" | "soft" | "bubble" | "wood" | "glass" | "marimba" | "chirp" | "digital" | "off";

/** Saved dictation samples (Dictation → Save samples). */
export interface SamplesUsage {
  count: number;
  bytes: number;
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

/** From the end of speech to the text typed: the wait the user notices. */
export interface WaitInsights {
  count: number;
  median_ms: number;
  p90_ms: number;
  /** How the median splits: still holding the key after speaking, then Quill. */
  held_ms: number;
  quill_ms: number;
  last_7_days_ms: number | null;
  previous_7_days_ms: number | null;
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
    /** Time to insert over the last 30 days (quill_core::insights::WaitInsights); null until measured. */
    wait: WaitInsights | null;
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
  days: {
    date: string;
    words: number;
    dictations: number;
    meeting_ms: number;
    /** That day's median and 90th-percentile time to insert, if measured. */
    wait_ms: number | null;
    wait_p90_ms: number | null;
  }[];
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
  /** That app's identifier (bundle id or executable), for its icon. */
  app_id: string | null;
  /** From the end of speech to the text typed; null if it couldn't be told. */
  wait_ms: number | null;
  /** The user's fixed version, if they corrected it. */
  corrected: string | null;
}

/** A rule suggested by the user's corrections. */
export interface Suggestion {
  id: number;
  kind: "vocabulary" | "replacement";
  /** Empty for vocabulary. */
  from: string;
  to: string;
  app: string | null;
  /** How many corrections suggested it. */
  seen: number;
  status: "pending" | "accepted" | "dismissed";
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
  /** Paused: nothing is recorded and the clock stands still. */
  paused: boolean;
  meeting_id: string | null;
  /** Time recorded, pauses left out. */
  elapsed_ms: number;
}

export type Stage = "downloading" | "transcribing" | "identifying" | "summarizing" | "done" | "failed";

/** Background processing progress for one meeting. */
export interface Progress {
  meeting_id: string;
  stage: Stage;
  /** How far through the stage, 0–1, when known. */
  fraction: number | null;
  message: string | null;
  /** The whole write-up: steps, time left, lines heard, what came out. */
  report: WriteUp | null;
  /** The meeting's title, on the last event (done or failed). */
  title: string | null;
}

/** The steps of a meeting's write-up after recording (quill_core::processing). */
export type StepKind = "prepare" | "you" | "call" | "speakers" | "notes";
export type StepStatus = "pending" | "active" | "done" | "skipped" | "failed";

export interface WriteUpStep {
  kind: StepKind;
  status: StepStatus;
  fraction: number | null;
  /** Time taken (done) or so far (active). */
  ms: number | null;
}

export interface WriteUp {
  steps: WriteUpStep[];
  elapsed_ms: number;
  /** Roughly how long is left, once there's enough to go on. */
  eta_ms: number | null;
  /** The last few lines heard, oldest first. */
  heard: { you: boolean; text: string }[];
  outcome: Outcome | null;
}

/** A saved dictation sample (quill_core::samples::Sample). */
export interface Sample {
  name: string;
  typed: string;
  /** Whisper's text, before clean-up. */
  whisper: string;
  app: string | null;
  /** The clean-up model, if it changed the text. */
  polish_model: string | null;
  duration_ms: number;
  /** What should have been typed, once reviewed. */
  reference: string | null;
  /** What was typed wrong, against the reference: "quil→quill", "-the", "+um". */
  errors: string[];
}

/** Word error counts (quill_core::wer::Tally). */
export interface WordTally {
  reference_words: number;
  substitutions: number;
  deletions: number;
  insertions: number;
}

export interface SampleReview {
  samples: Sample[];
  accuracy: {
    reviewed: number;
    typed: WordTally;
    whisper: WordTally;
    cleaned: number;
    /** The commonest mistakes in what was typed, with how often. */
    common: [string, number][];
  };
}

/** What came out of a finished write-up. */
export interface Outcome {
  words: number;
  /** Everyone who spoke, you included. */
  people: number;
  action_items: number;
  decisions: number;
  notes: boolean;
  took_ms: number;
}

/** What the About window shows. */
export interface AboutInfo {
  version: string;
  /** Commit the app was built from, and its date; empty outside a git checkout. */
  commit: string;
  commit_date: string;
  debug: boolean;
  os: string;
  arch: string;
  speech_model: string;
  /** Dictation's speech model, when it isn't speech_model. */
  dictation_model: string | null;
  /** null when clean-up is off. */
  cleanup_model: string | null;
  summary_model: string;
  identify_speakers: boolean;
  data_dir: string;
}

/** One person's share of a meeting (see quill_core::stats). */
export interface SpeakerStats {
  name: string;
  talk_ms: number;
  /** Of everyone's talking time, 0–1. */
  share: number;
  words: number;
  wpm: number | null;
  turns: number;
  questions: number;
  longest_stretch_ms: number;
  fillers: number;
  /** Words this person said far more than anyone else. */
  signature_words: string[];
}

/** One person talking without a long pause or anyone cutting in. */
export interface Stretch {
  speaker: string;
  start_ms: number;
  end_ms: number;
  words: number;
  preview: string;
}

export interface WordCount {
  text: string;
  count: number;
}

export interface MeetingStats {
  span_ms: number;
  speech_ms: number;
  words: number;
  wpm: number | null;
  turns: number;
  questions: number;
  /** "Me" first, then in order of first appearance (the transcript's colour order). */
  speakers: SpeakerStats[];
  stretches: Stretch[];
  longest_stretch: Stretch | null;
  longest_silence: { start_ms: number; end_ms: number } | null;
  busiest_pair: { a: string; b: string; handovers: number } | null;
  top_words: WordCount[];
  top_phrases: WordCount[];
}

export interface DictationStats {
  words: number;
  speaking_wpm: number | null;
  saved_ms: number;
  typing_wpm: number;
  /** Words Whisper heard, when clean-up changed them. */
  raw_words: number | null;
  fillers_removed: number;
  /** The user's average speaking speed, to compare. */
  usual_wpm: number | null;
}

/** Where updating stands (see updates.rs). */
export type UpdateStatus =
  | { state: "idle" }
  | { state: "checking" }
  | { state: "up_to_date" }
  | { state: "available"; version: string }
  | { state: "downloading"; version: string; done: number; total: number | null }
  | { state: "installing"; version: string };

export interface UpdateInfo {
  current_version: string;
  status: UpdateStatus;
  /** The last finished check, in Unix milliseconds. */
  checked_at: number | null;
}

/** Pages the app links to; the URLs live in Rust (about.rs). */
export type AboutLink = "repository" | "releases" | "report_problem" | "security" | "license" | "openrouter_keys";

/** Model choices for summaries or clean-up (see llm_models.rs). */
export interface ModelChoices {
  recommended: { id: string; label: string; note: string; per_hour: number | null }[];
  /** "ollama:<name>" for each installed model; null if Ollama isn't running. */
  ollama: string[] | null;
  /** OpenRouter's catalog; empty if it couldn't be loaded. */
  catalog: {
    id: string;
    name: string;
    input_per_million: number | null;
    output_per_million: number | null;
    context_length: number | null;
  }[];
}

/** Quill's disk footprint, by part. */
export interface Footprint {
  total: number;
  parts: { key: "recordings" | "models" | "library" | "logs" | "app"; bytes: number }[];
}

export interface ModelEntry {
  name: string;
  kind: "speech" | "vad" | "speakers" | "search";
  description: string;
  size_bytes: number;
  downloaded: boolean;
  /** Needed by the saved settings. */
  in_use: boolean;
  downloading: boolean;
  /** One of the models "Tell speakers apart" uses. */
  for_speakers: boolean;
  /** The speech model this file is part of (Parakeet's four files), e.g. "parakeet-v2". */
  part_of: string | null;
}

export interface ModelsInfo {
  dir: string;
  models: ModelEntry[];
  bytes_on_disk: number;
  /** Still to download for the saved settings. */
  missing_bytes: number;
}

/** Where search by meaning stands (`semantic-index` events). */
export interface SemanticStatus {
  /** Settings → Meetings → Search by meaning. */
  enabled: boolean;
  /** The search model is downloaded. */
  model_ready: boolean;
  /** What there is to index (transcripts, notes, action items, dictations), and how much is. */
  indexed: number;
  total: number;
  /** Indexing waits for a recording, transcription or dictation. */
  paused: boolean;
  error: string | null;
}

/** The localhost server for AI assistants (`assistant-server` events). */
export interface AssistantServer {
  running: boolean;
  /** Where assistants connect, while running. */
  url: string | null;
  /** The bearer token they must send, while running. */
  token: string | null;
  /** Why it isn't running although it's on (the port is taken, the keychain refused). */
  error: string | null;
}

/** A `model-download` event. */
export interface DownloadProgress {
  name: string;
  done: number;
  total: number;
  finished: boolean;
  error: string | null;
}

export const api = {
  listMeetings: () => invoke<Meeting[]>("list_meetings"),
  meetingOverviews: () => invoke<MeetingOverview[]>("meeting_overviews"),
  callPrompt: () => invoke<CallPrompt | null>("call_prompt"),
  answerCallPrompt: (answer: "record" | "stop" | "dismiss") => invoke<void>("answer_call_prompt", { answer }),
  liveNotesState: () => invoke<LiveState>("live_notes_state"),
  liveNotesRefresh: () => invoke<void>("live_notes_refresh"),
  liveNotesDismiss: (id: string) => invoke<void>("live_notes_dismiss", { id }),
  liveNotesTick: (id: string, done: boolean) => invoke<void>("live_notes_tick", { id, done }),
  liveNotesCollapse: (collapsed: boolean) => invoke<void>("live_notes_collapse", { collapsed }),
  /** The wrap-up card's height: the panel fits itself to it. */
  liveNotesFit: (height: number) => invoke<void>("live_notes_fit", { height }),
  liveNotesClose: () => invoke<void>("live_notes_close"),
  /** Opens the meeting being wrapped up in Quill's window. */
  liveNotesOpenMeeting: () => invoke<void>("live_notes_open_meeting"),
  getMeeting: (id: string) => invoke<MeetingDetail>("get_meeting", { id }),
  meetingStats: (id: string) => invoke<MeetingStats>("meeting_stats", { id }),
  /** Searches meetings, notes, action items and dictations: a few of each, or more of one `scope`. */
  search: (query: string, scope: SearchScope = "all") => invoke<SearchResults>("search_library", { query, scope }),
  semanticStatus: () => invoke<SemanticStatus>("semantic_status"),
  assistantServer: () => invoke<AssistantServer>("assistant_server"),
  /** A new token: assistants set up with the old one stop working until given this one. */
  renewAssistantToken: () => invoke<AssistantServer>("renew_assistant_token"),
  /** Every stored daily summary, marked outdated where the day's notes changed. */
  dailySummaries: () => invoke<DailySummary[]>("daily_summaries"),
  /** Writes a day's summary (YYYY-MM-DD, local) from its meetings' notes; takes a few seconds. */
  summarizeDay: (day: string) => invoke<DailySummary>("summarize_day", { day }),
  /** Loads the search model ahead of the first search. */
  prepareSearch: () => invoke<void>("prepare_search"),
  /** Downloads the search model (progress as `model-download` events). */
  downloadSearchModel: () => invoke<void>("download_search_model"),
  renameMeeting: (id: string, title: string) => invoke<void>("rename_meeting", { id, title }),
  deleteMeeting: (id: string) => invoke<void>("delete_meeting", { id }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  recordingState: () => invoke<RecordingState>("recording_state"),
  startRecording: () => invoke<Meeting>("start_recording"),
  stopRecording: () => invoke<string>("stop_recording"),
  pauseRecording: () => invoke<void>("pause_recording"),
  resumeRecording: () => invoke<void>("resume_recording"),
  /** transcribe=true: transcribe (+ summarize per settings); false: summarize only. */
  processMeeting: (id: string, transcribe: boolean) => invoke<void>("process_meeting", { id, transcribe }),
  /** Re-split the people on the call, into `speakers` if given. Replaces names; progress arrives as events. */
  splitSpeakers: (id: string, speakers: number | null) => invoke<void>("split_speakers", { id, speakers }),
  /** Renaming to a name another speaker has merges them. */
  renameSpeaker: (id: string, from: string, to: string) => invoke<void>("rename_speaker", { id, from, to }),
  /** "keychain" | "environment", or null if no key is configured. Never reads the key. */
  /** Never touches the keychain (which can prompt for the password on macOS). */
  /** "unknown" only on macOS, before the key has been read this session. */
  openrouterKeyStatus: () => invoke<KeySource>("openrouter_key_status"),
  setOpenrouterKey: (key: string) => invoke<void>("set_openrouter_key", { key }),
  /** False if there was no key to remove. May show the keychain prompt. */
  deleteOpenrouterKey: () => invoke<boolean>("delete_openrouter_key"),
  storageInfo: () => invoke<StorageInfo>("storage_info"),
  /** Opens a folder picker and moves all recordings there; null if cancelled. */
  chooseRecordingsDir: () => invoke<StorageInfo | null>("choose_recordings_dir"),
  resetRecordingsDir: () => invoke<StorageInfo>("reset_recordings_dir"),
  showRecordingsDir: () => invoke<void>("show_recordings_dir"),
  modelsInfo: () => invoke<ModelsInfo>("models_info"),
  actionItems: () => invoke<ActionItem[]>("action_items"),
  setActionItemDone: (id: number, done: boolean) => invoke<ActionItem>("set_action_item_done", { id, done }),
  graphicsCards: () => invoke<GraphicsCards>("graphics_cards"),
  /** PNG data URLs by app id; null where there's no icon (src-tauri/src/icons.rs). */
  appIcons: (ids: string[]) => invoke<Record<string, string | null>>("app_icons", { ids }),
  footprint: () => invoke<Footprint>("footprint"),
  /** Recordings with nothing said in them (not recording or processing). */
  emptyRecordings: () => invoke<{ count: number; bytes: number }>("empty_recordings"),
  /** Deletes them all, audio included; returns how many. */
  deleteEmptyRecordings: () => invoke<number>("delete_empty_recordings"),
  /** Keeps one: it stops being flagged as empty. */
  keepEmptyRecording: (id: string) => invoke<void>("keep_empty_recording", { id }),
  modelChoices: (purpose: "summary" | "cleanup") => invoke<ModelChoices>("model_choices", { purpose }),
  aboutInfo: () => invoke<AboutInfo>("about_info"),
  openLink: (link: AboutLink) => invoke<void>("open_link", { link }),
  /** Downloads what the saved settings need, in the background. */
  downloadNeededModels: () => invoke<void>("download_needed_models"),
  downloadModel: (name: string) => invoke<void>("download_model", { name }),
  /** Models the settings use download again when next needed. */
  deleteModel: (name: string) => invoke<ModelsInfo>("delete_model", { name }),
  /** Deletes the model and downloads a fresh, hash-checked copy in the background. */
  redownloadModel: (name: string) => invoke<ModelsInfo>("redownload_model", { name }),
  deleteAllModels: () => invoke<ModelsInfo>("delete_all_models"),
  showModelsDir: () => invoke<void>("show_models_dir"),
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
  /** Turns the dictation shortcut off while a new one is recorded (`true`), and back on. */
  pauseDictationShortcut: (paused: boolean) => invoke<void>("pause_dictation_shortcut", { paused }),
  /** Plays a set of dictation sounds, start then stop. */
  previewDictationSound: (sound: Sounds) => invoke<void>("preview_dictation_sound", { sound }),
  /** Newest first: up to `limit` entries older than `before` (an id), or the newest. */
  listDictations: (before: number | null, limit: number) =>
    invoke<DictationEntry[]>("list_dictations", { before, limit }),
  getDictation: (id: number) => invoke<DictationEntry>("get_dictation", { id }),
  dictationStats: (id: number) => invoke<DictationStats>("dictation_stats", { id }),
  deleteDictation: (id: number) => invoke<void>("delete_dictation", { id }),
  clearDictations: () => invoke<void>("clear_dictations"),
  dictationSamples: () => invoke<SamplesUsage>("dictation_samples"),
  showDictationSamples: () => invoke<void>("show_dictation_samples"),
  /** Deletes the samples, and any references written for them. */
  clearDictationSamples: () => invoke<void>("clear_dictation_samples"),
  /** The saved samples, oldest first, and how accurate the reviewed ones were. */
  sampleReview: () => invoke<SampleReview>("sample_review"),
  /** A sample's audio, as FLAC bytes. */
  sampleAudio: (name: string) => invoke<ArrayBuffer>("sample_audio", { name }),
  /** Saves what should have been typed: the sample's reference. */
  setSampleReference: (name: string, text: string) => invoke<void>("set_sample_reference", { name, text }),
  deleteSample: (name: string) => invoke<void>("delete_sample", { name }),
  /** Saves a fixed version and returns it with all pending and accepted suggestions. */
  correctDictation: (id: number, text: string) =>
    invoke<{ entry: DictationEntry; suggestions: Suggestion[] }>("correct_dictation", { id, text }),
  listSuggestions: () => invoke<Suggestion[]>("list_suggestions"),
  setSuggestionStatus: (id: number, status: Suggestion["status"]) =>
    invoke<Suggestion[]>("set_suggestion_status", { id, status }),
  /** macOS Accessibility: needed to simulate the paste keystroke. */
  pastePermission: () => invoke<boolean>("paste_permission"),
  requestPastePermission: () => invoke<boolean>("request_paste_permission"),
  systemLanguage: () => invoke<string | null>("system_language"),
  dictationApps: () => invoke<AppStyleInfo[]>("dictation_apps"),
  /** macOS's own action for fn / 🌐 ("Press 🌐 key to"); null off macOS. */
  globeKeyAction: () =>
    invoke<"nothing" | "change_input_source" | "show_emoji" | "start_dictation" | "default" | null>(
      "globe_key_action",
    ),
  openKeyboardSettings: () => invoke<void>("open_keyboard_settings"),
  updateInfo: () => invoke<UpdateInfo>("update_info"),
  /** Checks GitHub now, even if daily checks are off. */
  checkForUpdates: () => invoke<UpdateInfo>("check_for_updates"),
  /** Downloads, installs and restarts Quill. Rejects while recording, dictating, etc. */
  installUpdate: () => invoke<void>("install_update"),
  /** The people on a meeting's call that Quill has voices for (src-tauri/src/voices.rs). */
  meetingVoices: (id: string) => invoke<MeetingVoice[]>("meeting_voices", { id }),
  /** Says an automatic name is right; Quill learns this meeting's voice for them. */
  confirmSpeaker: (id: string, label: string) => invoke<void>("confirm_speaker", { id, label }),
  knownVoices: () => invoke<KnownVoice[]>("known_voices"),
  deleteKnownVoice: (id: number) => invoke<void>("delete_known_voice", { id }),
  /** Forgets them all; resolves to how many there were. */
  forgetKnownVoices: () => invoke<number>("forget_known_voices"),
};

export const isMac = navigator.userAgent.includes("Mac");
/** The OS file manager's name, for "Show in …" buttons. */
export const fileManager = isMac ? "Finder" : "Explorer";
/** Where API keys are kept, as the OS calls it. */
export const keyStore = isMac ? "keychain" : "Windows Credential Manager";

/** Where the OpenRouter key comes from, as far as Quill knows. */
export type KeySource = "environment" | "keychain" | "none" | "unknown";

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
  /** The live notes panel's state changed. */
  onLiveNotes: (cb: (state: LiveState) => void): Promise<UnlistenFn> =>
    listen<LiveState>("live-notes", (e) => cb(e.payload)),
  /** The call prompt's question, or null when it hides. */
  onCallPrompt: (cb: (prompt: CallPrompt | null) => void): Promise<UnlistenFn> =>
    listen<CallPrompt | null>("call-prompt", (e) => cb(e.payload)),
  /** The theme setting changed (in any window). */
  onTheme: (cb: (theme: Theme) => void): Promise<UnlistenFn> => listen<Theme>("theme", (e) => cb(e.payload)),
  /** A dictation sample was saved. */
  onDictationSamples: (cb: (usage: SamplesUsage) => void): Promise<UnlistenFn> =>
    listen<SamplesUsage>("dictation-samples", (e) => cb(e.payload)),
  onModelDownload: (cb: (p: DownloadProgress) => void): Promise<UnlistenFn> =>
    listen<DownloadProgress>("model-download", (e) => cb(e.payload)),
  /** Search by meaning's status changed (indexing progress, the model, the setting). */
  onSemanticIndex: (cb: (s: SemanticStatus) => void): Promise<UnlistenFn> =>
    listen<SemanticStatus>("semantic-index", (e) => cb(e.payload)),
  /** The server for AI assistants started, stopped or failed. */
  onAssistantServer: (cb: (s: AssistantServer) => void): Promise<UnlistenFn> =>
    listen<AssistantServer>("assistant-server", (e) => cb(e.payload)),
  /** The meeting list changed outside the meeting view. */
  onLibraryChanged: (cb: () => void): Promise<UnlistenFn> => listen<null>("library-changed", () => cb()),
  /** The backend asks to show a view, e.g. "dictation". */
  onNavigate: (cb: (view: string) => void): Promise<UnlistenFn> =>
    listen<string>("navigate", (e) => cb(e.payload)),
  onUpdate: (cb: (info: UpdateInfo) => void): Promise<UnlistenFn> =>
    listen<UpdateInfo>("update-status", (e) => cb(e.payload)),
  /** Mic peak 0..1, sent only to the indicator window while listening. */
  /** Live captions while dictating (indicator window). */
  onCaption: (cb: (caption: Caption) => void): Promise<UnlistenFn> =>
    listen<Caption>("dictation-caption", (e) => cb(e.payload)),
  onLevel: (cb: (level: number) => void): Promise<UnlistenFn> =>
    listen<number>("dictation-level", (e) => cb(e.payload)),
};

/** `DictationSettings.hotkey` for the fn (🌐) key on its own (macOS). */
export const FN_HOTKEY = "Fn";
/** `DictationSettings.hotkey` for the right Ctrl key on its own (Windows). */
export const RIGHT_CTRL_HOTKEY = "RightCtrl";

/**
 * "Alt+Space" → "⌥ Space" on macOS, "Alt + Space" elsewhere. Names of more
 * than one word keep a non-breaking space, so `Keys` shows them as one key.
 */
export function hotkeyLabel(hotkey: string): string {
  if (!isMac) {
    const names: Record<string, string> = {
      RightCtrl: "Right Ctrl",
      ScrollLock: "Scroll Lock",
      PageUp: "Page Up",
      PageDown: "Page Down",
    };
    return hotkey
      .split("+")
      .map((part) => names[part] ?? part)
      .join(" + ");
  }
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
    fn: "fn",
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

/** A person on a meeting's call that Quill has a voice for (quill_core::voices). */
export interface MeetingVoice {
  label: string;
  /** Named automatically by voice, not yet confirmed or changed. */
  recognized: boolean;
  /** How close the voices were (cosine similarity, up to 1), when recognized. */
  similarity: number | null;
  /** Quill remembers this voice under this name from this meeting. */
  remembered: boolean;
  /** Enough speech in this meeting to remember the voice from. */
  enough_speech: boolean;
}

/** A person Quill recognizes by voice, as Settings lists them. */
export interface KnownVoice {
  id: number;
  name: string;
  voice_model: string;
  /** Meetings their voice was learned from. */
  meetings: number;
  speech_ms: number;
  created_at: string;
  updated_at: string;
}
