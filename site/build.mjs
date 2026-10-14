// Builds the website into site/_site from the files next to this script.
//
//   node site/build.mjs
//
// Inputs, all optional (CI sets them; see .github/workflows/ci.yml):
//   QUILL_SITE_URL      the published address, for the og:url tag
//   QUILL_SITE_PUBLIC   "true" once the repository is public: shows source links
//   QUILL_SITE_RELEASE  path to `gh release view --json tagName,assets` output
//                       for the latest published release: shows downloads
//                       (once the repository is public)
//
// index.html is a template: {{NAME}} placeholders, and blocks
// <!--if KEY-->…<!--else KEY-->…<!--end KEY--> kept or dropped by KEY.

import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const out = join(here, "_site");
const env = process.env;

const version = JSON.parse(readFileSync(join(root, "app/src-tauri/tauri.conf.json"), "utf8")).version;
const repoUrl = "https://github.com/fisherinnovation/quill";

let release = null;
if (env.QUILL_SITE_RELEASE && existsSync(env.QUILL_SITE_RELEASE)) {
  const r = JSON.parse(readFileSync(env.QUILL_SITE_RELEASE, "utf8") || "{}");
  const asset = (re) => r.assets?.find((a) => re.test(a.name))?.url;
  const mac = asset(/\.dmg$/);
  const windows = asset(/-setup\.exe$/) ?? asset(/\.msi$/);
  if (r.tagName && mac && windows) release = { tag: r.tagName, mac, windows };
}

const isPublic = env.QUILL_SITE_PUBLIC === "true";
// A private repository's release downloads only work for its members.
const flags = { public: isPublic, release: isPublic && release !== null };
const values = {
  SITE_URL: env.QUILL_SITE_URL ?? "",
  REPO_URL: repoUrl,
  VERSION: release?.tag ?? `v${version}`,
  RELEASE_TAG: release?.tag ?? "",
  MAC_URL: release?.mac ?? "",
  WINDOWS_URL: release?.windows ?? "",
  BUILD: (env.GITHUB_SHA ?? String(Date.now())).slice(0, 8),
  // What the header video shows, for screen readers (site/video/script.json).
  VIDEO_LABEL: JSON.parse(readFileSync(join(here, "video/script.json"), "utf8")).label,
};

let html = readFileSync(join(here, "index.html"), "utf8");
// Innermost blocks first, so nested ones resolve.
const block = /<!--if (\w+)-->((?:(?!<!--if )[\s\S])*?)(?:<!--else \1-->([\s\S]*?))?<!--end \1-->/;
for (let m; (m = html.match(block)); ) {
  const [all, key, yes, no = ""] = m;
  if (!(key in flags)) throw new Error(`unknown block <!--if ${key}-->`);
  html = html.replace(all, flags[key] ? yes : no);
}
html = html.replace(/\{\{(\w+)\}\}/g, (_, k) => {
  if (!(k in values)) throw new Error(`unknown placeholder {{${k}}}`);
  return values[k];
});
html = html.replace('<html lang="en">', '<html lang="en" class="no-js">');

rmSync(out, { recursive: true, force: true });
mkdirSync(out);
writeFileSync(join(out, "index.html"), html);
for (const f of ["styles.css", "main.js"]) cpSync(join(here, f), join(out, f));
cpSync(join(root, "app/app-icon.svg"), join(out, "icon.svg"));
cpSync(join(here, "media"), join(out, "media"), { recursive: true });
// Serve files as they are (no Jekyll processing on GitHub Pages).
writeFileSync(join(out, ".nojekyll"), "");

console.log(
  `site/_site: ${values.VERSION}, ${flags.release ? "with downloads" : "no release yet"}, ` +
    `${flags.public ? "public source" : "private source"}`,
);
