// Renders the website's header video from site/video/script.json.
//
//   node site/video/render.mjs                 both themes, every scene
//   node site/video/render.mjs --theme light   one theme
//   node site/video/render.mjs --scene stats   one scene (for iterating; no final video)
//   node site/video/render.mjs --keep          keep the raw frames
//   node site/video/render.mjs --script <file> --clips <dir>
//                                              each scene of another script as
//                                              its own clip, <dir>/<id>.mp4, and
//                                              nothing else (the overview video)
//
// It runs the app's browser preview (Vite, with the sample data in
// app/src/dev/mock.ts), plays each scene in headless Chrome with a visible
// cursor, records the screen, adds the caption, and joins the scenes with
// crossfades. Needs Google Chrome and ffmpeg; no npm packages.
//
// To feel handled rather than scripted: the cursor travels on curves at a
// speed that depends on the distance and drifts a little at rest, clicks
// press in, typing has a rhythm, the camera pushes in slowly through each
// scene, and captions rise as they fade in. Randomness is seeded per scene,
// so the light and dark videos match.
//
// Writes site/media/hero-<theme>.mp4 and .jpg (the poster), and contact
// sheets with one frame per scene to site/video/out/ for review.

import { spawn, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..");
const media = join(here, "..", "media");
const out = join(here, "out");
const PORT = 1431;
const FPS = 30;
const FADE = 0.45;
/** How far the camera pushes in over a scene, unless the scene says. */
const PUSH = 0.035;

const args = process.argv.slice(2);
const arg = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
const script = JSON.parse(readFileSync(arg("--script") ?? join(here, "script.json"), "utf8"));
const clipsDir = arg("--clips");
const themes = arg("--theme") ? [arg("--theme")] : ["light", "dark"];
const only = arg("--scene");
const keep = args.includes("--keep");
const scenes = script.scenes.filter((s) => !only || s.id === only);
if (scenes.length === 0) throw new Error(`no scene "${only}" in script.json`);

const { width: W, height: H, scale: DPR } = script.viewport;
const [OW, OH] = script.output;
const work = mkdtempSync(join(tmpdir(), "quill-video-"));
mkdirSync(media, { recursive: true });
mkdirSync(out, { recursive: true });

const chromePath =
  process.env.CHROME ??
  [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ].find(existsSync);
if (!chromePath) throw new Error("Google Chrome not found; set CHROME to its path");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function ffmpeg(...a) {
  const r = spawnSync("ffmpeg", ["-hide_banner", "-loglevel", "error", "-y", ...a], { stdio: "inherit" });
  if (r.status !== 0) throw new Error(`ffmpeg failed: ${a.join(" ")}`);
}

function duration(file) {
  const r = spawnSync("ffprobe", ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", file]);
  return Number(String(r.stdout).trim());
}

// ---- The browser preview ---------------------------------------------------

async function startPreview() {
  // Vite's script run by this Node, not its .bin shim (a .cmd on Windows,
  // which Node won't spawn without a shell).
  const bin = join(root, "app/node_modules/vite/bin/vite.js");
  const vite = spawn(process.execPath, [bin, "--port", String(PORT), "--strictPort"], {
    cwd: join(root, "app"),
    stdio: "ignore",
  });
  for (let i = 0; i < 100; i++) {
    try {
      if ((await fetch(`http://localhost:${PORT}/`)).ok) return vite;
    } catch {}
    await sleep(200);
  }
  vite.kill();
  throw new Error(`the preview didn't start on port ${PORT}`);
}

// ---- Chrome over the DevTools protocol -------------------------------------

async function startChrome() {
  const profile = join(work, "profile");
  const chrome = spawn(
    chromePath,
    [
      "--headless=new",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--hide-scrollbars",
      "--mute-audio",
      "--force-color-profile=srgb",
      // The screencast only records at the device scale Chrome itself runs
      // at; the per-page override alone gives 1x frames.
      `--force-device-scale-factor=${DPR}`,
      `--window-size=${W},${H}`,
      "about:blank",
    ],
    { stdio: "ignore" },
  );
  const portFile = join(profile, "DevToolsActivePort");
  for (let i = 0; i < 100 && !existsSync(portFile); i++) await sleep(100);
  const [port, path] = readFileSync(portFile, "utf8").trim().split("\n");
  const ws = new WebSocket(`ws://127.0.0.1:${port}${path}`);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = reject;
  });
  let id = 0;
  const pending = new Map();
  const listeners = new Set();
  ws.onmessage = ({ data }) => {
    const msg = JSON.parse(data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      msg.error ? reject(new Error(`${msg.error.message} (${msg.error.data ?? ""})`)) : resolve(msg.result);
    } else if (msg.method) {
      for (const l of listeners) l(msg);
    }
  };
  const send = (method, params = {}, sessionId) =>
    new Promise((resolve, reject) => {
      pending.set(++id, { resolve, reject });
      ws.send(JSON.stringify({ id, method, params, sessionId }));
    });
  const close = async () => {
    ws.close();
    const exited = new Promise((r) => chrome.once("exit", r));
    chrome.kill();
    await exited;
  };
  return { chrome, send, listeners, close };
}

