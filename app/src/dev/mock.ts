// Browser-only preview of the UI with sample data, for working on the design
// without the Rust side: `npm run dev`, then open http://localhost:1420.
// main.tsx loads this only in dev builds and only outside Tauri.
//
// URL flags pick a state: ?onboarding, ?recording, ?listening, ?processing,
// ?failed, ?nopaste, ?empty, ?light to force the light theme, ?indicator
// for the dictation pill (with ?listening or ?processing), ?others for a
// call not yet split into speakers, ?nomodels for a fresh install, ?fn for
// fn as the dictation key, and ?about for the About window.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

const flags = new URLSearchParams(location.search);
const now = Date.now();
const iso = (msAgo: number) => new Date(now - msAgo).toISOString();
const H = 3_600_000;
const D = 24 * H;

const SUMMARY = `# Product sync — 2026-10-11

## TL;DR
The team agreed to ship the onboarding redesign on **Friday** behind a feature flag, and to move pricing experiments to next sprint while analytics are fixed.

## Decisions
- Ship the onboarding redesign Friday, behind the \`new-onboarding\` flag at 10% of new sign-ups.
- Pause pricing experiments until the events pipeline double-counting is fixed.
- Keep the weekly sync at 10:30 instead of 10:00.

## Action items
- [ ] **You:** send the rollout plan to support by Wednesday.
- [ ] **Priya:** fix the duplicate \`signup_completed\` events (target: Thursday).
- [ ] **Dan:** prepare two pricing-page variants for the next sprint.

## Discussion
- **Onboarding.** Completion rose from 41% to 58% in the usability study; the remaining drop-off is the workspace-invite step, which can be skipped later.
- **Analytics.** Signup numbers have been ~12% too high since the SDK upgrade; Priya traced it to a retry that re-sends the event.
- **Pricing.** Dan wants to test annual-first; agreed to wait for clean data.

## Open questions
- Should the invite step move after the first project is created?
`;

// ?others: the call side as one "Others" track, as before speakers are told apart.
const other = (n: number) => (flags.has("others") ? "Others" : `Speaker ${n}`);
let SEGMENTS = [
  [0, 6, "Me", "Okay, let's get started. Main things today are the onboarding launch and the analytics problem."],
  [7, 15, other(1), "Sure. So the usability study came back and completion went from forty-one to fifty-eight percent."],
  [15, 22, other(1), "Most of the remaining drop-off is the invite step. People don't have teammates ready yet."],
  [23, 31, "Me", "Could we make the invite skippable and nudge them later, after they've created a project?"],
  [32, 38, other(2), "Yes, that's easy. I'd still ship Friday behind the flag and test that separately."],
  [40, 52, "Me", "Agreed. Ten percent of new sign-ups to start. I'll write up the rollout plan for support."],
  [55, 70, other(3), "On analytics: signups have been about twelve percent too high since the SDK upgrade. There's a retry that re-sends the event."],
  [71, 78, "Me", "Then let's hold the pricing experiments until that's fixed. No point testing on bad data."],
  [79, 83, other(2), "Fine by me. I'll prepare two pricing-page variants for next sprint."],
].map(([s, e, speaker, text]) => ({
  start_ms: (s as number) * 1000,
  end_ms: (e as number) * 1000,
  speaker,
  text,
}));

let meetings = flags.has("empty")
  ? []
  : [
      { id: "m1", title: "Product sync", started_at: iso(2 * H), duration_ms: 31 * 60_000, has_transcript: true, has_summary: true },
      { id: "m2", title: "1:1 with Priya", started_at: iso(5 * H), duration_ms: 24 * 60_000, has_transcript: true, has_summary: true },
      { id: "m3", title: "Meeting Oct 10, 4:02 PM", started_at: iso(D + 3 * H), duration_ms: 12 * 60_000, has_transcript: false, has_summary: false },
      { id: "m4", title: "Design review: settings", started_at: iso(D + 6 * H), duration_ms: 48 * 60_000, has_transcript: true, has_summary: true },
      { id: "m5", title: "Customer call — Northwind", started_at: iso(4 * D), duration_ms: 37 * 60_000, has_transcript: true, has_summary: true },
      { id: "m6", title: "Hiring sync", started_at: iso(12 * D), duration_ms: 55 * 60_000, has_transcript: true, has_summary: true },
    ];

