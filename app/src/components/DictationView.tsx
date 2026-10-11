import { useEffect, useState, type KeyboardEvent } from "react";
import {
  api,
  errorText,
  events,
  hotkeyLabel,
  isMac,
  shortDate,
  type AppStyleInfo,
  type DictationEntry,
  type DictationSettings,
  type Style,
  type DictationState,
  type Replacement,
} from "../api";
import { Icon } from "./Icon";
import { Banner, Group, Keys, LANGUAGES, Row, SaveIndicator, Switch, useAutosave } from "./ui";

export function DictationView({
  state,
  onHotkeyChange,
}: {
  state: DictationState;
  onHotkeyChange: (hotkey: string) => void;
}) {
  const [settings, setSettings] = useState<DictationSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [systemLanguage, setSystemLanguage] = useState<string | null>(null);
  const saveState = useAutosave(settings, async (s) => {
    await api.saveDictationSettings(s);
    onHotkeyChange(s.hotkey);
  });

  useEffect(() => {
    api
      .getSettings()
      .then((s) => setSettings(s.dictation))
      .catch((e) => setError(errorText(e)));
    void api.systemLanguage().then(setSystemLanguage);
  }, []);

  if (!settings) return error ? <div className="page error-text">{error}</div> : null;
  const update = (patch: Partial<DictationSettings>) => setSettings({ ...settings, ...patch });
  const languageName = (code: string | null) => LANGUAGES.find(([c]) => c === code)?.[1] ?? code ?? "unknown";

  return (
    <div className="page dictation">
      <div className="page-head">
        <div>
          <h1 className="page-title">Dictation</h1>
          <p className="page-subtitle">Speak instead of typing, in any app.</p>
        </div>
        <span className="spacer" />
        <SaveIndicator state={saveState} />
      </div>

      <Hero state={state} hotkey={settings.hotkey} enabled={settings.enabled} />
      {state.error && (
        <Banner kind="error" icon="alert">
          {state.error}
        </Banner>
      )}
      <PastePermission />

      <Group title="Shortcut">
        <Row label="Dictation" hint="Listen for the shortcut in every app.">
          <Switch label="Dictation" checked={settings.enabled} onChange={(enabled) => update({ enabled })} />
        </Row>
        <Row label="Shortcut" hint="Hold it while you speak, or tap it to start and tap again to stop.">
          <HotkeyInput value={settings.hotkey} onChange={(hotkey) => update({ hotkey })} />
        </Row>
        <Row label="Language">
          <select value={settings.language ?? ""} onChange={(e) => update({ language: e.target.value || null })}>
            <option value="">System language ({languageName(systemLanguage)})</option>
            <option value="auto">Detect each time (about 0.5 s slower)</option>
            {LANGUAGES.map(([code, name]) => (
              <option key={code} value={code}>
                {name}
              </option>
            ))}
          </select>
        </Row>
      </Group>

      <Group title="Clean-up">
        <Row
          label="Tidy up what you said"
          hint={'Removes “um”s, applies corrections like “3, no wait, 4” and formats spoken lists. Only runs when needed.'}
        >
          <Switch label="Tidy up" checked={settings.polish} onChange={(polish) => update({ polish })} />
        </Row>
        {settings.polish && (
          <Row
            label="Model"
            hint={
              settings.polish_model.startsWith("ollama:")
                ? "Runs locally with Ollama."
                : "Via OpenRouter. Only the text is sent, never audio, to providers that don't keep it."
            }
          >
            <input
              value={settings.polish_model}
              spellCheck={false}
              onChange={(e) => update({ polish_model: e.target.value.trim() })}
            />
          </Row>
        )}
      </Group>

      <AppStyles settings={settings} onChange={(app_styles) => update({ app_styles })} />

      <Group title="Words">
        <div className="row stack">
          <div className="row-label">
            <span>Vocabulary</span>
            <p className="row-hint">Names and terms to spell exactly, one per line.</p>
          </div>
          <ListInput
            rows={3}
            placeholder={"Quill\nOpenRouter"}
            value={settings.vocabulary.join("\n")}
            onChange={(text) => update({ vocabulary: lines(text) })}
          />
        </div>
        <div className="row stack">
          <div className="row-label">
            <span>Replacements</span>
            <p className="row-hint">One per line: what you say → what to type.</p>
          </div>
          <ListInput
            rows={3}
            placeholder="open router → OpenRouter"
            value={settings.replacements.map((r) => `${r.from} → ${r.to}`).join("\n")}
            onChange={(text) => update({ replacements: parseReplacements(text) })}
          />
        </div>
      </Group>

      <Group title="History">
        <Row label="Keep a history" hint="Stored only on this computer, so you can copy something again.">
          <Switch
            label="Keep a history"
            checked={settings.keep_history}
            onChange={(keep_history) => update({ keep_history })}
          />
        </Row>
      </Group>

      {settings.keep_history && <History />}
    </div>
  );
}