/** A fresh tab at the scene's size and theme. */
async function openPage(cdp, theme, { width = W, height = H, scale = DPR } = {}) {
  const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
  const send = (method, params) => cdp.send(method, params, sessionId);
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: scale, mobile: false });
  await send("Emulation.setEmulatedMedia", {
    features: [
      { name: "prefers-color-scheme", value: theme },
      { name: "prefers-reduced-motion", value: "no-preference" },
    ],
  });
  const errors = [];
  const onEvent = (msg) => {
    if (msg.sessionId !== sessionId) return;
    if (msg.method === "Runtime.exceptionThrown") errors.push(msg.params.exceptionDetails.exception?.description);
    if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error")
      errors.push(msg.params.args.map((a) => a.value ?? a.description).join(" "));
  };
  cdp.listeners.add(onEvent);
  const evaluate = async (expression) => {
    const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(`${expression.slice(0, 80)}…: ${r.exceptionDetails.exception?.description}`);
    return r.result.value;
  };
  const close = async () => {
    cdp.listeners.delete(onEvent);
    await cdp.send("Target.closeTarget", { targetId });
  };
  return { send, evaluate, errors, sessionId, close };
}

// ---- In-page helpers: the cursor and the steps ------------------------------

// Injected into each scene. Targets are CSS selectors, or "text:Label" for
// the first visible button, tab, link or list item whose text starts with
// Label (exact matches first).
const HELPERS = (seed) => `(() => {
  if (window.__demo) return;
  // Seeded per scene: the same moves in both themes.
  let seed = ${seed};
  const random = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
  const cursor = document.createElement("div");
  cursor.innerHTML = '<svg width="26" height="26" viewBox="0 0 24 24"><path d="M5 3l14 8-6.2 1.6L10 19z" fill="#111" stroke="#fff" stroke-width="1.6" stroke-linejoin="round"/></svg>';
  Object.assign(cursor.style, { position: "fixed", left: "0", top: "0", zIndex: 2147483647, pointerEvents: "none",
    transition: "opacity 200ms", filter: "drop-shadow(0 2px 3px rgb(0 0 0 / .3))" });
  const ripple = document.createElement("div");
  Object.assign(ripple.style, { position: "fixed", width: "36px", height: "36px", margin: "-18px 0 0 -18px", borderRadius: "50%",
    background: "rgb(120 120 140 / .32)", zIndex: 2147483646, pointerEvents: "none", opacity: "0", transform: "scale(.3)" });
  document.body.append(ripple, cursor);
  let at = { x: innerWidth * 0.62, y: innerHeight * 0.7 };
  let press = 1;
  let moving = false;
  // At rest the hand isn't perfectly still: a slow drift of a pixel or two.
  let drift = { x: 0, y: 0 };
  const place = (p) => {
    cursor.style.transform = "translate(" + (p.x + drift.x - 4) + "px," + (p.y + drift.y - 3) + "px) scale(" + press + ")";
  };
  const wander = (now) => {
    const t = now / 1000;
    const k = moving ? 0 : 1;
    drift = { x: k * (Math.sin(t * 0.9) * 1.6 + Math.sin(t * 2.3) * 0.5), y: k * (Math.cos(t * 0.7) * 1.3) };
    if (!moving) place(at);
    requestAnimationFrame(wander);
  };
  requestAnimationFrame(wander);
  place(at);
  const visible = (el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0 && r.bottom > 0 && r.top < innerHeight; };
  const find = (target) => {
    if (target.startsWith("text:")) {
      const want = target.slice(5).trim().toLowerCase();
      // Things you click first, then containers (a list item clicks its button).
      const all = [
        ...document.querySelectorAll('button, a, [role="tab"], [role="button"], summary'),
        ...document.querySelectorAll("label, li"),
      ];
      const text = (el) => (el.innerText || el.textContent || "").replace(/\\s+/g, " ").trim().toLowerCase();
      // Visible matches first; one scrolled out of view is scrolled to when used.
      const ranked = [...all.filter(visible), ...all.filter((e) => !visible(e))];
      const el = ranked.find((e) => text(e) === want) ?? ranked.find((e) => text(e).startsWith(want));
      if (!el) throw new Error("nothing visible with text " + JSON.stringify(want));
      return el;
    }
    const el = [...document.querySelectorAll(target)].find(visible) ?? document.querySelector(target);
    if (!el) throw new Error("no element " + target);
    return el;
  };
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  // Somewhere near the middle of the target, not its exact centre.
  const aim = (el) => {
    const r = el.getBoundingClientRect();
    const x = r.left + Math.min(r.width / 2, 60) + (random() - 0.5) * Math.min(r.width * 0.2, 10);
    const y = r.top + r.height / 2 + (random() - 0.5) * Math.min(r.height * 0.25, 4);
    return { x, y };
  };
  const ease = (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);
  // A gentle curve, longer moves taking longer, easing out at the end.
  const glide = (to) => new Promise((resolve) => {
    const from = { ...at };
    const dx = to.x - from.x, dy = to.y - from.y;
    const dist = Math.hypot(dx, dy);
    if (dist < 2) return resolve();
    const ms = Math.min(950, 300 + dist * 0.75) * (0.9 + random() * 0.2);
    const bend = (random() < 0.5 ? -1 : 1) * (0.08 + random() * 0.1);
    const c = { x: from.x + dx / 2 - dy * bend, y: from.y + dy / 2 + dx * bend };
    const start = performance.now();
    moving = true;
    const step = (now) => {
      const t = Math.min(1, (now - start) / ms);
      const e = ease(t), u = 1 - e;
      at = { x: u * u * from.x + 2 * u * e * c.x + e * e * to.x, y: u * u * from.y + 2 * u * e * c.y + e * e * to.y };
      place(at);
      if (t < 1) requestAnimationFrame(step);
      else { at = to; moving = false; resolve(); }
    };
    requestAnimationFrame(step);
  });
  const move = async (el) => { el.scrollIntoView({ block: "nearest" }); await sleep(50); await glide(aim(el)); await sleep(90 + random() * 120); };
  const setValue = (el, value) => {
    const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, "value").set.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  };
  window.__demo = {
    find,
    rect(target) { const r = find(target).getBoundingClientRect(); return { x: r.left, y: r.top, width: r.width, height: r.height }; },
    async click(target) {
      const found = find(target);
      const el = found.matches("li, label") ? (found.querySelector("button, a, input") ?? found) : found;
      await move(el);
      // Press in, ripple out, let go.
      press = 0.86; place(at);
      ripple.style.transition = "none";
      Object.assign(ripple.style, { left: at.x + "px", top: at.y + "px", opacity: "1", transform: "scale(.3)" });
      ripple.getBoundingClientRect();
      ripple.style.transition = "opacity 420ms ease-out, transform 420ms cubic-bezier(.2,.8,.2,1)";
      Object.assign(ripple.style, { opacity: "0", transform: "scale(1.5)" });
      await sleep(110); el.click(); press = 1; place(at); await sleep(160);
    },
    async hover(target) {
      const el = find(target); await move(el);
      for (const type of ["pointerover", "mouseover", "pointerenter", "mouseenter"])
        el.dispatchEvent(new MouseEvent(type, { bubbles: !type.endsWith("enter"), clientX: at.x, clientY: at.y }));
    },
    async type(target, text, cps) {
      const el = find(target); await move(el); el.focus();
      // Bursts and small hesitations, a beat after spaces and punctuation.
      for (let i = 1; i <= text.length; i++) {
        setValue(el, text.slice(0, i));
        const ch = text[i - 1];
        await sleep((1000 / cps) * (0.55 + random() * 0.9) + (ch === " " && random() < 0.25 ? 90 : 0) + (/[,.?]/.test(ch) ? 140 : 0));
      }
    },
    async scroll(target, to) {
      const el = find(target);
      if (to === undefined) el.scrollIntoView({ behavior: "smooth", block: "center" });
      else {
        // Puts the element's top \`to\` pixels from the top of the window.
        let box = el.parentElement;
        while (box && !(box.scrollHeight > box.clientHeight && /auto|scroll/.test(getComputedStyle(box).overflowY))) box = box.parentElement;
        (box ?? document.scrollingElement).scrollBy({ top: el.getBoundingClientRect().top - to, behavior: "smooth" });
      }
      await sleep(800);
    },
    cursor(show) { cursor.style.opacity = show ? "1" : "0"; },
    point(target) { at = aim(find(target)); place(at); },
  };
})()`;