const recording = flags.has("recording") ? { recording: true, meeting_id: "m1", elapsed_ms: 754_000 } : null;
const phase = flags.has("listening") ? "listening" : flags.has("processing") ? "processing" : "idle";

const settings = {
  stt_model: "turbo",
  language: null,
  vad: true,
  summary_model: "anthropic/claude-haiku-5.5",
  zdr: false,
  auto_summarize: true,
  identify_speakers: true,
  recordings_dir: null,
  onboarded: !flags.has("onboarding"),
  dictation: {
    enabled: true,
    hotkey: flags.has("fn") ? "Fn" : "Alt+Space",
    language: null,
    polish: true,
    polish_model: "google/gemini-3.5-flash-lite",
    about: "",
    vocabulary: ["Quill", "OpenRouter", "Priya"],
    replacements: [{ from: "open router", to: "OpenRouter" }],
    keep_history: true,
    app_styles: [],
  },
};

const dictations = [
  { id: 3, created_at: iso(4 * 60_000), text: "Sounds good, I'll send the rollout plan to support by Wednesday.", raw: null, polish_model: null, audio_ms: 3900, latency_ms: 310, pasted: true, app: "Slack" },
  { id: 2, created_at: iso(40 * 60_000), text: "For the launch we need three things:\n1. The landing page\n2. The pricing table\n3. The onboarding emails", raw: "For the launch we need three things. First, the landing page. Second, the pricing table. And third, the onboarding emails.", polish_model: "google/gemini-3.5-flash-lite", audio_ms: 9100, latency_ms: 980, pasted: true, app: "Notion" },
  { id: 1, created_at: iso(3 * H), text: "Let's meet at 4 p.m. on Thursday in the big conference room.", raw: "Let's meet at 3, no wait, 4 p.m. on Thursday in the big conference room.", polish_model: "google/gemini-3.5-flash-lite", audio_ms: 6100, latency_ms: 940, pasted: false, app: "Mail" },
];

const fresh = flags.has("nomodels");
let models = [
  ["turbo", "speech", "large-v3 with a 4-layer decoder; near large-v3 accuracy, much faster", 1_624_555_275, !fresh, true, false],
  ["small", "speech", "244M params", 487_601_967, !fresh, false, false],
  ["vad", "vad", "Silero VAD v6.2: skips silence so Whisper doesn't invent text", 885_098, !fresh, true, false],
  ["segmentation", "speakers", "pyannote segmentation 3.0: where speaker turns are", 5_992_913, !fresh, true, true],
  ["voice-titanet-small", "speakers", "NVIDIA NeMo TitaNet small (CC BY 4.0)", 40_257_283, !fresh, true, true],
  ["voice-eres2net", "speakers", "3D-Speaker ERes2Net, VoxCeleb (Apache-2.0)", 26_485_263, !fresh, false, false],
].map(([name, kind, description, size_bytes, downloaded, in_use, for_speakers]) => ({
  name,
  kind,
  description,
  size_bytes,
  downloaded,
  in_use,
  for_speakers,
  downloading: false,
})) as { name: string; kind: string; description: string; size_bytes: number; downloaded: boolean; in_use: boolean; for_speakers: boolean; downloading: boolean }[];

function modelsInfo() {
  return {
    dir: "/Users/you/Library/Application Support/com.fisherinnovation.Quill/models",
    models,
    bytes_on_disk: models.filter((m) => m.downloaded).reduce((n, m) => n + m.size_bytes, 0),
    missing_bytes: models.filter((m) => m.in_use && !m.downloaded).reduce((n, m) => n + m.size_bytes, 0),
  };
}

