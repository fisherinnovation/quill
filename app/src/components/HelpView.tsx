// Help: how Quill works, as topics with sections (help/content.ts), a
// table of contents that follows the reading, and search over all of it.
import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { isMac } from "../api";
import { TOPICS, type HelpSection, type HelpTopic } from "../help/content";
import { Icon, type IconName } from "./Icon";

/** Ways in for someone new, shown above the topics. */
const QUICK: { id: string; icon: IconName; title: string; text: string }[] = [
  { id: "record-meeting", icon: "users", title: "Record a meeting", text: "Notes and a transcript from any call." },
  { id: "dictating", icon: "mic", title: "Dictate anywhere", text: "Hold a key, speak, and it's typed." },
  { id: "what-leaves", icon: "lock", title: "Keep it private", text: "What stays on your computer." },
  { id: "dictation-not-typing", icon: "alert", title: "Fix a problem", text: "The most common fixes." },
];

/** Section ids are prefixed in the page, so they can't clash with others. */
const anchor = (id: string) => `help-${id}`;

interface Entry {
  topic: HelpTopic;
  section: HelpSection;
  /** Plain text, for matching and snippets. */
  text: string;
  /** Lower-cased (and accent-free) title, keywords, topic and text. */
  title: string;
  keywords: string;
  topicTitle: string;
  body: string;
}