/** A scene's random seed, from its id. */
function seedOf(id) {
  let h = 7;
  for (const c of id) h = (h * 31 + c.charCodeAt(0)) % 2147483647;
  return h || 1;
}

/** Runs one step of a scene (see SKILL.md for the vocabulary). */
async function step(page, s) {
  const q = JSON.stringify;
  if ("wait" in s) return sleep(s.wait);
  if ("click" in s) return page.evaluate(`__demo.click(${q(s.click)})`);
  if ("hover" in s) return page.evaluate(`__demo.hover(${q(s.hover)})`);
  if ("scroll" in s) return page.evaluate(`__demo.scroll(${q(s.scroll)}, ${s.to ?? "undefined"})`);
  if ("type" in s) return page.evaluate(`__demo.type(${q(s.type.into)}, ${q(s.type.text)}, ${s.type.cps ?? 16})`);
  if ("emit" in s) return page.evaluate(`window.mockEmit(${q(s.emit.event)}, ${q(s.emit.payload ?? null)})`);
  if ("cursor" in s)
    return page.evaluate(
      s.cursor === "hide" || s.cursor === "show" ? `__demo.cursor(${s.cursor === "show"})` : `__demo.point(${q(s.cursor)})`,
    );
  if ("stage" in s) return page.evaluate(readFileSync(join(here, "stages", `${s.stage}.js`), "utf8"));
  if ("eval" in s) return page.evaluate(s.eval);
  throw new Error(`unknown step ${q(s)}`);
}

