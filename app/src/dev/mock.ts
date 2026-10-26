// Browser-only preview of the UI with sample data, for working on the design
// without the Rust side: `npm run dev`, then open http://localhost:1420.
// main.tsx loads this only in dev builds and only outside Tauri.
//
// URL flags pick a state: ?onboarding, ?recording, ?listening, ?processing,
// ?failed, ?nopaste, ?empty, ?light or ?dark to pick the theme, ?indicator
// for the dictation pill (with ?listening or ?processing), ?others for a
// call not yet split into speakers, ?nomodels for a fresh install, ?fn for
// fn as the dictation key, ?about for the About window, ?prompt (or
// ?prompt=stop) for the call prompt, ?live (or ?live=empty) for the live
// notes panel, ?writeup (or ?writeup=failed) for a meeting being written
// up after recording (also what stopping a recording shows), ?update for a
// new version waiting to be installed, ?backlog for enough older
// meetings and dictations to need "Show more", ?voices for a meeting
// where Quill recognised someone by voice (Speaker 3 named Priya),
// ?nosearch for search by meaning without its model yet, and ?indexing
// for meetings being indexed for it.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { SearchResults, SearchScope, UpdateInfo } from "../api";
import { simulateWriteUp } from "./writeup";

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

/** A local date `days` from now, YYYY-MM-DD. */
const dayFromNow = (days: number) => new Date(now + days * D).toLocaleDateString("en-CA");

/** Sample action items across meetings: due soon, overdue, undated and done. */
const item = (
  id: number,
  meeting: [string, string, number],
  owner: string | null,
  task: string,
  due?: [string, number],
  doneAgo?: number,
) => ({
  id,
  meeting_id: meeting[0],
  meeting_title: meeting[1],
  meeting_started_at: iso(meeting[2]),
  owner,
  mine: owner === "You",
  task,
  due_text: due?.[0] ?? null,
  due_date: due ? dayFromNow(due[1]) : null,
  done_at: doneAgo === undefined ? null : iso(doneAgo),
});
const PRODUCT: [string, string, number] = ["m1", "Product sync", 2 * H];
const ONE_ON_ONE: [string, string, number] = ["m2", "1:1 with Priya", 5 * H];
const DESIGN: [string, string, number] = ["m4", "Design review: settings", D + 6 * H];
const NORTHWIND: [string, string, number] = ["m5", "Customer call — Northwind", 4 * D];
const HIRING: [string, string, number] = ["m6", "Hiring sync", 12 * D];
let actionItems = flags.has("empty")
  ? []
  : [
      item(1, PRODUCT, "You", "Send the rollout plan to support", ["by Wednesday", 1]),
      item(2, PRODUCT, "Priya", "Fix the duplicate `signup_completed` events", ["Thursday", 2]),
      item(3, PRODUCT, "Dan", "Prepare two pricing-page variants for the next sprint"),
      item(4, ONE_ON_ONE, "You", "Share the promotion packet template with Priya", ["end of the week", 3]),
      item(5, ONE_ON_ONE, "Priya", "Draft her goals for next quarter", ["next Monday", 6]),
      item(6, DESIGN, "You", "Group the settings pages like System Settings", ["by Monday", -2]),
      item(7, DESIGN, "You", "Check contrast on the dark theme"),
      item(8, NORTHWIND, "You", "Send Northwind the SSO pricing", ["today", 0]),
      item(9, NORTHWIND, "Sam", "Confirm the pilot start date"),
      item(10, HIRING, "You", "Book the onsite interviews", ["this week", -8], 2 * D),
      item(11, HIRING, "Dan", "Write up the take-home rubric", undefined, 5 * D),
    ];

// ?others: the call side as one "Others" track, as before speakers are told apart.
const other = (n: number) => (flags.has("others") ? "Others" : `Speaker ${n}`);
let SEGMENTS = [
  [0, 6, "Me", "Okay, let's get started. Main things today are the onboarding launch and the analytics problem."],
  [7, 15, other(1), "Sure. So the usability study came back and completion went from forty-one to fifty-eight percent."],
  [15, 22, other(1), "Most of the remaining drop-off is the invite step. People don't have teammates ready yet."],
  [23, 31, "Me", "Could we make the invite skippable and nudge them later, after they've created a project?"],
  [32, 38, other(2), "Yes, that's easy. I'd still ship Friday behind the flag and test that separately."],
  [40, 52, "Me", "Agreed. Ten percent of new sign-ups to start. I'll write up the rollout plan for support."],
  [55, 70, flags.has("voices") ? "Priya" : other(3), "On analytics: signups have been about twelve percent too high since the SDK upgrade. There's a retry that re-sends the event."],
  [71, 78, "Me", "Then let's hold the pricing experiments until that's fixed. No point testing on bad data."],
  [79, 83, other(2), "Fine by me. I'll prepare two pricing-page variants for next sprint."],
].map(([s, e, speaker, text]) => ({
  start_ms: (s as number) * 1000,
  end_ms: (e as number) * 1000,
  speaker,
  text,
}));

// Known voices (quill_core::voices): who's on the sample call, and who
// Quill remembers.
const callVoice = (label: string, recognized = false) => ({
  label,
  recognized,
  similarity: recognized ? 0.78 : null,
  remembered: false,
  enough_speech: true,
});
let meetingVoices = flags.has("others")
  ? []
  : [callVoice("Speaker 1"), callVoice("Speaker 2"), flags.has("voices") ? callVoice("Priya", true) : callVoice("Speaker 3")];
const knownVoice = (id: number, name: string, meetings: number, daysAgo: number) => ({
  id,
  name,
  voice_model: "voice-titanet-small",
  meetings,
  speech_ms: meetings * 240_000,
  created_at: iso(daysAgo * D),
  updated_at: iso(D),
});
let knownVoices = [knownVoice(1, "Priya", 3, 12), knownVoice(2, "Dan", 1, 4)];
/** Naming someone remembers their voice, as `Library::rename_speaker` does. */
function rememberVoice(name: string) {
  if (/^Speaker \d+$/.test(name) || ["me", "you", "others"].includes(name.toLowerCase())) return;
  const known = knownVoices.find((v) => v.name.toLowerCase() === name.toLowerCase());
  if (known) known.meetings += 1;
  else knownVoices = [...knownVoices, knownVoice(Date.now(), name, 1, 0)];
}

