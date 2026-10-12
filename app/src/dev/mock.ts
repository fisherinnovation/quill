// Browser-only preview of the UI with sample data, for working on the design
// without the Rust side: `npm run dev`, then open http://localhost:1420.
// main.tsx loads this only in dev builds and only outside Tauri.
//
// URL flags pick a state: ?onboarding, ?recording, ?listening, ?processing,
// ?failed, ?nopaste, ?empty, ?light to force the light theme, and ?indicator
// for the dictation pill (with ?listening or ?processing).
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
    hotkey: "Alt+Space",
    language: null,
    polish: true,
    polish_model: "google/gemini-3.5-flash-lite",
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
  rename_speaker: ({ from, to }) => {
    SEGMENTS = SEGMENTS.map((seg) => (seg.speaker === from ? { ...seg, speaker: String(to) } : seg));
  },
  openrouter_key_status: () => "keychain",
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
  mockWindows(flags.has("indicator") ? "indicator" : "main");
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