// ---- Recording ---------------------------------------------------------------

/** Records `run()` with the screencast; returns frame files and how long each shows. */
async function record(cdp, page, dir, run) {
  mkdirSync(dir, { recursive: true });
  const frames = [];
  const onFrame = (msg) => {
    if (msg.sessionId !== page.sessionId || msg.method !== "Page.screencastFrame") return;
    const file = join(dir, `f${String(frames.length).padStart(5, "0")}.jpg`);
    writeFileSync(file, Buffer.from(msg.params.data, "base64"));
    frames.push({ file, t: performance.now() });
    page.send("Page.screencastFrameAck", { sessionId: msg.params.sessionId }).catch(() => {});
  };
  cdp.listeners.add(onFrame);
  await page.send("Page.startScreencast", { format: "jpeg", quality: 92, maxWidth: W * DPR, maxHeight: H * DPR });
  const start = performance.now();
  // Something must change on screen for the first frame to arrive.
  await page.evaluate(`__demo.cursor(true)`);
  await run();
  const end = performance.now();
  await page.send("Page.stopScreencast");
  cdp.listeners.delete(onFrame);
  if (frames.length === 0) throw new Error("no frames recorded");
  const list = frames.map((f, i) => {
    const from = i === 0 ? start : f.t;
    const to = i + 1 < frames.length ? frames[i + 1].t : end;
    return `file '${f.file}'\nduration ${Math.max(to - from, 1) / 1000}`;
  });
  // The concat demuxer needs the last file again to honour its duration.
  list.push(`file '${frames[frames.length - 1].file}'`);
  writeFileSync(join(dir, "frames.txt"), list.join("\n"));
  return { list: join(dir, "frames.txt"), seconds: (end - start) / 1000 };
}