/** Markdown down to the words a reader sees. */
function plain(markdown: string): string {
  return markdown
    .replace(/`key:([^`]+)`/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^\s*\|?\s*-{3,}.*$/gm, " ")
    .replace(/^\s*(?:[-*]|\d+\.)\s+/gm, "")
    .replace(/[*_`]/g, "")
    .replace(/[#>|]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function fold(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

const ENTRIES: Entry[] = TOPICS.flatMap((topic) =>
  topic.sections.map((section) => {
    const text = plain(section.body);
    return {
      topic,
      section,
      text,
      title: fold(section.title),
      keywords: fold(section.keywords ?? ""),
      topicTitle: fold(topic.title),
      body: fold(text),
    };
  }),
);

/** Words that say nothing about the topic, so "how do I…" questions work. */
const FILLER = new Set(
  "a an and are be can do does doesn't for from get how i i'm in is it it's my of on or the to what when where which why with you your".split(
    " ",
  ),
);

function terms(query: string): string[] {
  const words = fold(query)
    .split(/[\s,?!.]+/)
    .map((t) => t.replace(/[^\p{L}\p{N}'’-]/gu, ""))
    .filter((t) => t.length > 0);
  const meaningful = words.filter((t) => !FILLER.has(t));
  return meaningful.length > 0 ? meaningful : words;
}

function count(haystack: string, needle: string): number {
  let n = 0;
  for (let i = haystack.indexOf(needle); i !== -1 && n < 5; i = haystack.indexOf(needle, i + needle.length)) n++;
  return n;
}

/**
 * Sections matching the words, best first: a word in the title counts most
 * (more at the start of a word), then the keywords and the topic, then how
 * often it's in the text. Sections with every word come first; if none
 * has them all, those with the most of them.
 */
function search(words: string[]): Entry[] {
  if (words.length === 0) return [];
  const scored: { entry: Entry; matched: number; score: number }[] = [];
  for (const entry of ENTRIES) {
    let score = 0;
    let matched = 0;
    for (const word of words) {
      const inTitle = entry.title.includes(word);
      const inKeywords = entry.keywords.includes(word);
      const inTopic = entry.topicTitle.includes(word);
      const inBody = count(entry.body, word);
      if (!inTitle && !inKeywords && !inTopic && inBody === 0) continue;
      matched++;
      if (inTitle) score += new RegExp(`(^|[^\\p{L}])${escape(word)}`, "u").test(entry.title) ? 14 : 8;
      if (inKeywords) score += 5;
      if (inTopic) score += 2;
      score += inBody;
    }
    if (matched > 0) scored.push({ entry, matched, score });
  }
  const best = Math.max(0, ...scored.map((s) => s.matched));
  // With several words, a section matching only one of them is noise
  // unless nothing matches more.
  const floor = best === words.length ? words.length : Math.max(1, best);
  return scored
    .filter((s) => s.matched >= floor)
    .sort((a, b) => b.matched - a.matched || b.score - a.score)
    .map((s) => s.entry);
}

function escape(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** A stretch of the text around the first match, cut at word boundaries. */
function snippet(entry: Entry, words: string[]): string {
  const at = Math.min(...words.map((w) => entry.body.indexOf(w)).filter((i) => i >= 0));
  if (!Number.isFinite(at)) return entry.text.slice(0, 160) + (entry.text.length > 160 ? "…" : "");
  // About 220 characters, starting a little before the match, or ending
  // at the end of the text when the match is near it.
  let start = Math.max(0, at - 60);
  let end = Math.min(entry.text.length, start + 220);
  if (end === entry.text.length) start = Math.max(0, end - 220);
  if (start > 0) start = entry.text.indexOf(" ", start) + 1;
  if (end < entry.text.length) end = entry.text.lastIndexOf(" ", end);
  return `${start > 0 ? "…" : ""}${entry.text.slice(start, end)}${end < entry.text.length ? "…" : ""}`;
}

/** `text` with the search terms marked. */
function marked(text: string, words: string[]): ReactNode {
  if (words.length === 0) return text;
  // Matched against the folded text, so "resume" finds "résumé" too;
  // folding keeps one character per character for the text here.
  const folded = fold(text);
  const pattern = new RegExp(words.map(escape).join("|"), "g");
  const parts: ReactNode[] = [];
  let last = 0;
  if (folded.length === text.length) {
    for (const m of folded.matchAll(pattern)) {
      const at = m.index ?? 0;
      if (at > last) parts.push(text.slice(last, at));
      parts.push(<mark key={at}>{text.slice(at, at + m[0].length)}</mark>);
      last = at + m[0].length;
    }
  }
  parts.push(text.slice(last));
  return parts;
}

export function HelpView() {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [current, setCurrent] = useState(TOPICS[0].sections[0].id);
  const [flash, setFlash] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const page = useRef<HTMLDivElement>(null);
  // Where to scroll once the articles are showing again.
  const pending = useRef<string | null>(null);

  const words = useMemo(() => terms(query), [query]);
  const results = useMemo(() => search(words), [words]);
  const searching = words.length > 0;

  function go(id: string) {
    pending.current = id;
    if (searching || query) setQuery("");
    else reveal();
  }

  function reveal() {
    const id = pending.current;
    pending.current = null;
    if (!id) return;
    const el = document.getElementById(anchor(id));
    if (!el) return;
    const smooth = !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    el.scrollIntoView({ block: "start", behavior: smooth ? "smooth" : "auto" });
    setCurrent(id);
    setFlash(id);
  }

  // After a search closes, the articles are back: scroll to the one picked.
  useEffect(() => {
    if (!searching) reveal();
  });

  useEffect(() => {
    if (!flash) return;
    const t = window.setTimeout(() => setFlash(null), 1600);
    return () => window.clearTimeout(t);
  }, [flash]);

  useEffect(() => setActive(0), [query]);

  // Keeps the current entry in view in a table of contents taller than
  // the window, by scrolling the table itself (not the page).
  useEffect(() => {
    const toc = page.current?.querySelector<HTMLElement>(".help-toc");
    const link = toc?.querySelector<HTMLElement>("a.current");
    if (!toc || !link) return;
    const margin = 32;
    if (link.offsetTop - margin < toc.scrollTop) {
      toc.scrollTop = link.offsetTop - margin;
    } else if (link.offsetTop + link.offsetHeight + margin > toc.scrollTop + toc.clientHeight) {
      toc.scrollTop = link.offsetTop + link.offsetHeight + margin - toc.clientHeight;
    }
  }, [current]);

  // ⌘F / Ctrl+F, or / outside a text field, goes to the search box.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;
      const find = (isMac ? e.metaKey : e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "f";
      if (find || (e.key === "/" && !typing && !e.metaKey && !e.ctrlKey)) {
        e.preventDefault();
        input.current?.focus();
        input.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The table of contents follows the reading: the last section whose top
  // has scrolled past the top of the page.
  useEffect(() => {
    if (searching) return;
    const scroller = page.current?.closest(".main-scroll");
    if (!scroller) return;
    let frame = 0;
    const update = () => {
      frame = 0;
      const top = scroller.getBoundingClientRect().top + 96;
      let id = ENTRIES[0].section.id;
      for (const entry of ENTRIES) {
        const el = document.getElementById(anchor(entry.section.id));
        if (el && el.getBoundingClientRect().top <= top) id = entry.section.id;
      }
      // At the very bottom, the last section is the one being read.
      if (scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 4) {
        id = ENTRIES[ENTRIES.length - 1].section.id;
      }
      setCurrent(id);
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(update);
    };
    scroller.addEventListener("scroll", onScroll, { passive: true });
    update();
    return () => {
      scroller.removeEventListener("scroll", onScroll);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [searching]);

  const components: Components = {
    code: ({ children }) => {
      const text = String(children);
      return text.startsWith("key:") ? <kbd>{text.slice(4)}</kbd> : <code>{children}</code>;
    },
    a: ({ href, children }) =>
      href?.startsWith("#") ? (
        <a
          href={href}
          onClick={(e) => {
            e.preventDefault();
            go(href.slice(1));
          }}
        >
          {children}
        </a>
      ) : (
        <>{children}</>
      ),
    table: ({ children }) => (
      <div className="help-table">
        <table>{children}</table>
      </div>
    ),
  };

  const onSearchKey = (e: ReactKeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown" && results.length > 0) {
      e.preventDefault();
      setActive((i) => Math.min(results.length - 1, i + 1));
    } else if (e.key === "ArrowUp" && results.length > 0) {
      e.preventDefault();
      setActive((i) => Math.max(0, i - 1));
    } else if (e.key === "Enter" && results[active]) {
      e.preventDefault();
      go(results[active].section.id);
      input.current?.blur();
    } else if (e.key === "Escape") {
      if (query) setQuery("");
      else input.current?.blur();
    }
  };

  const shortcut = isMac ? "⌘F" : "Ctrl+F";

  return (
    <div className="page help" ref={page}>
      <header className="help-head">
        <h1 className="page-title">Help</h1>
        <p className="page-subtitle">How Quill works, and how to get the most from it.</p>
        <label className="search help-search">
          <Icon name="search" size={15} />
          <input
            ref={input}
            type="search"
            placeholder="Search help"
            aria-label="Search help"
            aria-controls="help-results"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onSearchKey}
            spellCheck={false}
          />
          {!query && <kbd className="help-search-key">{shortcut}</kbd>}
        </label>
      </header>

      {searching ? (
        <section className="help-results" id="help-results" aria-live="polite">
          <p className="help-count">
            {results.length === 0
              ? `Nothing found for “${query.trim()}”.`
              : `${results.length} ${results.length === 1 ? "result" : "results"}`}
          </p>
          {results.length === 0 ? (
            <p className="help-none">Try fewer or different words, or browse the topics below.</p>
          ) : (
            <ul>
              {results.map((entry, i) => (
                <li key={entry.section.id}>
                  <button
                    className={`help-result${i === active ? " active" : ""}`}
                    onClick={() => go(entry.section.id)}
                    onMouseEnter={() => setActive(i)}
                  >
                    <span className="help-result-topic">{entry.topic.title}</span>
                    <span className="help-result-title">{marked(entry.section.title, words)}</span>
                    <span className="help-result-text">{marked(snippet(entry, words), words)}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
          {results.length === 0 && (
            <div className="help-browse">
              {TOPICS.map((topic) => (
                <button key={topic.id} className="link-button" onClick={() => go(topic.sections[0].id)}>
                  {topic.title}
                </button>
              ))}
            </div>
          )}
        </section>
      ) : (
        <div className="help-layout">
          <nav className="help-toc" aria-label="Help topics">
            {TOPICS.map((topic) => (
              <div key={topic.id} className="help-toc-topic">
                <p className="help-toc-title">{topic.title}</p>
                {topic.sections.map((section) => (
                  <a
                    key={section.id}
                    href={`#${section.id}`}
                    className={current === section.id ? "current" : undefined}
                    aria-current={current === section.id ? "location" : undefined}
                    onClick={(e) => {
                      e.preventDefault();
                      go(section.id);
                    }}
                  >
                    {section.title}
                  </a>
                ))}
              </div>
            ))}
          </nav>

          <div className="help-articles">
            <div className="help-quick">
              {QUICK.map((q) => (
                <button key={q.id} className="help-quick-card" onClick={() => go(q.id)}>
                  <Icon name={q.icon} size={16} />
                  <span className="help-quick-title">{q.title}</span>
                  <span className="help-quick-text">{q.text}</span>
                </button>
              ))}
            </div>

            {TOPICS.map((topic) => (
              <section key={topic.id} className="help-topic" aria-labelledby={`help-topic-${topic.id}`}>
                <h2 id={`help-topic-${topic.id}`}>{topic.title}</h2>
                <p className="help-topic-intro">{topic.intro}</p>
                {topic.sections.map((section) => (
                  <article
                    key={section.id}
                    id={anchor(section.id)}
                    className={`help-section${flash === section.id ? " flash" : ""}`}
                  >
                    <h3>{section.title}</h3>
                    <div className="help-body">
                      <Markdown remarkPlugins={[remarkGfm]} components={components}>
                        {section.body}
                      </Markdown>
                    </div>
                  </article>
                ))}
              </section>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