function Hero({ state, hotkey, enabled }: { state: DictationState; hotkey: string; enabled: boolean }) {
  const phase = enabled ? state.phase : "off";
  return (
    <div className={`hero ${phase}`}>
      <div className="hero-mark">
        {phase === "listening" ? (
          <div className="bars live">
            <span />
            <span />
            <span />
            <span />
            <span />
          </div>
        ) : phase === "processing" ? (
          <span className="spinner" style={{ width: 22, height: 22, borderWidth: 2.5 }} />
        ) : (
          <Icon name="mic" size={26} />
        )}
      </div>
      <div className="hero-body">
        <div className="hero-title">
          {phase === "listening" ? (
            <>Listening…</>
          ) : phase === "processing" ? (
            <>Typing it out…</>
          ) : phase === "off" ? (
            <>Dictation is off</>
          ) : (
            <>
              Hold <Keys hotkey={hotkey} /> and speak
            </>
          )}
        </div>
        <p>
          {phase === "listening"
            ? "Let go (or tap the shortcut again) to type it. Esc cancels."
            : phase === "processing"
              ? "The text goes wherever your cursor is."
              : phase === "off"
                ? "Turn it on below to dictate in any app."
                : "Let go and the text is typed where your cursor is. Or tap to start and tap again to stop."}
        </p>
      </div>
      {phase === "idle" && (
        <span className="pill green">
          <span className="rec-dot" style={{ width: 7, height: 7 }} /> Ready
        </span>
      )}
    </div>
  );
}

/** Accessibility (macOS) lets Quill press ⌘V for you. */
function PastePermission() {
  const [allowed, setAllowed] = useState<boolean | null>(null);

  useEffect(() => {
    const check = () => void api.pastePermission().then(setAllowed);
    check();
    // Re-check when coming back from System Settings.
    window.addEventListener("focus", check);
    return () => window.removeEventListener("focus", check);
  }, []);

  if (allowed !== false) return null;
  return (
    <Banner
      kind="warning"
      icon="keyboard"
      action={
        <button className="primary" onClick={() => void api.requestPastePermission().then(setAllowed)}>
          Allow…
        </button>
      }
    >
      <strong>Let Quill type for you.</strong>{" "}
      {isMac ? "Allow it under Privacy & Security → Accessibility." : "Allow it to send keystrokes."} Until then,
      dictated text is left on the clipboard.
    </Banner>
  );
}