/**
 * The camera: the region's width at the output's shape, from the region's
 * top (or centred on it when it's shorter), kept inside the window.
 */
function camera(rect) {
  const aspect = OW / OH;
  let width = Math.min(rect.width, W, H * aspect);
  let height = width / aspect;
  let x = rect.x + (rect.width - width) / 2;
  let y = rect.height < height ? rect.y - (height - rect.height) / 2 : rect.y;
  x = Math.max(0, Math.min(x, W - width));
  y = Math.max(0, Math.min(y, H - height));
  const even = (n) => Math.round((n * DPR) / 2) * 2;
  return { x: even(x), y: even(y), w: even(width), h: even(width / aspect) };
}

// The caption card, drawn by the browser so it uses the site's font, then
// laid over the video. Same look as the site's cards.
/** `part` is "scrim" (on from the first frame) or "card" (fades in). */
function captionHtml(text, theme, part) {
  const dark = theme === "dark";
  return `<!doctype html><html><head><style>
    html, body { margin: 0; background: transparent; }
    body { width: ${OW}px; height: ${OH}px; position: relative; }
    /* A scrim in the app's background colour, so text passing under the
       caption fades out instead of colliding with it. */
    body::before { content: ""; position: absolute; inset: auto 0 0 0; height: 30%; display: ${part === "scrim" ? "block" : "none"};
      background: linear-gradient(to bottom, ${dark ? "rgb(20 20 28 / 0)" : "rgb(255 255 255 / 0)"}, ${dark ? "rgb(20 20 28 / .96)" : "rgb(255 255 255 / .96)"} 62%); }
    div { display: ${part === "card" ? "block" : "none"}; position: absolute; left: 48px; bottom: 44px; padding: 16px 26px; border-radius: 16px;
      font: 600 34px/1.2 -apple-system, BlinkMacSystemFont, "SF Pro Display", "Segoe UI", Inter, sans-serif;
      letter-spacing: -0.01em; color: ${dark ? "#f0f0f4" : "#121218"};
      background: ${dark ? "rgb(31 31 40 / .94)" : "rgb(255 255 255 / .95)"};
      box-shadow: 0 2px 4px rgb(0 0 0 / .08), 0 12px 32px rgb(0 0 0 / ${dark ? ".45" : ".14"}); }
    div::before { content: ""; display: inline-block; width: 12px; height: 12px; margin-right: 14px; border-radius: 50%;
      background: #ff1053; vertical-align: 3px; }
  </style></head><body><div>${text.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c])}</div></body></html>`;
}

async function renderCaption(cdp, text, theme, part, file) {
  const page = await openPage(cdp, theme, { width: OW, height: OH, scale: 1 });
  await page.send("Emulation.setDefaultBackgroundColorOverride", { color: { r: 0, g: 0, b: 0, a: 0 } });
  await page.send("Page.navigate", { url: `data:text/html;base64,${Buffer.from(captionHtml(text, theme, part)).toString("base64")}` });
  await sleep(300);
  const { data } = await page.send("Page.captureScreenshot", { format: "png" });
  writeFileSync(file, Buffer.from(data, "base64"));
  await page.close();
}

