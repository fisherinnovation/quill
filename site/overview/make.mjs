// Makes Quill's overview video from site/overview/script.json:
//
//   node site/overview/make.mjs                 everything, then the render
//   node site/overview/make.mjs --plan          what it would generate and spend; no API calls
//   node site/overview/make.mjs --budget 15     refuse to spend more than $15 (default $10)
//   node site/overview/make.mjs --fresh voice   regenerate one stage: voice, music, footage or app
//
// 1. Narration: one ElevenLabs line per scene, through OpenRouter. Each
//    line's length sets its scene's.
// 2. Music (Lyria) and footage of people (Veo), through OpenRouter.
// 3. App shots: recorded from the browser preview by site/video/render.mjs,
//    from the header video's scenes, held for as long as their line.
// 4. Remotion (./remotion) cuts it together with captions and the music
//    dipping under the voice.
//
// Everything generated is cached in site/overview/cache/ by what made it, so
// editing one line only pays for that line. Writes site/overview/out/
// quill-overview.mp4 and contact.jpg (a frame per scene, for review).
// Needs Google Chrome, ffmpeg and `npm ci` in ./remotion.

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as openrouter from "./openrouter.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..");
const cache = join(here, "cache");
const pub = join(cache, "public");
const out = join(here, "out");
const args = process.argv.slice(2);
const arg = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
const plan = args.includes("--plan");
const budget = Number(arg("--budget") ?? 10);
const fresh = new Set((arg("--fresh") ?? "").split(",").filter(Boolean));

const script = JSON.parse(readFileSync(join(here, "script.json"), "utf8"));
const hero = JSON.parse(readFileSync(join(root, "site/video/script.json"), "utf8"));
const FPS = script.fps;
const [WIDTH, HEIGHT] = script.output;
/** Frames each scene overlaps the next by, as it crossfades. */
const FADE = 12;

mkdirSync(pub, { recursive: true });
mkdirSync(out, { recursive: true });

const hash = (x) => createHash("sha256").update(JSON.stringify(x)).digest("hex").slice(0, 10);
const kind = (s) => Object.keys(s.shot)[0];