let meetings = flags.has("empty")
  ? []
  : [
      { id: "m0", title: "Meeting Oct 12, 9:07 PM", started_at: iso(H / 2), duration_ms: 12_000, has_transcript: false, has_summary: false, empty: true },
      { id: "m1", title: "Product sync", started_at: iso(2 * H), duration_ms: 31 * 60_000, has_transcript: true, has_summary: true , empty: false },
      { id: "m2", title: "1:1 with Priya", started_at: iso(5 * H), duration_ms: 24 * 60_000, has_transcript: true, has_summary: true , empty: false },
      { id: "m3", title: "Meeting Oct 10, 4:02 PM", started_at: iso(D + 3 * H), duration_ms: 12 * 60_000, has_transcript: false, has_summary: false , empty: false },
      { id: "m4", title: "Design review: settings", started_at: iso(D + 6 * H), duration_ms: 48 * 60_000, has_transcript: true, has_summary: true , empty: false },
      { id: "m5", title: "Customer call — Northwind", started_at: iso(4 * D), duration_ms: 37 * 60_000, has_transcript: true, has_summary: true , empty: false },
      { id: "m6", title: "Hiring sync", started_at: iso(12 * D), duration_ms: 55 * 60_000, has_transcript: true, has_summary: true , empty: false },
    ];
// Daily summaries (quill_core::daily) by day; ?daily starts with today's written.
const SAMPLE_DAY = `## TL;DR
Onboarding ships Friday behind a flag at 10% of new sign-ups, and pricing experiments wait until Priya fixes the double-counted sign-ups. Priya's promotion packet and next quarter's goals are under way.

## Decisions
- Ship the onboarding redesign Friday at 10% of new sign-ups (Product sync)
- Pause pricing experiments until the events pipeline is fixed (Product sync)

## Your action items
- Send the rollout plan to support by Wednesday (Product sync)
- Share the promotion packet template with Priya by the end of the week (1:1 with Priya)

## Others' action items
- **Priya** — fix the duplicate \`signup_completed\` events by Thursday (Product sync)
- **Dan** — prepare two pricing-page variants for the next sprint (Product sync)

## Across meetings
- **Priya's week** — she owns the analytics fix from the Product sync, and her 1:1 set next quarter's goals around it.

## Open questions
- Should the invite step move after the first project is created? (Product sync)`;
const dayOf = (at: string) => new Date(at).toLocaleDateString("en-CA");
function dailyFor(day: string) {
  return {
    day,
    markdown: SAMPLE_DAY,
    model: "google/gemini-3.5-flash",
    prompt_version: "daily-v1",
    created_at: new Date().toISOString(),
    cost_usd: 0.0011,
    meetings: meetings.filter((m) => m.has_summary && dayOf(m.started_at) === day).length,
    outdated: false,
  };
}
const dailies = new Map<string, ReturnType<typeof dailyFor>>();
if (flags.has("daily")) dailies.set(dayOf(iso(2 * H)), dailyFor(dayOf(iso(2 * H))));

// A meeting's write-up after recording: it isn't transcribed until the
// transcription steps are done, and has notes once the notes are.
function startWriteUp(id: string, fail = flags.get("writeup") === "failed") {
  const set = (patch: Partial<(typeof meetings)[number]>) => {
    meetings = meetings.map((m) => (m.id === id ? { ...m, ...patch } : m));
  };
  set({ has_transcript: false, has_summary: false });
  simulateWriteUp({
    id,
    title: meetings.find((m) => m.id === id)?.title ?? "Meeting",
    lines: SEGMENTS.map((s) => ({ you: s.speaker === "Me", text: String(s.text) })),
    fail,
    onTranscribed: () => set({ has_transcript: true }),
    onDone: (ok) => set({ has_summary: ok }),
  });
}
if (flags.has("writeup")) {
  meetings = meetings.map((m) => (m.id === "m1" ? { ...m, has_transcript: false, has_summary: false } : m));
  setTimeout(() => startWriteUp("m1"), 900);
}

if (flags.has("backlog")) {
  const titles = ["Weekly planning", "1:1 with Dan", "Vendor demo", "Retro", "Roadmap review"];
  meetings = meetings.concat(
    Array.from({ length: 40 }, (_, i) => ({
      id: `old${i}`,
      title: titles[i % titles.length],
      started_at: iso((14 + i * 2) * D),
      duration_ms: (20 + (i % 4) * 10) * 60_000,
      has_transcript: true,
      has_summary: true,
      empty: false,
    })),
  );
}

// The live notes panel in the preview (?live; ?live=empty before the first
// update).
let live = {
  active: true,
  notes:
    flags.get("live") === "empty"
      ? { gist: "", items: [] }
      : {
          gist: "Planning Friday's onboarding launch; now on why signups look 12% too high.",
          items: [
            { id: "n1", kind: "ask", text: "Ask who owns the rollout plan for support", who: null, done: false },
            { id: "n2", kind: "ask", text: "Check whether the 10% flag is per region or global", who: null, done: false },
            { id: "n3", kind: "ask", text: "Confirm the date for the pricing experiments", who: null, done: true },
            { id: "n4", kind: "follow_up", text: "Fix the duplicate signup events by Thursday", who: "Priya", done: false },
            { id: "n5", kind: "follow_up", text: "Send the rollout plan to support by Wednesday", who: "You", done: false },
            { id: "n6", kind: "decision", text: "Ship the redesign Friday behind a flag at 10%", who: null, done: false },
          ],
        },
  updated_ms: Date.now() - 60_000,
  next_ms: null,
  updating: false,
  heard: "and then we can look at pricing once the numbers are clean",
  error: null,
  collapsed: false,
  // ?live&writeup: the panel after the recording, following the write-up.
  wrapping: (flags.has("writeup") ? { meeting_id: "m1", title: "Product sync" } : null) as {
    meeting_id: string;
    title: string;
  } | null,
};