async function renderScene(cdp, scene, theme, dir) {
  const page = await openPage(cdp, theme);
  const flags = new URLSearchParams(scene.flags ?? "");
  if (theme === "light") flags.set("light", "");
  const url = `http://localhost:${PORT}/?${flags.toString().replace(/=(&|$)/g, "$1")}`;
  await page.send("Page.navigate", { url });
  await page.evaluate(`new Promise((resolve) => {
    const ready = () => document.querySelector(${JSON.stringify(scene.ready ?? "#root > *")});
    const check = () => (ready() ? resolve() : setTimeout(check, 50));
    check();
  })`);
  await page.evaluate("document.fonts.ready.then(() => true)");
  await sleep(500);
  await page.evaluate(HELPERS(seedOf(scene.id)));
  await page.evaluate(`__demo.cursor(false)`);
  for (const s of scene.setup ?? []) await step(page, s);
  await sleep(300);

  const pad = scene.pad ?? 16;
  const rect = Array.isArray(scene.camera)
    ? { x: scene.camera[0], y: scene.camera[1], width: scene.camera[2], height: scene.camera[2] / (OW / OH) }
    : scene.camera
      ? await page.evaluate(`(() => {
          const r = __demo.rect(${JSON.stringify(scene.camera)});
          return { x: r.x - ${pad}, y: r.y - ${pad}, width: r.width + ${2 * pad}, height: r.height + ${2 * pad} };
        })()`)
      : { x: 0, y: 0, width: W, height: H };
  const cam = camera(rect);
  if (scene.cursorStart) await step(page, { cursor: scene.cursorStart });

  const { list, seconds } = await record(cdp, page, join(dir, "frames"), async () => {
    const start = performance.now();
    for (const s of scene.steps ?? []) await step(page, s);
    // Held for `hold` ms, and longer if the scene must last `minSeconds`.
    await sleep(Math.max(scene.hold ?? 1500, (scene.minSeconds ?? 0) * 1000 - (performance.now() - start)));
  });
  if (page.errors.length) console.warn(`  ${scene.id} (${theme}): page errors:\n    ${page.errors.join("\n    ")}`);
  await page.close();

  const clip = join(dir, "clip.mp4");
  // A slow push-in through the scene, centred. Zoomed at twice the output
  // size, so zoompan's whole-pixel steps don't show as a judder.
  const push = scene.push ?? PUSH;
  const frames = Math.max(1, Math.round(seconds * FPS));
  const zoom = push
    ? `,scale=${OW * 2}:${OH * 2}:flags=lanczos,zoompan=z='1+${push}*on/${frames}':x='iw/2-iw/zoom/2':y='ih/2-ih/zoom/2':d=1:s=${OW * 2}x${OH * 2}:fps=${FPS}`
    : "";
  const base = `crop=${cam.w}:${cam.h}:${cam.x}:${cam.y},fps=${FPS}${zoom},scale=${OW}:${OH}:flags=lanczos,format=yuv420p,settb=AVTB`;
  if (scene.caption) {
    // The scrim is there from the first frame so it matches across the
    // crossfade from the previous scene; only the card fades in.
    const scrim = join(dir, "scrim.png");
    const card = join(dir, "card.png");
    await renderCaption(cdp, scene.caption, theme, "scrim", scrim);
    await renderCaption(cdp, scene.caption, theme, "card", card);
    ffmpeg(
      ...["-f", "concat", "-safe", "0", "-i", list, "-loop", "1", "-i", scrim, "-loop", "1", "-i", card],
      ...[
        "-filter_complex",
        // The card rises 22 px as it fades in, easing out.
        `[0:v]${base}[v];[1:v]scale=${OW}:${OH},format=rgba[s];[2:v]scale=${OW}:${OH},format=rgba,fade=in:st=0.35:d=0.45:alpha=1[c];` +
          `[v][s]overlay=shortest=1:format=auto[vs];` +
          `[vs][c]overlay=x=0:y='22*pow(1-clip((t-0.35)/0.6,0,1),3)':shortest=1:format=auto,format=yuv420p[o]`,
      ],
      ...["-map", "[o]", "-t", seconds.toFixed(3), "-c:v", "libx264", "-crf", "14", "-preset", "fast", clip],
    );
  } else {
    ffmpeg("-f", "concat", "-safe", "0", "-i", list, "-vf", base, "-t", seconds.toFixed(3), "-c:v", "libx264", "-crf", "14", "-preset", "fast", clip);
  }
  return { clip, seconds: duration(clip) };
}