function run(cmd, a, opts = {}) {
  const r = spawnSync(cmd, a, { stdio: "inherit", ...opts });
  if (r.status !== 0) throw new Error(`${cmd} ${a.slice(0, 3).join(" ")}… failed`);
}
const ffmpeg = (...a) => run("ffmpeg", ["-hide_banner", "-loglevel", "error", "-y", ...a]);
function seconds(file) {
  const r = spawnSync("ffprobe", ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", file]);
  return Number(String(r.stdout).trim());
}

// ---- What's needed, and what it costs ---------------------------------------

const voiceFile = (s) => `voice-${s.id}-${hash([script.voice, s.say])}.mp3`;
const musicKey = hash(script.music.prompt + script.music.model);
const footageFile = (s) => `footage-${s.id}-${hash([script.footage, s.shot.footage, s.shot.seconds])}.mp4`;
const need = (stage, file) => fresh.has(stage) || !existsSync(join(pub, file));

const videoModels = await fetch("https://openrouter.ai/api/v1/videos/models")
  .then((r) => r.json())
  .then((j) => j.data)
  .catch(() => []);
const perSecond = (() => {
  const skus = videoModels.find((m) => m.id === script.footage.model)?.pricing_skus ?? {};
  return Number(skus.duration_seconds_without_audio ?? skus.duration_seconds ?? 0.4);
})();

const todo = [];
for (const s of script.scenes) {
  if (need("voice", voiceFile(s))) todo.push({ what: `voice: ${s.id}`, usd: s.say.length * 80e-6 });
  if (kind(s) === "footage" && need("footage", footageFile(s)))
    todo.push({ what: `footage: ${s.id} (${s.shot.seconds} s)`, usd: s.shot.seconds * perSecond });
}
const musicCached = !fresh.has("music") && ["mp3", "wav", "ogg", "flac"].some((e) => existsSync(join(pub, `music-${musicKey}.${e}`)));
if (!musicCached) todo.push({ what: "music", usd: 0.08 });
const spend = todo.reduce((n, t) => n + t.usd, 0);
console.log(todo.length ? "To generate:" : "Everything's cached.");
for (const t of todo) console.log(`  ${t.what}  ~$${t.usd.toFixed(2)}`);
if (todo.length) console.log(`  about $${spend.toFixed(2)} in all (budget $${budget})`);
if (!plan && spend > budget) throw new Error(`that's over the budget: pass --budget ${Math.ceil(spend)} to go ahead`);
// (Not process.exit: on Windows, exiting with fetch's sockets open trips an assertion in libuv.)
if (!plan) await make();

async function make() {

// ---- 1. Narration -----------------------------------------------------------

for (const s of script.scenes) {
  const file = voiceFile(s);
  if (need("voice", file)) {
    console.log(`voice: ${s.id}`);
    writeFileSync(join(pub, file), await openrouter.speech({ ...script.voice, input: s.say }));
  }
  s.voiceSeconds = seconds(join(pub, file));
}

// ---- 2. Music and footage, side by side --------------------------------------

async function makeMusic() {
  const existing = ["mp3", "wav", "ogg", "flac"].map((e) => `music-${musicKey}.${e}`).find((f) => existsSync(join(pub, f)));
  if (existing && !fresh.has("music")) return existing;
  console.log("music: generating");
  const { bytes, format, cost } = await openrouter.music(script.music);
  const magic = bytes.subarray(0, 4).toString("latin1");
  const ext =
    magic === "RIFF" ? "wav" : magic === "OggS" ? "ogg" : magic === "fLaC" ? "flac" : magic.startsWith("ID3") || bytes[0] === 0xff ? "mp3" : null;
  if (!ext) throw new Error(`music: unrecognised audio (format ${format}, starts ${bytes.subarray(0, 8).toString("hex")})`);
  const file = `music-${musicKey}.${ext}`;
  writeFileSync(join(pub, file), bytes);
  console.log(`music: done (${seconds(join(pub, file)).toFixed(0)} s${cost !== undefined ? `, $${cost}` : ""})`);
  return file;
}

async function makeFootage(s) {
  const file = footageFile(s);
  if (!need("footage", file)) return;
  console.log(`footage: ${s.id}: generating with ${script.footage.model}`);
  const { bytes, cost } = await openrouter.video({
    model: script.footage.model,
    prompt: s.shot.footage,
    seconds: s.shot.seconds,
    resolution: script.footage.resolution,
    aspect: script.footage.aspect,
    extra: script.footage.negative
      ? { provider: { options: { "google-vertex": { parameters: { negativePrompt: script.footage.negative } } } } }
      : {},
    log: (m) => console.log(`footage: ${s.id}${m}`),
  });
  writeFileSync(join(pub, file), bytes);
  console.log(`footage: ${s.id}: done${cost !== undefined ? ` ($${cost})` : ""}`);
}

// Settled, not all: if one fails, the others still finish and are kept
// (they're paid for either way).
const [musicJob, ...footageJobs] = await Promise.allSettled([
  makeMusic(),
  ...script.scenes.filter((s) => kind(s) === "footage").map(makeFootage),
]);
const failed = [musicJob, ...footageJobs].filter((j) => j.status === "rejected");
if (failed.length) throw new Error(failed.map((j) => j.reason.message).join("; "));
const musicSrc = musicJob.value;

// ---- The timeline -----------------------------------------------------------

let from = 0;
const scenes = script.scenes.map((s, i) => {
  // A beat before the line (after the crossfade) and after it; the end card
  // stays up a little longer.
  const lead = i === 0 ? 0.5 : FADE / FPS + 0.1;
  const tail = kind(s) === "end" ? 2.4 : 0.45;
  const frames = Math.round((lead + s.voiceSeconds + tail) * FPS);
  const scene = {
    id: s.id,
    kind: kind(s),
    say: s.say,
    from,
    frames,
    voice: { src: voiceFile(s), from: from + Math.round(lead * FPS), frames: Math.ceil(s.voiceSeconds * FPS) },
  };
  if (scene.kind === "footage") {
    scene.src = footageFile(s);
    // The parts of the clip to use, in order: [from, to] seconds, jump-cut together.
    scene.cuts = (s.shot.cuts ?? [[0, s.shot.seconds]]).map(([a, b]) => [Math.round(a * FPS), Math.round(b * FPS)]);
    const usable = scene.cuts.reduce((n, [a, b]) => n + b - a, 0);
    if (frames + FADE > usable) console.warn(`  ${s.id}: the line outlasts its ${(usable / FPS).toFixed(1)} s of footage`);
  }
  if (scene.kind === "card") scene.card = s.shot.card;
  if (scene.kind === "end") scene.end = s.shot.end;
  from += frames;
  return scene;
});

// ---- 3. App shots ------------------------------------------------------------

const appScenes = scenes.filter((s) => s.kind === "app");
const pending = [];
for (const s of appScenes) {
  const source = script.scenes.find((x) => x.id === s.id);
  const def = hero.scenes.find((x) => x.id === source.shot.app);
  if (!def) throw new Error(`${s.id}: no scene "${source.shot.app}" in site/video/script.json`);
  // Long enough to cover the scene and its crossfade out.
  const { caption, ...rest } = def;
  const scene = { ...rest, id: s.id, minSeconds: (s.frames + FADE) / FPS + 0.3 };
  s.src = `app-${s.id}-${hash([scene, hero.viewport, hero.output])}.mp4`;
  if (fresh.has("app") || !existsSync(join(pub, s.src))) pending.push({ s, scene });
}
if (pending.length) {
  console.log(`app: recording ${pending.map((p) => p.s.id).join(", ")}`);
  const tmp = join(cache, "app");
  rmSync(tmp, { recursive: true, force: true });
  mkdirSync(tmp, { recursive: true });
  const file = join(tmp, "script.json");
  writeFileSync(file, JSON.stringify({ ...hero, scenes: pending.map((p) => p.scene) }, null, 2));
  run(process.execPath, [join(root, "site/video/render.mjs"), "--script", file, "--clips", tmp, "--theme", "light"]);
  for (const { s } of pending) copyFileSync(join(tmp, `${s.id}.mp4`), join(pub, s.src));
}

// ---- 4. Cut it together ---------------------------------------------------------

copyFileSync(join(root, "app/app-icon.svg"), join(pub, "icon.svg"));
const total = from;
const props = {
  width: WIDTH,
  height: HEIGHT,
  fps: FPS,
  frames: total,
  fade: FADE,
  scenes,
  music: { src: musicSrc, volume: script.music.volume, ducked: script.music.ducked },
};
const propsFile = join(cache, "props.json");
writeFileSync(propsFile, JSON.stringify(props, null, 2));
console.log(`render: ${(total / FPS).toFixed(1)} s`);
const video = join(out, "quill-overview.mp4");
const raw = join(cache, "render.mp4");
const remotion = join(here, "remotion");
run(
  process.execPath,
  [
    join(remotion, "node_modules/@remotion/cli/remotion-cli.js"),
    "render",
    "src/index.jsx",
    "Overview",
    raw,
    `--props=${propsFile}`,
    `--public-dir=${pub}`,
    "--codec=h264",
    "--crf=18",
    "--log=warn",
  ],
  { cwd: remotion },
);

// Loudness for the web: -16 LUFS, peaks under -1.5 dB, measured first so
// the second pass is linear (the mix keeps its balance).
const LOUDNORM = "I=-16:TP=-1.5:LRA=11";
const probe = spawnSync("ffmpeg", ["-hide_banner", "-nostats", "-i", raw, "-vn", "-af", `loudnorm=${LOUDNORM}:print_format=json`, "-f", "null", "-"], {
  encoding: "utf8",
});
const m = JSON.parse(probe.stderr.match(/\{[^{}]*"input_i"[^{}]*\}/)[0]);
ffmpeg(
  ...["-i", raw, "-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"],
  ...["-af", `loudnorm=${LOUDNORM}:measured_I=${m.input_i}:measured_TP=${m.input_tp}:measured_LRA=${m.input_lra}:measured_thresh=${m.input_thresh}:offset=${m.target_offset}:linear=true`],
  ...["-ar", "48000", video],
);

// A frame from the middle of each scene, three to a row.
const stills = scenes.map((s, i) => {
  const f = join(cache, `still-${i}.jpg`);
  ffmpeg("-ss", ((s.from + s.frames * 0.6) / FPS).toFixed(2), "-i", video, "-frames:v", "1", "-vf", "scale=640:-2", f);
  return f;
});
const cols = 3;
const layout = stills
  .map((_, i) => `${i % cols ? Array.from({ length: i % cols }, () => "w0").join("+") : 0}_${Math.floor(i / cols) ? Array.from({ length: Math.floor(i / cols) }, () => "h0").join("+") : 0}`)
  .join("|");
ffmpeg(...stills.flatMap((f) => ["-i", f]), "-filter_complex", `xstack=inputs=${stills.length}:layout=${layout}:fill=gray`, "-frames:v", "1", join(out, "contact.jpg"));

console.log(`${video}: ${(total / FPS).toFixed(1)} s, ${(statSync(video).size / 1e6).toFixed(1)} MB`);
console.log(`contact sheet: ${join(out, "contact.jpg")}`);
}