// What the dictation captions say in the preview (?indicator&listening).
const CAPTION =
  "Okay so for the launch plan I think we should ship the onboarding redesign on Friday. Then we can look at the pricing page next week once the analytics are fixed. And I'll send the rollout notes to support tomorrow.";

// The Meetings page's overviews: a made-up but steady conversation shape
// for each meeting (seeded by its id), with people, gist and action items.
const OVERVIEW_DETAILS: Record<string, [string[], string, number]> = {
  m1: [["Speaker 1", "Speaker 2", "Speaker 3"], "Ship the onboarding redesign Friday behind a flag, at 10% of new sign-ups.", 3],
  m2: [["Priya"], "Priya takes over the analytics pipeline; the SDK retry bug is first.", 2],
  m4: [["Priya", "Speaker 2"], "Settings get grouped like System Settings, with search at the top.", 4],
  m5: [["Northwind"], "Northwind wants SSO before rollout; a pilot with 20 seats starts in November.", 1],
  m6: [["Dan", "Speaker 2", "Speaker 3", "Speaker 4"], "Two finalists for the design role; references this week.", 2],
};

function mockOverview(m: (typeof meetings)[number]) {
  const [people, gist, actions] = OVERVIEW_DETAILS[m.id] ?? [["Speaker 1", "Speaker 2"], "Agreed the plan for the next two weeks.", 2];
  if (!m.has_transcript) return { id: m.id, speaker: [], level: [], people: [], my_share: null, gist: null, action_items: 0 };
  let seed = [...m.id].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7);
  const random = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32);
  const speaker: number[] = [];
  const level: number[] = [];
  let who = 1;
  for (let i = 0; i < 96; ) {
    const run = 1 + Math.floor(random() * 7);
    const quiet = random() < 0.12;
    for (let j = 0; j < run && i < 96; j++, i++) {
      speaker.push(quiet ? 0 : who);
      level.push(quiet ? 0 : Math.round(45 + random() * 55));
    }
    // Back and forth: often you, otherwise someone on the call.
    who = who !== 1 && random() < 0.55 ? 1 : 2 + Math.floor(random() * people.length);
  }
  const mine = speaker.filter((s) => s === 1).length / Math.max(1, speaker.filter((s) => s > 0).length);
  return { id: m.id, speaker, level, people, my_share: mine, gist: m.has_summary ? gist : null, action_items: m.has_summary ? actions : 0 };
}

let recording: { recording: boolean; paused: boolean; meeting_id: string | null; elapsed_ms: number } | null =
  flags.has("recording") || flags.has("live") ? { recording: true, paused: false, meeting_id: "m1", elapsed_ms: 754_000 } : null;
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
  check_for_updates: true,
  theme: flags.has("light") ? "light" : flags.has("dark") ? "dark" : "system",
  offer_to_record_calls: true,
  live_notes: true,
  notify_notes_ready: true,
  recognize_voices: true,
  semantic_search: true,
  gpu: { kind: "automatic" },
  dictation: {
    enabled: true,
    hotkey: flags.has("fn") ? "Fn" : "Alt+Space",
    language: null,
    stt_model: null,
    polish: true,
    polish_model: "google/gemini-3.5-flash-lite",
    about: "",
    vocabulary: ["Quill", "OpenRouter", "Priya"],
    replacements: [{ from: "open router", to: "OpenRouter" }],
    keep_history: true,
    save_samples: false,
    live_captions: true,
    sound: "classic",
    app_styles: [],
  },
};

let samples = { count: 23, bytes: 9_400_000 };

let update: UpdateInfo = {
  current_version: "0.1.0",
  status: flags.has("update") ? { state: "available", version: "0.2.0" } : { state: "up_to_date" },
  checked_at: now - 2 * H,
};

/** Plays an update download, then "restarts" (reloads without ?update). */
async function installUpdate() {
  const total = 48_000_000;
  for (let done = 0; done <= total; done += 6_000_000) {
    update = { ...update, status: { state: "downloading", version: "0.2.0", done, total } };
    await emit("update-status", update);
    await new Promise((r) => setTimeout(r, 300));
  }
  update = { ...update, status: { state: "installing", version: "0.2.0" } };
  await emit("update-status", update);
  await new Promise((r) => setTimeout(r, 1000));
  flags.delete("update");
  location.search = flags.toString();
}

/** Sample apps: their ids (as Quill records them) and stand-in icons. */
const MOCK_APPS: Record<string, { id: string; color?: string; glyph: string }> = {
  Slack: { id: "com.tinyspeck.slackmacgap", color: "#4a154b", glyph: "#" },
  Mail: { id: "com.apple.mail", color: "#1a8cff", glyph: "✉" },
  Code: { id: "com.microsoft.vscode", color: "#0065a9", glyph: "❮❯" },
  Cursor: { id: "com.todesktop.230313mzl4w4u92", color: "#111111", glyph: "▲" },
  Messages: { id: "com.apple.mobilesms", color: "#30d158", glyph: "…" },
  "Google Chrome": { id: "com.google.chrome", color: "#ea4335", glyph: "●" },
  Notion: { id: "notion.id", glyph: "N" },
};

function mockIcon(color: string, glyph: string) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect x="4" y="4" width="56" height="56" rx="13" fill="${color}"/><text x="32" y="42" font-family="-apple-system, sans-serif" font-size="26" font-weight="700" fill="#fff" text-anchor="middle">${glyph}</text></svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

const withAppId = <T extends { app: string | null; id: number; latency_ms: number }>(d: T) => ({
  ...d,
  app_id: d.app ? (MOCK_APPS[d.app]?.id ?? null) : null,
  // Time to insert: the newer ones were measured.
  wait_ms: d.id >= 103 ? d.latency_ms + 380 + (d.id % 3) * 120 : null,
});

