// Renders the link preview image (og:image) from site/card/index.html into
// site/media/card.jpg, 1200×630. It shows a frame of the header video, so
// re-render it whenever the video changes.
//
//   node site/card/render.mjs
//
// Needs Google Chrome (or CHROME set to a Chromium) and ffmpeg; no npm packages.

import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, "../media/card.jpg");

const chromePath =
  process.env.CHROME ??
  [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ].find(existsSync);
if (!chromePath) throw new Error("Google Chrome not found; set CHROME to its path");

const run = (cmd, args) => {
  const r = spawnSync(cmd, args, { stdio: "inherit" });
  if (r.status !== 0) throw new Error(`${cmd} failed`);
};

// A throwaway profile, so a Chrome the user has open doesn't get in the way.
const tmp = mkdtempSync(join(tmpdir(), "quill-card-"));
try {
  const png = join(tmp, "card.png");
  run(chromePath, [
    "--headless=new",
    `--user-data-dir=${join(tmp, "profile")}`,
    "--no-first-run",
    "--hide-scrollbars",
    "--force-color-profile=srgb",
    "--force-device-scale-factor=1",
    // Lets the page load the icon and the video poster next to it.
    "--allow-file-access-from-files",
    "--window-size=1200,630",
    `--screenshot=${png}`,
    pathToFileURL(join(here, "index.html")).href,
  ]);
  // A photo-like image: JPEG is a fifth of the PNG's size.
  run("ffmpeg", ["-hide_banner", "-loglevel", "error", "-y", "-i", png, "-q:v", "3", out]);
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
console.log(`site/media/card.jpg`);