function History() {
  const [entries, setEntries] = useState<DictationEntry[]>([]);
  const [copied, setCopied] = useState<number | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);

  useEffect(() => {
    void api.listDictations().then(setEntries);
    const unlisten = events.onDictationEntry((entry) => setEntries((all) => [entry, ...all]));
    return () => void unlisten.then((f) => f());
  }, []);

  async function copy(entry: DictationEntry) {
    await navigator.clipboard.writeText(entry.text);
    setCopied(entry.id);
    setTimeout(() => setCopied(null), 1500);
  }

  return (
    <section className="group">
      <div className="row-gap" style={{ marginBottom: 8 }}>
        <h2 className="group-title" style={{ margin: "0 0 0 2px" }}>
          Recent
        </h2>
        <span className="spacer" />
        {entries.length > 0 &&
          (confirmClear ? (
            <div className="confirm">
              Delete all history?
              <button
                className="danger"
                onClick={async () => {
                  await api.clearDictations();
                  setEntries([]);
                  setConfirmClear(false);
                }}
              >
                Delete
              </button>
              <button onClick={() => setConfirmClear(false)}>Cancel</button>
            </div>
          ) : (
            <button className="ghost" onClick={() => setConfirmClear(true)}>
              Clear all
            </button>
          ))}
      </div>
      <div className="card">
        {entries.length === 0 ? (
          <div className="placeholder" style={{ padding: "32px 20px" }}>
            <div className="placeholder-icon">
              <Icon name="history" size={20} />
            </div>
            <span>Nothing dictated yet.</span>
          </div>
        ) : (
          <ul className="history-list">
            {entries.map((entry) => (
              <li key={entry.id} className="history-item">
                <p className="history-text">{entry.text}</p>
                <div className="history-meta">
                  <span>{shortDate(entry.created_at)}</span>
                  {entry.app && <span className="dot-sep">{entry.app}</span>}
                  <span className="dot-sep">{(entry.latency_ms / 1000).toFixed(1)} s</span>
                  {entry.polish_model && (
                    <span className="dot-sep row-gap" style={{ gap: 4 }}>
                      <Icon name="sparkle" size={12} /> tidied
                    </span>
                  )}
                  {!entry.pasted && <span className="pill amber">Left on clipboard</span>}
                  <span className="spacer" />
                  <span className="history-actions row-gap" style={{ gap: 4 }}>
                    <button className="ghost" onClick={() => void copy(entry)}>
                      <Icon name={copied === entry.id ? "check" : "copy"} size={13} />
                      {copied === entry.id ? "Copied" : "Copy"}
                    </button>
                    <button
                      className="ghost icon-btn"
                      aria-label="Delete"
                      title="Delete"
                      onClick={async () => {
                        await api.deleteDictation(entry.id);
                        setEntries((all) => all.filter((e) => e.id !== entry.id));
                      }}
                    >
                      <Icon name="trash" size={13} />
                    </button>
                  </span>
                </div>
                {entry.raw && (
                  <details>
                    <summary>As spoken</summary>
                    {entry.raw}
                  </details>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}

const STYLES: [Style, string, string][] = [
  ["default", "Default", "As you said it, tidied up"],
  ["casual", "Casual", "Chat: relaxed, no period on one-liners"],
  ["formal", "Formal", "Email and documents: complete sentences"],
  ["technical", "Technical", "Code and terminals: names and commands kept exact"],
];
const styleHint = (style: Style) => STYLES.find(([s]) => s === style)?.[2] ?? "";

/**
 * How text should read in each app. Lists the apps dictated into (and any
 * with a chosen style); picking the built-in style removes the choice.
 */
function AppStyles({
  settings,
  onChange,
}: {
  settings: DictationSettings;
  onChange: (styles: DictationSettings["app_styles"]) => void;
}) {
  const [apps, setApps] = useState<AppStyleInfo[] | null>(null);
  useEffect(() => {
    void api.dictationApps().then(setApps);
  }, []);
  if (!apps) return null;

  const chosen = (app: AppStyleInfo) =>
    settings.app_styles.find((s) => s.app_id.toLowerCase() === app.app_id.toLowerCase())?.style ?? app.built_in;
  function choose(app: AppStyleInfo, style: Style) {
    const others = settings.app_styles.filter((s) => s.app_id.toLowerCase() !== app.app_id.toLowerCase());
    onChange(style === app.built_in ? others : [...others, { app_id: app.app_id, name: app.name, style }]);
  }

  return (
    <Group title="Style per app">
      {apps.length === 0 ? (
        <Row
          label="No apps yet"
          hint="Apps appear here after you dictate into them. Chat apps start out casual, email formal, and code editors technical."
        />
      ) : (
        apps.map((app) => {
          const style = chosen(app);
          return (
            <Row
              key={app.app_id}
              label={app.name}
              hint={style === app.built_in ? `${styleHint(style)} (automatic)` : styleHint(style)}
            >
              <select value={style} onChange={(e) => choose(app, e.target.value as Style)}>
                {STYLES.map(([value, label]) => (
                  <option key={value} value={value}>
                    {value === app.built_in ? `${label} (automatic)` : label}
                  </option>
                ))}
              </select>
            </Row>
          );
        })
      )}
    </Group>
  );
}

/** Click, then press the new shortcut. Esc cancels. */
function HotkeyInput({ value, onChange }: { value: string; onChange: (hotkey: string) => void }) {
  const [capturing, setCapturing] = useState(false);

  function onKeyDown(e: KeyboardEvent<HTMLButtonElement>) {
    if (!capturing) return;
    e.preventDefault();
    if (e.key === "Escape") {
      setCapturing(false);
      return;
    }
    const key = keyName(e.code);
    if (!key) return; // a modifier on its own: wait for the main key
    const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(
      Boolean,
    );
    // Plain keys would fire while typing; require a modifier unless it's F1–F24.
    if (mods.length === 0 && !/^F\d+$/.test(key)) return;
    onChange([...mods, key].join("+"));
    setCapturing(false);
  }

  return (
    <button
      type="button"
      className={`hotkey${capturing ? " capturing" : ""}`}
      title={capturing ? undefined : `${hotkeyLabel(value)}: click to change`}
      onClick={() => setCapturing(true)}
      onBlur={() => setCapturing(false)}
      onKeyDown={onKeyDown}
    >
      {capturing ? "Press a shortcut…" : <Keys hotkey={value} />}
    </button>
  );
}

/** KeyboardEvent.code → the key name the global-shortcut parser expects. */
function keyName(code: string): string | null {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  if (/^F\d+$/.test(code)) return code;
  const named = [
    "Space",
    "Enter",
    "Tab",
    "Backquote",
    "Minus",
    "Equal",
    "BracketLeft",
    "BracketRight",
    "Backslash",
    "Semicolon",
    "Quote",
    "Comma",
    "Period",
    "Slash",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
  ];
  return named.includes(code) ? code : null;
}

/** A textarea that keeps what's typed (blank lines, half-typed rules) while editing. */
function ListInput({
  value,
  onChange,
  rows,
  placeholder,
}: {
  value: string;
  onChange: (text: string) => void;
  rows: number;
  placeholder: string;
}) {
  const [text, setText] = useState(value);
  return (
    <textarea
      rows={rows}
      spellCheck={false}
      placeholder={placeholder}
      value={text}
      onChange={(e) => {
        setText(e.target.value);
        onChange(e.target.value);
      }}
    />
  );
}

function lines(text: string): string[] {
  return text
    .split(/\n|,/)
    .map((l) => l.trim())
    .filter(Boolean);
}

/** "from → to" or "from => to" or "from -> to", one per line. */
function parseReplacements(text: string): Replacement[] {
  return text
    .split("\n")
    .map((line) => line.split(/\s*(?:→|=>|->)\s*/))
    .filter((parts) => parts.length === 2 && parts[0].trim() && parts[1].trim())
    .map(([from, to]) => ({ from: from.trim(), to: to.trim() }));
}