let dictations: {
  id: number;
  created_at: string;
  text: string;
  raw: string | null;
  polish_model: string | null;
  audio_ms: number;
  latency_ms: number;
  pasted: boolean;
  app: string;
  corrected?: string | null;
}[] = [
  { id: 105, created_at: iso(60_000), text: "Can we move the design review to Friday afternoon?", raw: null, polish_model: null, audio_ms: 3200, latency_ms: 290, pasted: true, app: "Messages" },
  { id: 104, created_at: iso(3 * 60_000), text: "The retry loop in sync_client backs off exponentially but never caps the delay, so after a long outage the client waits hours before trying again. Cap it at sixty seconds and add jitter, then log each retry at debug level with the attempt number and the error, so we can see it in the field without flooding the logs.", raw: "The retry loop in sync client backs off exponentially but never caps the delay, um, so after a long outage the client waits hours before trying again. Cap it at sixty seconds and add jitter, then log each retry at debug level with the attempt number and the error, so we can see it in the field without flooding the logs.", polish_model: "google/gemini-3.5-flash-lite", audio_ms: 41_000, latency_ms: 720, pasted: true, app: "Cursor" },
  { id: 103, created_at: iso(4 * 60_000), text: "Sounds good, I'll send the rollout plan to support by Wednesday.", raw: null, polish_model: null, audio_ms: 3900, latency_ms: 310, pasted: true, app: "Slack", corrected: "Sounds good, I'll send the rollout plan to Priya by Wednesday." },
  { id: 102, created_at: iso(40 * 60_000), text: "For the launch we need three things:\n1. The landing page\n2. The pricing table\n3. The onboarding emails", raw: "For the launch we need three things. First, the landing page. Second, the pricing table. And third, the onboarding emails.", polish_model: "google/gemini-3.5-flash-lite", audio_ms: 9100, latency_ms: 980, pasted: true, app: "Notion" },
  { id: 101, created_at: iso(3 * H), text: "Let's meet at 4 p.m. on Thursday in the big conference room.", raw: "Let's meet at 3, no wait, 4 p.m. on Thursday in the big conference room.", polish_model: "google/gemini-3.5-flash-lite", audio_ms: 6100, latency_ms: 940, pasted: false, app: "Mail" },
];
if (flags.has("backlog")) {
  const lines = ["Can you send me the deck before the call?", "Looks good to me, ship it.", "Let's push this to next sprint.", "Thanks, that fixed it."];
  const apps = ["Slack", "Mail", "Notion", "Messages"];
  dictations = dictations.concat(
    Array.from({ length: 30 }, (_, i) => ({
      id: 100 - i,
      created_at: iso((1 + i * 0.4) * D),
      text: lines[i % lines.length],
      raw: null,
      polish_model: null,
      audio_ms: 2500,
      latency_ms: 300,
      pasted: true,
      app: apps[i % apps.length],
    })),
  );
}

// Saved dictation samples to review (Settings → Dictation → Review…).
let reviewSamples = [
  ["2026-10-16_091204", "Can you send me the deck before the call with Priya?", null, "Slack", 3800, "Can you send me the deck before the call with Priya?"],
  ["2026-10-16_093517", "The retry loop in sync client never caps the delay.", "The retry loop in sync client, um, never caps the delay.", "Cursor", 4600, "The retry loop in sync_client never caps the delay."],
  ["2026-10-16_101145", "Let's ship the onboarding redesign Friday behind a flag.", null, "Slack", 3900, null],
  ["2026-10-16_112030", "Quil should open the notes when the call ends.", null, "Notion", 3300, null],
  ["2026-10-16_140212", "Thanks, that fixed it.", null, "Messages", 1500, null],
  ["2026-10-16_151840", "For the launch we need three things:\n1. The landing page\n2. The pricing table\n3. The onboarding emails", "For the launch we need three things. First, the landing page. Second, the pricing table. And third, the onboarding emails.", "Notion", 9100, null],
].map(([name, typed, whisper, app, duration_ms, reference]) => ({
  name: name as string,
  typed: typed as string,
  whisper: (whisper as string | null) ?? (typed as string),
  app: app as string,
  polish_model: whisper ? "google/gemini-3.5-flash-lite" : null,
  duration_ms: duration_ms as number,
  reference: reference as string | null,
}));

