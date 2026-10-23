// Builds the website into site/_site from the files next to this script.
//
//   node site/build.mjs
//
// Inputs, all optional (CI sets them; see .github/workflows/ci.yml):
//   QUILL_SITE_URL      the published address, ending in "/": for link
//                       previews, structured data and the sitemap, which
//                       need absolute URLs (left out without it)
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

const siteUrl = env.QUILL_SITE_URL ?? "";
if (siteUrl && !siteUrl.endsWith("/")) throw new Error("QUILL_SITE_URL must end in /");
const isPublic = env.QUILL_SITE_PUBLIC === "true";
// A private repository's release downloads only work for its members.
const flags = {
  url: siteUrl !== "",
  public: isPublic,
  release: isPublic && release !== null,
  // The narrated overview (site/overview/make.mjs), once there is one.
  overview: existsSync(join(here, "media/overview.mp4")),
};

let html = readFileSync(join(here, "index.html"), "utf8");
const description = html.match(/<meta\s+name="description"\s+content="([^"]*)"/)?.[1];
if (!description) throw new Error("index.html has no meta description");

// Structured data (schema.org), for search engines: what Quill is, for
// which systems, what it costs and where to get it.
const publisher = { "@type": "Organization", name: "Fisher Innovation", url: "https://fisherinnovation.com" };
const jsonLd = {
  "@context": "https://schema.org",
  "@graph": [
    { "@type": "WebSite", name: "Quill", url: siteUrl, publisher },
    {
      "@type": "SoftwareApplication",
      name: "Quill",
      description,
      url: siteUrl,
      image: `${siteUrl}media/card.jpg`,
      screenshot: `${siteUrl}media/hero-light.jpg`,
      applicationCategory: "BusinessApplication",
      operatingSystem: "macOS 14.6 or later (Apple Silicon), Windows 10, Windows 11",
      softwareVersion: (release?.tag ?? version).replace(/^v/, ""),
      offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
      isAccessibleForFree: true,
      publisher,
      ...(flags.release && { downloadUrl: [release.mac, release.windows] }),
      ...(isPublic && { sameAs: [repoUrl] }),
    },
  ],
};

const values = {
  SITE_URL: siteUrl,
  // The preview card's alt text (site/card/index.html), for link previews.
  CARD_ALT: "Quill: meeting notes without the meeting bot. Live notes beside a video call.",
  // "<" escaped, so nothing in it can close the <script> it sits in.
  JSON_LD: JSON.stringify(jsonLd, null, 2).replace(/</g, "\\u003c"),
  REPO_URL: repoUrl,
  VERSION: release?.tag ?? `v${version}`,
  RELEASE_TAG: release?.tag ?? "",
  MAC_URL: release?.mac ?? "",
  WINDOWS_URL: release?.windows ?? "",
  BUILD: (env.GITHUB_SHA ?? String(Date.now())).slice(0, 8),
  // What the header video shows, for screen readers (site/video/script.json).
  VIDEO_LABEL: JSON.parse(readFileSync(join(here, "video/script.json"), "utf8")).label,
  // And the overview's (site/overview/script.json), as an attribute.
  OVERVIEW_LABEL: JSON.parse(readFileSync(join(here, "overview/script.json"), "utf8")).label.replace(
    /[&"<]/g,
    (c) => ({ "&": "&amp;", '"': "&quot;", "<": "&lt;" })[c],
  ),
};

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
if (siteUrl) {
  writeFileSync(
    join(out, "sitemap.xml"),
    `<?xml version="1.0" encoding="UTF-8"?>\n` +
      `<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n` +
      `  <url><loc>${siteUrl}</loc></url>\n` +
      `</urlset>\n`,
  );
  // Crawlers only read robots.txt at a domain's root, which the site is
  // at (quill.fisherinnovation.com, set in the repository's Pages settings).
  writeFileSync(join(out, "robots.txt"), `User-agent: *\nAllow: /\n\nSitemap: ${siteUrl}sitemap.xml\n`);
}

console.log(
  `site/_site: ${values.VERSION}, ${flags.release ? "with downloads" : "no release yet"}, ` +
    `${flags.public ? "public source" : "private source"}`,
);