/** Joins the scenes with crossfades and fades the loop's seam through the page colour. */
function join_(clips, theme, file) {
  const bg = theme === "dark" ? "0x0f0f14" : "0xf4f3f0";
  const inputs = clips.flatMap((c) => ["-i", c.clip]);
  let chain = "";
  let last = "[0:v]";
  let offset = 0;
  for (let i = 1; i < clips.length; i++) {
    offset += clips[i - 1].seconds - FADE;
    chain += `${last}[${i}:v]xfade=transition=fade:duration=${FADE}:offset=${offset.toFixed(3)}[x${i}];`;
    last = `[x${i}]`;
  }
  const total = clips.reduce((n, c) => n + c.seconds, 0) - FADE * (clips.length - 1);
  chain += `${last}fade=in:st=0:d=0.3:color=${bg},fade=out:st=${(total - 0.35).toFixed(3)}:d=0.35:color=${bg}[o]`;
  ffmpeg(
    ...inputs,
    ...["-filter_complex", chain, "-map", "[o]", "-an"],
    ...["-c:v", "libx264", "-preset", "slow", "-crf", String(script.crf ?? 24), "-pix_fmt", "yuv420p", "-movflags", "+faststart", file],
  );
  return total;
}

// ---- Main --------------------------------------------------------------------

const vite = await startPreview();
const cdp = await startChrome();
// "Copy" buttons write to the clipboard, which headless Chrome refuses by default.
await cdp.send("Browser.grantPermissions", {
  origin: `http://localhost:${PORT}`,
  permissions: ["clipboardReadWrite", "clipboardSanitizedWrite"],
});
try {
  for (const theme of themes) {
    console.log(`${theme}:`);
    const clips = [];
    for (const scene of scenes) {
      const dir = join(work, theme, scene.id);
      const clip = await renderScene(cdp, scene, theme, dir);
      console.log(`  ${scene.id}: ${clip.seconds.toFixed(1)} s`);
      clips.push({ ...clip, id: scene.id });
      if (clipsDir) {
        mkdirSync(clipsDir, { recursive: true });
        copyFileSync(clip.clip, join(clipsDir, `${scene.id}.mp4`));
      }
    }
    if (clipsDir) continue;

    // One frame per scene, late enough for the caption and the action.
    const sheet = join(out, `contact-${theme}${only ? `-${only}` : ""}.jpg`);
    const stills = clips.map((c, i) => {
      const f = join(work, theme, `still-${i}.jpg`);
      ffmpeg("-ss", String(Math.max(c.seconds - 0.6, 0)), "-i", c.clip, "-frames:v", "1", "-vf", "scale=800:-2", f);
      return f;
    });
    const cols = Math.min(3, stills.length);
    // xstack needs two or more inputs.
    if (stills.length === 1) ffmpeg("-i", stills[0], "-frames:v", "1", sheet);
    else ffmpeg(
      ...stills.flatMap((f) => ["-i", f]),
      "-filter_complex",
      `${stills.map((_, i) => `[${i}:v]`).join("")}xstack=inputs=${stills.length}:layout=${stills
        .map((_, i) => `${(i % cols) ? Array.from({ length: i % cols }, () => "w0").join("+") : 0}_${Math.floor(i / cols) ? Array.from({ length: Math.floor(i / cols) }, () => "h0").join("+") : 0}`)
        .join("|")}:fill=gray`,
      "-frames:v",
      "1",
      sheet,
    );
    console.log(`  contact sheet: ${sheet}`);

    if (!only) {
      const file = join(media, `hero-${theme}.mp4`);
      const total = join_(clips, theme, file);
      ffmpeg("-ss", String(Math.min(clips[0].seconds - 0.6, total)), "-i", file, "-frames:v", "1", "-vf", "scale=1200:-2", "-q:v", "4", join(media, `hero-${theme}.jpg`));
      console.log(`  ${file}: ${total.toFixed(1)} s, ${(statSync(file).size / 1e6).toFixed(1)} MB`);
    }
  }
} finally {
  await cdp.close();
  vite.kill();
  if (!keep) rmSync(work, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  else console.log(`frames kept in ${work}`);
}