/** Pretends to download, sending model-download events over a few seconds. */
async function fakeDownload(names: string[]) {
  for (const name of names) {
    const model = models.find((m) => m.name === name);
    if (!model || model.downloaded) continue;
    model.downloading = true;
    for (let i = 1; i <= 20; i++) {
      await new Promise((r) => setTimeout(r, 200));
      const done = Math.round((model.size_bytes * i) / 20);
      await emit("model-download", { name, done, total: model.size_bytes, finished: false, error: null });
    }
    model.downloading = false;
    model.downloaded = true;
    await emit("model-download", { name, done: model.size_bytes, total: model.size_bytes, finished: true, error: null });
  }
}

let suggestions: { id: number; kind: string; from: string; to: string; app: string | null; seen: number; status: string }[] = [
  { id: 1, kind: "replacement", from: "open router", to: "OpenRouter", app: "Slack", seen: 2, status: "pending" },
  { id: 2, kind: "vocabulary", from: "", to: "Kubernetes", app: "VS Code", seen: 1, status: "pending" },
];

const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
  list_meetings: () => meetings,
  get_meeting: ({ id }) => {
    const meeting = meetings.find((m) => m.id === id) ?? meetings[0];
    return {
      meeting,
      segments: meeting.has_transcript ? SEGMENTS : [],
      summaries: meeting.has_summary
        ? [{ markdown: SUMMARY, model: "anthropic/claude-haiku-5.5", prompt_version: "summary-v2", prompt_tokens: 5120, completion_tokens: 410, cost_usd: 0.0007, latency_ms: 4100, created_at: iso(H) }]
        : [],
    };
  },
  search_transcripts: ({ query }) =>
    String(query).length < 2
      ? []
      : [
          { meeting_id: "m1", title: "Product sync", start_ms: 55_000, speaker: "Others", snippet: `…signups have been about twelve percent too high since the [${query}] upgrade…` },
          { meeting_id: "m4", title: "Design review: settings", start_ms: 312_000, speaker: "Me", snippet: `…the [${query}] page needs grouping like System Settings…` },
        ],
  rename_meeting: ({ id, title }) => {
    meetings = meetings.map((m) => (m.id === id ? { ...m, title: String(title) } : m));
  },
  delete_meeting: ({ id }) => {
    meetings = meetings.filter((m) => m.id !== id);
  },
  get_settings: () => settings,
  save_settings: () => null,
  recording_state: () => recording ?? { recording: false, meeting_id: null, elapsed_ms: 0 },
  start_recording: () => meetings[0],
  stop_recording: () => "m1",
  process_meeting: () => null,
  split_speakers: () => null,
  models_info: () => modelsInfo(),
  footprint: () => {
    const parts = [
      { key: "recordings", bytes: 3_412_000_000 },
      { key: "models", bytes: modelsInfo().bytes_on_disk },
      { key: "library", bytes: 2_400_000 },
      { key: "logs", bytes: 380_000 },
      { key: "app", bytes: 54_000_000 },
    ];
    return { total: parts.reduce((n, p) => n + p.bytes, 0), parts };
  },
  download_needed_models: () => {
    void fakeDownload(models.filter((m) => m.in_use && !m.downloaded).map((m) => m.name));
  },
  download_model: ({ name }) => {
    void fakeDownload([String(name)]);
  },
  delete_model: ({ name }) => {
    models = models.map((m) => (m.name === name ? { ...m, downloaded: false } : m));
    return modelsInfo();
  },
  redownload_model: ({ name }) => {
    models = models.map((m) => (m.name === name ? { ...m, downloaded: false } : m));
    void fakeDownload([String(name)]);
    return modelsInfo();
  },
  delete_all_models: () => {
    models = models.map((m) => ({ ...m, downloaded: false }));
    return modelsInfo();
  },
  show_models_dir: () => null,
  model_choices: ({ purpose }) => ({
    recommended:
      purpose === "summary"
        ? [
            { id: "anthropic/claude-haiku-5.5", label: "Claude Haiku 5.5", note: "Fast and inexpensive (the default)", per_hour: 0.00205 },
            { id: "anthropic/claude-sonnet-5.5", label: "Claude Sonnet 5.5", note: "More thorough notes; costs more", per_hour: 0.041 },
            { id: "openai/gpt-6-luna", label: "GPT-6 Luna", note: "Fast and inexpensive", per_hour: 0.00205 },
            { id: "deepseek/deepseek-v4.1-flash", label: "DeepSeek V4.1 Flash", note: "Least expensive", per_hour: 0.0029 },
          ]
        : [
            { id: "google/gemini-3.5-flash-lite", label: "Gemini 3.5 Flash-Lite", note: "Fastest reliable: about 0.7 s (the default)", per_hour: null },
            { id: "anthropic/claude-haiku-5.5", label: "Claude Haiku 5.5", note: "Accurate, but about 1.1 s", per_hour: null },
          ],
    ollama: flags.has("noollama") ? null : ["ollama:qwen3:8b", "ollama:llama3.3:70b"],
    catalog: [
      { id: "mistralai/mistral-large-4-0", name: "Mistral Large 4", input_per_million: 0.68, output_per_million: 2.09, context_length: 1048576 },
      { id: "x-ai/grok-4.7", name: "Grok 4.7", input_per_million: 2, output_per_million: 6, context_length: 500000 },
    ],
  }),
  about_info: () => ({
    version: "0.1.0",
    commit: "6530ca8f2",
    commit_date: "2026-10-12",
    debug: false,
    os: "macOS 26.0",
    arch: "Apple Silicon",
    speech_model: "turbo",
    cleanup_model: "google/gemini-3.5-flash-lite",
    summary_model: "anthropic/claude-haiku-5.5",
    identify_speakers: true,
    data_dir: "/Users/you/Library/Application Support/com.fisherinnovation.Quill",
  }),
  open_link: () => null,
  list_suggestions: () => suggestions,
  set_suggestion_status: ({ id, status }) => {
    suggestions = suggestions.map((s) => (s.id === id ? { ...s, status: String(status) } : s));
    return suggestions.filter((s) => s.status !== "dismissed");
  },
  correct_dictation: ({ id, text }) => {
    const entry = dictations.find((d) => d.id === id);
    if (entry) (entry as { corrected?: string }).corrected = String(text);
    suggestions = [
      { id: 10 + suggestions.length, kind: "vocabulary", from: "", to: "Northwind", app: entry?.app ?? null, seen: 1, status: "pending" },
      ...suggestions,
    ];
    return { entry, suggestions: suggestions.filter((s) => s.status !== "dismissed") };
  },
  globe_key_action: () => "default",
  open_keyboard_settings: () => null,
  rename_speaker: ({ from, to }) => {
    SEGMENTS = SEGMENTS.map((seg) => (seg.speaker === from ? { ...seg, speaker: String(to) } : seg));
  },
  openrouter_key_status: () => "unknown",
  delete_openrouter_key: () => true,
  storage_info: () => ({ path: "/Users/you/Library/Application Support/com.fisherinnovation.Quill/recordings", is_default: true, available: true, bytes: 3_412_000_000 }),
  check_audio_access: async () => {
    await new Promise((r) => setTimeout(r, 1500));
    return {
      microphone: { device: "MacBook Pro Microphone", heard: true, peak_db: -18 },
      system_audio: { device: "MacBook Pro Speakers", heard: !flags.has("failed"), peak_db: flags.has("failed") ? null : -16 },
    };
  },
  dictation_state: () => ({ phase, error: flags.has("failed") ? "Quill isn't allowed to paste, so the text is on your clipboard." : null }),
  list_dictations: () => dictations,
  paste_permission: () => !flags.has("nopaste"),
  system_language: () => "en",
  dictation_apps: () => [
    { app_id: "com.tinyspeck.slackmacgap", name: "Slack", style: "casual", built_in: "casual", words: 5_210 },
    { app_id: "com.apple.mail", name: "Mail", style: "formal", built_in: "formal", words: 3_380 },
    { app_id: "com.microsoft.vscode", name: "Code", style: "technical", built_in: "technical", words: 2_140 },
    { app_id: "com.google.chrome", name: "Google Chrome", style: "default", built_in: "default", words: 1_250 },
  ],
  export_summary: () => "/Users/you/Documents/2026-10-11 Product sync.md",
  get_insights: () => {
    // Weekdays busier than weekends, a few quiet days.
    const days = Array.from({ length: 30 }, (_, i) => {
      const date = new Date(now - (29 - i) * D);
      const weekend = date.getDay() === 0 || date.getDay() === 6;
      const busy = weekend ? 0.15 : 0.6 + 0.4 * Math.sin(i * 1.3) ** 2;
      const words = i % 9 === 4 ? 0 : Math.round(busy * 900);
      return {
        date: date.toISOString().slice(0, 10),
        words,
        dictations: Math.round(words / 28),
        meeting_ms: weekend ? 0 : Math.round(busy * 3) * 30 * 60_000,
      };
    });
    const hours = Array.from({ length: 24 }, (_, h) =>
      h < 8 || h > 19 ? (h === 21 ? 3 : 0) : Math.round(14 * Math.exp(-((h - 10.5) ** 2) / 6) + 9 * Math.exp(-((h - 15) ** 2) / 4)),
    );
    return {
      dictation: { count: 412, words: 12_480, words_last_7_days: 2_904, speaking_wpm: 151, minutes_saved: 229, median_latency_ms: 430, polished_share: 0.27 },
      meetings: { count: 38, total_ms: 27.5 * H, count_last_7_days: 7, ms_last_7_days: 5.2 * H, average_ms: 43 * 60_000, my_talk_share: 0.36, summaries: 41, summary_cost_usd: 0.031 },
      streak_days: 6,
      days,
      hours,
      typing_wpm: 40,
      apps: [
        { app_id: "com.tinyspeck.slackmacgap", name: "Slack", dictations: 188, words: 5_210 },
        { app_id: "com.apple.mail", name: "Mail", dictations: 61, words: 3_380 },
        { app_id: "com.microsoft.vscode", name: "Code", dictations: 97, words: 2_140 },
        { app_id: "com.google.chrome", name: "Google Chrome", dictations: 44, words: 1_250 },
        { app_id: "notion.id", name: "Notion", dictations: 22, words: 500 },
      ],
    };
  },
};

export function installMocks() {
  if (flags.has("light")) document.documentElement.dataset.theme = "light";
  mockWindows(flags.has("indicator") ? "indicator" : flags.has("about") ? "about" : "main");
  mockIPC(
    (cmd, args) => {
      const handler = handlers[cmd];
      if (!handler) {
        console.debug("mock: unhandled", cmd, args);
        return null;
      }
      return handler((args ?? {}) as Record<string, unknown>);
    },
    { shouldMockEvents: true },
  );
  // Let the console drive live states, e.g. mockEmit("dictation-state", {phase: "listening", error: null}).
  (window as unknown as { mockEmit: typeof emit }).mockEmit = emit;
  if (flags.has("indicator") && phase === "listening") {
    // A talking-like level: bursts with pauses.
    setInterval(() => {
      const t = Date.now() / 1000;
      const speaking = Math.sin(t * 1.7) > -0.3;
      void emit("dictation-level", speaking ? 0.05 + Math.random() * 0.4 : 0.004);
    }, 50);
  }
}