function sampleReview() {
  // Rough word errors, enough for the preview: words that differ by position.
  const words = (t: string) => t.toLowerCase().replace(/[^a-z0-9_\s']/g, " ").split(/\s+/).filter(Boolean);
  const diff = (ref: string, hyp: string) => {
    const [r, h] = [words(ref), words(hyp)];
    const errors: string[] = [];
    for (let i = 0; i < Math.max(r.length, h.length); i++) {
      if (r[i] !== h[i]) errors.push(r[i] && h[i] ? `${h[i]}→${r[i]}` : r[i] ? `-${r[i]}` : `+${h[i]}`);
    }
    return errors;
  };
  const list = reviewSamples.map((s) => ({ ...s, errors: s.reference ? diff(s.reference, s.typed) : [] }));
  const tally = (pick: (s: (typeof list)[number]) => string) => {
    const t = { reference_words: 0, substitutions: 0, deletions: 0, insertions: 0 };
    for (const s of list) {
      if (!s.reference) continue;
      t.reference_words += words(s.reference).length;
      t.substitutions += diff(s.reference, pick(s)).length;
    }
    return t;
  };
  const counts = new Map<string, number>();
  for (const s of list) for (const e of s.errors) counts.set(e, (counts.get(e) ?? 0) + 1);
  return {
    samples: list,
    accuracy: {
      reviewed: list.filter((s) => s.reference).length,
      typed: tally((s) => s.typed),
      whisper: tally((s) => s.whisper),
      cleaned: list.filter((s) => s.reference && s.polish_model).length,
      common: [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 8),
    },
  };
}

/** A second of soft tone as a WAV, standing in for a sample's audio. */
function toneWav(): ArrayBuffer {
  const rate = 16_000;
  const n = rate * 1.2;
  const buf = new ArrayBuffer(44 + n * 2);
  const v = new DataView(buf);
  const str = (o: number, t: string) => [...t].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
  str(0, "RIFF");
  v.setUint32(4, 36 + n * 2, true);
  str(8, "WAVEfmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, rate, true);
  v.setUint32(28, rate * 2, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  str(36, "data");
  v.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++) {
    const env = Math.sin((Math.PI * i) / n);
    v.setInt16(44 + i * 2, Math.sin((2 * Math.PI * 220 * i) / rate) * env * 3000, true);
  }
  return buf;
}

const fresh = flags.has("nomodels");
let models = [
  ["turbo", "speech", "large-v3 with a 4-layer decoder; near large-v3 accuracy, much faster", 1_624_555_275, !fresh, true, false],
  ["small", "speech", "244M params", 487_601_967, !fresh, false, false],
  ["vad", "vad", "Silero VAD v6.2: skips silence so Whisper doesn't invent text", 885_098, !fresh, true, false],
  ["segmentation", "speakers", "pyannote segmentation 3.0: where speaker turns are", 5_992_913, !fresh, true, true],
  ["voice-titanet-small", "speakers", "NVIDIA NeMo TitaNet small (CC BY 4.0)", 40_257_283, !fresh, true, true],
  ["voice-eres2net", "speakers", "3D-Speaker ERes2Net, VoxCeleb (Apache-2.0)", 26_485_263, !fresh, false, false],
  ["search", "search", "multilingual-e5-small: finds moments by meaning, in ~100 languages", 470_641_600, !fresh && !flags.has("nosearch"), true, false],
  ["search-tokenizer", "search", "multilingual-e5-small's tokenizer", 17_082_730, !fresh && !flags.has("nosearch"), true, false],
  ["parakeet-v2-encoder", "speech", "Parakeet v2 encoder", 652_184_296, false, false, false],
  ["parakeet-v2-decoder", "speech", "Parakeet v2 decoder", 7_257_753, false, false, false],
  ["parakeet-v2-joiner", "speech", "Parakeet v2 joiner", 1_739_080, false, false, false],
  ["parakeet-v2-tokens", "speech", "Parakeet v2 tokens", 9_384, false, false, false],
  ["parakeet-v3-encoder", "speech", "Parakeet v3 encoder", 652_184_281, false, false, false],
  ["parakeet-v3-decoder", "speech", "Parakeet v3 decoder", 11_845_275, false, false, false],
  ["parakeet-v3-joiner", "speech", "Parakeet v3 joiner", 6_355_277, false, false, false],
  ["parakeet-v3-tokens", "speech", "Parakeet v3 tokens", 93_939, false, false, false],
].map(([name, kind, description, size_bytes, downloaded, in_use, for_speakers]) => ({
  name,
  kind,
  description,
  size_bytes,
  downloaded,
  in_use,
  for_speakers,
  downloading: false,
  // Parakeet's files belong to a model: "parakeet-v2-encoder" to "parakeet-v2".
  part_of: String(name).startsWith("parakeet-") ? String(name).split("-").slice(0, 2).join("-") : null,
})) as {
  name: string;
  kind: string;
  description: string;
  size_bytes: number;
  downloaded: boolean;
  in_use: boolean;
  for_speakers: boolean;
  downloading: boolean;
  part_of: string | null;
}[];

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
  if (names.some((n) => n.startsWith("search")) && semanticReady()) void fakeIndexing(0);
}

/** Search by meaning: meetings indexed so far (all, unless ?indexing). */
let indexed = flags.has("indexing") ? 2 : Infinity;

const semanticReady = () => models.filter((m) => m.kind === "search").every((m) => m.downloaded);

function semanticStatus() {
  const total = meetings.filter((m) => m.has_transcript).length;
  return {
    enabled: settings.semantic_search,
    model_ready: semanticReady(),
    indexed: Math.min(indexed, total),
    total,
    paused: false,
    error: null,
  };
}

/** Pretends to index meetings, one every second or so. */
async function fakeIndexing(from: number) {
  indexed = from;
  const total = semanticStatus().total;
  while (indexed < total) {
    await emit("semantic-index", semanticStatus());
    await new Promise((r) => setTimeout(r, 1200));
    indexed++;
  }
  await emit("semantic-index", semanticStatus());
}
if (flags.has("indexing")) setTimeout(() => void fakeIndexing(2), 1500);

/**
 * Search across the library (quill_core::search), over the sample data: by
 * words (each must appear, the last as a prefix), plus a result or two "by
 * meaning" once the search model is in.
 */
function searchLibrary(query: string, scope: SearchScope): SearchResults {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const limit = scope === "all" ? 5 : 50;
  const group = <T>(hits: T[]) => ({ hits: hits.slice(0, limit), more: hits.length > limit });
  const empty = { hits: [], more: false };
  if (words.length === 0) return { meetings: empty, moments: empty, notes: empty, actions: empty, dictations: empty };
  const esc = (w: string) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const matches = (text: string) => {
    const t = text.toLowerCase();
    return words.every((w, i) => (i === words.length - 1 ? t.includes(w) : new RegExp(`\\b${esc(w)}\\b`).test(t)));
  };
  // Matches wrapped in [ ], about twelve words either side of the first.
  const snippet = (text: string) => {
    const marked = text.replace(new RegExp(`(${words.map(esc).join("|")})`, "gi"), "[$1]");
    const at = marked.indexOf("[");
    if (at < 0) return text.slice(0, 160);
    const before = marked.slice(0, at).split(" ");
    return (before.length > 12 ? "…" + before.slice(-12).join(" ") : marked.slice(0, at)) + marked.slice(at, at + 140);
  };
  // The sample "by meaning" results are about hiring: other queries find none, as the
  // real search finds nothing past its cut-offs.
  const byMeaning =
    settings.semantic_search && semanticReady() && /hir|recruit|candidat|engineer|role|interview|team/i.test(query);
  const covers = (s: SearchScope) => scope === "all" || scope === s;
  const title = (id: string) => meetings.find((m) => m.id === id);

  // The sample notes, by section, as the Product sync's.
  const sections = SUMMARY.split(/^## /m)
    .slice(1)
    .map((s) => {
      const [heading, ...lines] = s.trim().split("\n");
      return { heading, text: lines.map((l) => l.replace(/^- /, "").replace(/\*\*|`/g, "")).join(" ").trim() };
    })
    .filter((s) => !s.heading.toLowerCase().startsWith("action"));
  const m1 = title("m1");
  return {
    meetings: covers("meetings")
      ? group(meetings.filter((m) => matches(m.title)).map((m) => ({ meeting_id: m.id, title: m.title, started_at: m.started_at, snippet: snippet(m.title) })))
      : empty,
    moments: covers("meetings")
      ? group([
          ...(m1 ? SEGMENTS.filter((s) => matches(String(s.text))) : []).map((s) => ({
            meeting_id: "m1",
            title: m1?.title ?? "",
            start_ms: s.start_ms,
            speaker: String(s.speaker),
            snippet: snippet(String(s.text)),
            matched: "words" as const,
          })),
          // What search by meaning adds: the same subject in other words.
          ...(byMeaning && title("m6")
            ? [{ meeting_id: "m6", title: "Hiring sync", start_ms: 141_000, speaker: "Dan", snippet: "We're bringing on two more engineers before the end of the quarter, and the design role has two finalists.", matched: "meaning" as const }]
            : []),
        ])
      : empty,
    notes: covers("notes")
      ? group([
          ...(m1 ? sections.filter((s) => matches(s.text)) : []).map((s) => ({
            meeting_id: "m1",
            title: m1?.title ?? "",
            started_at: m1?.started_at ?? "",
            heading: s.heading,
            snippet: snippet(String(s.text)),
            matched: "words" as const,
          })),
          ...(byMeaning && title("m6")
            ? [{ meeting_id: "m6", title: "Hiring sync", started_at: title("m6")?.started_at ?? "", heading: "Decisions", snippet: "Open two engineering roles this quarter; the design role goes to the two finalists for a paid trial week.", matched: "meaning" as const }]
            : []),
        ])
      : empty,
    actions: covers("actions")
      ? group(
          actionItems
            .filter((a) => matches(`${a.owner ?? ""}: ${a.task}`))
            .map((a) => ({
              action_id: a.id,
              meeting_id: a.meeting_id,
              title: a.meeting_title,
              owner: a.owner,
              task: a.task,
              done: a.done_at !== null,
              snippet: snippet(`${a.owner ? `${a.owner}: ` : ""}${a.task}`),
              matched: "words" as const,
            })),
        )
      : empty,
    dictations: covers("dictations")
      ? group(
          dictations
            .map(withAppId)
            .filter((d) => matches(d.corrected ?? d.text))
            .map((d) => ({
              dictation_id: d.id,
              created_at: d.created_at,
              app: d.app,
              app_id: d.app_id,
              snippet: snippet((d.corrected ?? d.text).replace(/\n/g, " ")),
              matched: "words" as const,
            })),
        )
      : empty,
  };
}

let suggestions: { id: number; kind: string; from: string; to: string; app: string | null; seen: number; status: string }[] = [
  { id: 1, kind: "replacement", from: "open router", to: "OpenRouter", app: "Slack", seen: 2, status: "pending" },
  { id: 2, kind: "vocabulary", from: "", to: "Kubernetes", app: "VS Code", seen: 1, status: "pending" },
];

/** Stats for a 31-minute call, from made-up stretches of talking. */
function meetingStats() {
  const names = flags.has("others") ? ["Me", "Others"] : ["Me", "Speaker 1", "Speaker 2", "Speaker 3"];
  const weights = flags.has("others") ? [0.4, 0.6] : [0.34, 0.38, 0.18, 0.1];
  const pace = [148, 162, 131, 175];
  const previews = [
    "Okay, let's get started. Main things today are the onboarding launch and the analytics problem",
    "So the usability study came back and completion went from forty-one to fifty-eight percent",
    "Yes, that's easy. I'd still ship Friday behind the flag and test that separately",
    "On analytics: signups have been about twelve percent too high since the SDK upgrade",
  ];
  let seed = 7;
  const random = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  const pick = () => {
    let r = random();
    for (let i = 0; i < weights.length; i++) if ((r -= weights[i]) < 0) return i;
    return 0;
  };
  const stretches: { speaker: string; start_ms: number; end_ms: number; words: number; preview: string }[] = [];
  let t = 2_000;
  let last = -1;
  while (t < 31 * 60_000) {
    let who = pick();
    if (who === last) who = (who + 1) % names.length;
    const ms = (who === 1 && random() < 0.12 ? 90 : 6 + random() * 40) * 1000;
    stretches.push({ speaker: names[who], start_ms: t, end_ms: t + ms, words: Math.round((ms / 60_000) * pace[who]), preview: previews[who] });
    // A long quiet moment about two-thirds through.
    t += ms + (t > 20 * 60_000 && t < 20.8 * 60_000 ? 38_000 : 600 + random() * 2500);
    last = who;
  }
  const signature = [["rollout", "support"], ["onboarding", "invite", "study"], ["flag", "variants"], ["analytics", "SDK", "events"]];
  const speakers = names.map((name, i) => {
    const own = stretches.filter((s) => s.speaker === name);
    const talk_ms = own.reduce((n, s) => n + s.end_ms - s.start_ms, 0);
    const words = own.reduce((n, s) => n + s.words, 0);
    return {
      name,
      talk_ms,
      share: 0,
      words,
      wpm: talk_ms > 30_000 ? (words / talk_ms) * 60_000 : null,
      turns: own.length,
      questions: [9, 4, 2, 1][i] ?? 0,
      longest_stretch_ms: Math.max(...own.map((s) => s.end_ms - s.start_ms)),
      fillers: [3, 11, 2, 6][i] ?? 0,
      signature_words: flags.has("others") ? [] : signature[i],
    };
  });
  const total = speakers.reduce((n, s) => n + s.talk_ms, 0);
  speakers.forEach((s) => (s.share = s.talk_ms / total));
  const longest = stretches.reduce((a, b) => (b.end_ms - b.start_ms > a.end_ms - a.start_ms ? b : a));
  return {
    span_ms: stretches[stretches.length - 1].end_ms,
    speech_ms: total,
    words: speakers.reduce((n, s) => n + s.words, 0),
    wpm: (speakers.reduce((n, s) => n + s.words, 0) / total) * 60_000,
    turns: stretches.length,
    questions: speakers.reduce((n, s) => n + s.questions, 0),
    speakers,
    stretches,
    longest_stretch: longest,
    longest_silence: { start_ms: 20.4 * 60_000, end_ms: 20.4 * 60_000 + 38_000 },
    busiest_pair: { a: "Me", b: names[1], handovers: 41 },
    top_words: [
      ["onboarding", 23], ["pricing", 17], ["analytics", 15], ["flag", 12], ["launch", 11], ["signups", 9],
      ["invite", 8], ["friday", 7], ["support", 6], ["variants", 5], ["events", 5], ["sprint", 4],
    ].map(([text, count]) => ({ text, count })),
    top_phrases: [
      ["pricing page", 6], ["feature flag", 5], ["invite step", 4], ["usability study", 3], ["next sprint", 3], ["rollout plan", 2],
    ].map(([text, count]) => ({ text, count })),
  };
}

const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
  list_meetings: () => meetings,
  meeting_overviews: () => meetings.map(mockOverview),
  call_prompt: () =>
    flags.get("prompt") === "stop" ? { kind: "stop", app: "Zoom" } : { kind: "record", app: "Google Chrome" },
  answer_call_prompt: () => {},
  live_notes_state: () => live,
  live_notes_refresh: () => {
    live = { ...live, updating: true };
    void emit("live-notes", live);
    setTimeout(() => {
      live = { ...live, updating: false, updated_ms: Date.now() };
      void emit("live-notes", live);
    }, 1500);
  },
  live_notes_dismiss: ({ id }) => {
    live = { ...live, notes: { ...live.notes, items: live.notes.items.filter((i) => i.id !== id) } };
    void emit("live-notes", live);
  },
  live_notes_tick: ({ id, done }) => {
    live = {
      ...live,
      notes: { ...live.notes, items: live.notes.items.map((i) => (i.id === id ? { ...i, done: Boolean(done) } : i)) },
    };
    void emit("live-notes", live);
  },
  live_notes_fit: () => {},
  live_notes_close: () => {
    live = { ...live, wrapping: null };
    void emit("live-notes", live);
  },
  live_notes_open_meeting: () => {
    live = { ...live, wrapping: null };
    void emit("live-notes", live);
  },
  live_notes_collapse: ({ collapsed }) => {
    live = { ...live, collapsed: Boolean(collapsed) };
    void emit("live-notes", live);
  },
  meeting_stats: ({ id }) =>
    meetings.find((m) => m.id === id)?.has_transcript ? meetingStats() : { ...meetingStats(), speakers: [], stretches: [] },
  dictation_stats: ({ id }) => {
    const entry = dictations.find((d) => d.id === id);
    const words = entry ? entry.text.split(/\s+/).length : 0;
    const audio = entry?.audio_ms ?? 1;
    return {
      words,
      speaking_wpm: (words / audio) * 60_000,
      saved_ms: Math.max(0, (words / 40) * 60_000 - audio),
      typing_wpm: 40,
      raw_words: entry?.raw ? entry.raw.split(/\s+/).length : null,
      fillers_removed: entry?.raw ? 2 : 0,
      usual_wpm: 151,
    };
  },
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
  search_library: ({ query, scope }) => searchLibrary(String(query), String(scope) as SearchScope),
  semantic_status: () => semanticStatus(),
  daily_summaries: () => [...dailies.values()],
  summarize_day: async ({ day }) => {
    // About as long as a fast hosted model takes.
    await new Promise((r) => setTimeout(r, 1500));
    const summary = dailyFor(String(day));
    dailies.set(summary.day, summary);
    return summary;
  },
  prepare_search: () => null,
  download_search_model: () => {
    void fakeDownload(models.filter((m) => m.kind === "search").map((m) => m.name));
  },
  rename_meeting: ({ id, title }) => {
    meetings = meetings.map((m) => (m.id === id ? { ...m, title: String(title) } : m));
  },
  delete_meeting: ({ id }) => {
    meetings = meetings.filter((m) => m.id !== id);
  },
  get_settings: () => settings,
  save_settings: ({ settings: next }) => {
    // Kept for what depends on it here (search by meaning's status).
    settings.semantic_search = (next as typeof settings).semantic_search;
    void emit("semantic-index", semanticStatus());
  },
  recording_state: () => recording ?? { recording: false, paused: false, meeting_id: null, elapsed_ms: 0 },
  pause_recording: () => {
    if (recording) recording = { ...recording, paused: true };
    void emit("recording-state", recording);
  },
  resume_recording: () => {
    if (recording) recording = { ...recording, paused: false };
    void emit("recording-state", recording);
  },
  start_recording: () => meetings[0],
  stop_recording: () => {
    recording = null;
    // The live notes panel stays, following the write-up.
    live = { ...live, active: false, collapsed: false, wrapping: { meeting_id: "m1", title: "Product sync" } };
    void emit("live-notes", live);
    void emit("recording-state", { recording: false, paused: false, meeting_id: null, elapsed_ms: 0 });
    startWriteUp("m1");
    return "m1";
  },
  process_meeting: ({ id }) => startWriteUp(String(id), false),
  split_speakers: () => null,
  models_info: () => modelsInfo(),
  action_items: () => actionItems,
  // The sounds are made by the app itself; a browser has nothing to play.
  preview_dictation_sound: () => undefined,
  set_action_item_done: ({ id, done }) => {
    actionItems = actionItems.map((i) => (i.id === id ? { ...i, done_at: done ? new Date().toISOString() : null } : i));
    return actionItems.find((i) => i.id === id);
  },
  // ?gpus: a PC listing its integrated graphics first, as Windows can.
  graphics_cards: () =>
    flags.has("gpus")
      ? {
          choosable: true,
          cards: [
            { name: "Intel(R) UHD Graphics 770", integrated: true, memory_bytes: 16 * 2 ** 30 },
            { name: "NVIDIA GeForce RTX 4070", integrated: false, memory_bytes: 12 * 2 ** 30 },
          ],
          automatic: "NVIDIA GeForce RTX 4070",
        }
      : { choosable: false, cards: [{ name: "Apple M3 Pro", integrated: false, memory_bytes: 28 * 2 ** 30 }], automatic: "Apple M3 Pro" },
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
  empty_recordings: () => {
    const empty = meetings.filter((m) => m.empty);
    return { count: empty.length, bytes: empty.length * 410_000 };
  },
  delete_empty_recordings: () => {
    const n = meetings.filter((m) => m.empty).length;
    meetings = meetings.filter((m) => !m.empty);
    return n;
  },
  keep_empty_recording: ({ id }) => {
    meetings = meetings.map((m) => (m.id === id ? { ...m, empty: false } : m));
  },
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
    dictation_model: settings.dictation.stt_model,
    cleanup_model: "google/gemini-3.5-flash-lite",
    summary_model: "anthropic/claude-haiku-5.5",
    identify_speakers: true,
    data_dir: "/Users/you/Library/Application Support/com.fisherinnovation.Quill",
  }),
  open_link: () => null,
  update_info: () => update,
  check_for_updates: async () => {
    await new Promise((r) => setTimeout(r, 800));
    update = { ...update, checked_at: Date.now() };
    return update;
  },
  install_update: () => {
    void installUpdate();
    return null;
  },
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
    const merging = meetingVoices.some((v) => v.label === to);
    meetingVoices = meetingVoices
      .filter((v) => !(merging && v.label === from))
      .map((v) => (v.label === from || v.label === to ? { ...callVoice(String(to)), remembered: true } : v));
    rememberVoice(String(to));
  },
  meeting_voices: () => meetingVoices,
  confirm_speaker: ({ label }) => {
    meetingVoices = meetingVoices.map((v) => (v.label === label ? { ...v, recognized: false, similarity: null, remembered: true } : v));
    rememberVoice(String(label));
  },
  known_voices: () => knownVoices,
  delete_known_voice: ({ id }) => {
    knownVoices = knownVoices.filter((v) => v.id !== id);
  },
  forget_known_voices: () => {
    const n = knownVoices.length;
    knownVoices = [];
    return n;
  },
  openrouter_key_status: () => (flags.has("nokey") ? "none" : "unknown"),
  set_openrouter_key: () => null,
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
  list_dictations: ({ before, limit }) =>
    dictations
      .filter((d) => before == null || d.id < Number(before))
      .slice(0, Number(limit))
      .map(withAppId),
  get_dictation: ({ id }) => {
    const d = dictations.find((d) => d.id === id);
    return d && withAppId(d);
  },
  // Stand-in icons: a tile in each app's colour. Notion has none, to show
  // the letter fallback.
  app_icons: ({ ids }) =>
    Object.fromEntries(
      (ids as string[]).map((id) => {
        const app = Object.values(MOCK_APPS).find((a) => a.id === id);
        return [id, app?.color ? mockIcon(app.color, app.glyph) : null];
      }),
    ),
  delete_dictation: ({ id }) => {
    dictations = dictations.filter((d) => d.id !== id);
  },
  clear_dictations: () => {
    dictations = [];
  },
  dictation_samples: () => samples,
  sample_review: () => sampleReview(),
  sample_audio: () => toneWav(),
  set_sample_reference: ({ name, text }) => {
    reviewSamples = reviewSamples.map((r) => (r.name === name ? { ...r, reference: String(text).trim() } : r));
  },
  delete_sample: ({ name }) => {
    reviewSamples = reviewSamples.filter((r) => r.name !== name);
  },
  show_dictation_samples: () => {},
  clear_dictation_samples: () => {
    samples = { count: 0, bytes: 0 };
  },
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
        // Time to insert: measured for the last 18 days, getting a little quicker.
        wait_ms: words > 0 && i >= 12 ? Math.round(1_050 - i * 9 + 120 * Math.sin(i * 2.1)) : null,
        wait_p90_ms: words > 0 && i >= 12 ? Math.round(1_900 - i * 14 + 260 * Math.cos(i * 1.7)) : null,
      };
    });
    const hours = Array.from({ length: 24 }, (_, h) =>
      h < 8 || h > 19 ? (h === 21 ? 3 : 0) : Math.round(14 * Math.exp(-((h - 10.5) ** 2) / 6) + 9 * Math.exp(-((h - 15) ** 2) / 4)),
    );
    return {
      dictation: {
        count: 412,
        words: 12_480,
        words_last_7_days: 2_904,
        speaking_wpm: 151,
        minutes_saved: 229,
        median_latency_ms: 430,
        polished_share: 0.27,
        wait: { count: 214, median_ms: 860, p90_ms: 1_640, held_ms: 430, quill_ms: 410, last_7_days_ms: 790, previous_7_days_ms: 930 },
      },
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
  mockWindows(
    flags.has("indicator")
      ? "indicator"
      : flags.has("prompt")
        ? "prompt"
        : flags.has("live")
          ? "live"
          : flags.has("about")
            ? "about"
            : "main",
  );
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
  if (phase === "listening") {
    // A talking-like level: bursts with pauses (the pill's meter, and the
    // Dictations page's waveform).
    setInterval(() => {
      const t = Date.now() / 1000;
      const speaking = Math.sin(t * 1.7) > -0.3;
      void emit("dictation-level", speaking ? 0.05 + Math.random() * 0.4 : 0.004);
    }, 50);
  }
  if (flags.has("indicator") && phase === "listening") {
    // Captions growing like the app's: a few words per 600 ms pass, a
    // sentence locking in once it's over.
    const words = CAPTION.split(" ");
    let n = 0;
    setInterval(() => {
      n = n >= words.length ? 2 : n + 1 + Math.floor(Math.random() * 3);
      const said = words.slice(0, n);
      const lockedAt = said.map((w, i) => (w.endsWith(".") ? i + 1 : 0)).reduce((a, b) => Math.max(a, b), 0);
      const locked = Math.max(0, lockedAt === said.length ? lockedAt - (n % 2) : lockedAt);
      void emit("dictation-caption", {
        committed: said.slice(0, locked).join(" "),
        tentative: said.slice(locked).join(" "),
      });
    }, 600);
  }
}
