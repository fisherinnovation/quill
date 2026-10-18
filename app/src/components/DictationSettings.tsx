import { useEffect, useState, type KeyboardEvent } from "react";
import {
  api,
  events,
  fileManager,
  fileSize,
  FN_HOTKEY,
  hotkeyLabel,
  isMac,
  type AppStyleInfo,
  type DictationSettings,
  type Style,
  type Replacement,
  type SamplesUsage,
  type Suggestion,
} from "../api";
import { Icon } from "./Icon";
import { ModelPicker } from "./ModelPicker";
import { ModelDownload } from "./Models";
import { Banner, Group, Keys, LANGUAGES, Row, Switch } from "./ui";

/**
 * Settings → Dictation: the shortcut, clean-up, styles, words, history and
 * samples. The dictations themselves are on the Dictations page.
 */
export function DictationSettingsTab({
  settings,
  update,
  onReviewSamples,
}: {
  settings: DictationSettings;
  update: (patch: Partial<DictationSettings>) => void;
  /** Opens the page that reviews the saved samples. */
  onReviewSamples?: () => void;
}) {
  const [systemLanguage, setSystemLanguage] = useState<string | null>(null);
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [confirmClear, setConfirmClear] = useState(false);
  const [cleared, setCleared] = useState(false);
  // Bumped when removing a learned rule changes the vocabulary or
  // replacements, so their text boxes show it.
  const [wordsRevision, setWordsRevision] = useState(0);

  useEffect(() => {
    void api.systemLanguage().then(setSystemLanguage);
    void api.listSuggestions().then(setSuggestions);
  }, []);

  async function undo(s: Suggestion) {
    setSuggestions(await api.setSuggestionStatus(s.id, "dismissed"));
    update(withRule(settings, s, false));
    setWordsRevision((n) => n + 1);
  }
  const languageName = (code: string | null) => LANGUAGES.find(([c]) => c === code)?.[1] ?? code ?? "unknown";

  return (
    <>
      <ModelDownload variant="banner" />
      <PastePermission />

      <Group title="Shortcut">
        <Row label="Dictation" hint="Listen for the shortcut in every app.">
          <Switch label="Dictation" checked={settings.enabled} onChange={(enabled) => update({ enabled })} />
        </Row>
        <Row
          label="Shortcut"
          hint={
            settings.hotkey === FN_HOTKEY
              ? "Hold fn while you speak, or tap it to start and tap again to stop. fn with another key still works as usual."
              : "Hold it while you speak, or tap it to start and tap again to stop."
          }
        >
          {isMac ? (
            <ShortcutChoice value={settings.hotkey} onChange={(hotkey) => update({ hotkey })} />
          ) : (
            <HotkeyInput value={settings.hotkey} onChange={(hotkey) => update({ hotkey })} />
          )}
        </Row>
        {isMac && settings.enabled && settings.hotkey === FN_HOTKEY && <GlobeKeyHint />}
        <Row
          label="Show what you're saying"
          hint="Captions above the dictation pill as you speak. Long dictations are typed sooner, as most is transcribed before you let go."
        >
          <Switch
            label="Show what you're saying"
            checked={settings.live_captions}
            onChange={(live_captions) => update({ live_captions })}
          />
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
            <ModelPicker
              purpose="cleanup"
              value={settings.polish_model}
              onChange={(polish_model) => update({ polish_model })}
            />
          </Row>
        )}
        {settings.polish && (
          <div className="row stack">
            <div className="row-label">
              <span>About you</span>
              <p className="row-hint">
                Helps the clean-up get names, jargon and spelling right. It's never added to your text, and it's sent
                only with text being tidied.
              </p>
            </div>
            <ListInput
              rows={3}
              placeholder={"I'm a product designer at Northwind. I work with Priya and Dan.\nUse British spelling. “PR” means pull request."}
              value={settings.about}
              onChange={(about) => update({ about })}
              spellCheck
            />
          </div>
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
            key={`vocabulary-${wordsRevision}`}
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
            key={`replacements-${wordsRevision}`}
            rows={3}
            placeholder="open router → OpenRouter"
            value={settings.replacements.map((r) => `${r.from} → ${r.to}`).join("\n")}
            onChange={(text) => update({ replacements: parseReplacements(text) })}
          />
        </div>
        <Learned suggestions={suggestions} settings={settings} onUndo={undo} />
      </Group>

      <Group title="History">
        <Row
          label="Keep a history"
          hint="Stored only on this computer and listed under Dictations, so you can copy or correct something later."
        >
          <Switch
            label="Keep a history"
            checked={settings.keep_history}
            onChange={(keep_history) => update({ keep_history })}
          />
        </Row>
        <Row
          label="Delete all history"
          hint={cleared ? "Deleted." : "Removes every dictation listed under Dictations. Learned words stay."}
        >
          {confirmClear ? (
            <div className="confirm">
              Delete all?
              <button
                className="danger"
                onClick={async () => {
                  await api.clearDictations();
                  setConfirmClear(false);
                  setCleared(true);
                }}
              >
                Delete
              </button>
              <button onClick={() => setConfirmClear(false)}>Cancel</button>
            </div>
          ) : (
            <button onClick={() => setConfirmClear(true)}>Delete…</button>
          )}
        </Row>
      </Group>

      <Samples
        onReview={onReviewSamples}
        enabled={settings.save_samples}
        onChange={(save_samples) => update({ save_samples })}
      />
    </>
  );
}

/** Accessibility (macOS) lets Quill press ⌘V for you. */
export function PastePermission() {
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

/** `settings` with a suggestion's rule added (or taken out). */
export function withRule(settings: DictationSettings, s: Suggestion, add: boolean): DictationSettings {
  if (s.kind === "vocabulary") {
    const rest = settings.vocabulary.filter((v) => v !== s.to);
    return { ...settings, vocabulary: add ? [...rest, s.to] : rest };
  }
  const rest = settings.replacements.filter((r) => !(r.from.toLowerCase() === s.from.toLowerCase() && r.to === s.to));
  return { ...settings, replacements: add ? [...rest, { from: s.from, to: s.to }] : rest };
}

/** Rules learned from corrections that are still in use, to undo. */
function Learned({
  suggestions,
  settings,
  onUndo,
}: {
  suggestions: Suggestion[];
  settings: DictationSettings;
  onUndo: (s: Suggestion) => void;
}) {
  // Accepted rules still in the settings (the user may have deleted them by hand).
  const learned = suggestions.filter(
    (s) =>
      s.status === "accepted" &&
      (s.kind === "vocabulary"
        ? settings.vocabulary.includes(s.to)
        : settings.replacements.some((r) => r.from.toLowerCase() === s.from.toLowerCase() && r.to === s.to)),
  );
  if (learned.length === 0) return null;
  return (
    <div className="row stack">
      <div className="row-label">
        <span>Learned from your fixes</span>
        <p className="row-hint">Remove one to stop using it.</p>
      </div>
      <div className="learned-list">
        {learned.map((s) => (
          <span key={s.id} className="learned-chip">
            {s.kind === "vocabulary" ? s.to : `${s.from} → ${s.to}`}
            <button className="ghost icon-btn" aria-label={`Remove ${s.to}`} title="Remove" onClick={() => onUndo(s)}>
              <Icon name="x" size={12} />
            </button>
          </span>
        ))}
      </div>
    </div>
  );
}

/**
 * Keeping dictations' audio to measure accuracy with `quill-cli dictate`
 * (eval/dictation/README.md).
 */
function Samples({
  enabled,
  onChange,
  onReview,
}: {
  enabled: boolean;
  onChange: (on: boolean) => void;
  onReview?: () => void;
}) {
  const [usage, setUsage] = useState<SamplesUsage | null>(null);
  const [reviewed, setReviewed] = useState<number | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  useEffect(() => {
    void api.dictationSamples().then(setUsage);
    void api
      .sampleReview()
      .then((r) => setReviewed(r.accuracy.reviewed))
      .catch(() => {});
    const unlisten = events.onDictationSamples(setUsage);
    return () => void unlisten.then((f) => f());
  }, []);

  return (
    <Group title="Testing accuracy">
      <Row
        label="Save dictations for testing"
        hint="Keeps the audio of each dictation and what was typed, only on this computer, to measure accuracy with quill-cli. About 1 MB per minute of speech."
      >
        <Switch label="Save dictations for testing" checked={enabled} onChange={onChange} />
      </Row>
      {usage && usage.count > 0 && onReview && (
        <Row
          label="Check what was typed"
          hint={`Listen to each sample and fix what was typed. Your fixes measure accuracy on your own voice.${reviewed !== null ? ` ${reviewed} of ${usage.count} reviewed.` : ""}`}
        >
          <button className="primary" onClick={onReview}>
            Review…
          </button>
        </Row>
      )}
      {usage && usage.count > 0 && (
        <Row
          label={`${usage.count} saved`}
          hint={`${fileSize(usage.bytes)}. Deleting them also deletes any reference text you wrote beside them.`}
        >
          <button onClick={() => void api.showDictationSamples()}>Show in {fileManager}</button>
          {confirmClear ? (
            <div className="confirm">
              Delete all?
              <button
                className="danger"
                onClick={async () => {
                  await api.clearDictationSamples();
                  setConfirmClear(false);
                  setUsage(await api.dictationSamples());
                }}
              >
                Delete
              </button>
              <button onClick={() => setConfirmClear(false)}>Cancel</button>
            </div>
          ) : (
            <button onClick={() => setConfirmClear(true)}>Delete…</button>
          )}
        </Row>
      )}
    </Group>
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

/**
 * macOS: the fn (🌐) key on its own, or a shortcut. fn can't be recorded
 * like other keys (macOS reports it only as a modifier), so it's a choice.
 */
function ShortcutChoice({ value, onChange }: { value: string; onChange: (hotkey: string) => void }) {
  const isFn = value === FN_HOTKEY;
  // The shortcut to go back to when switching away from fn.
  const [custom, setCustom] = useState(isFn ? "Alt+Space" : value);
  useEffect(() => {
    if (value !== FN_HOTKEY) setCustom(value);
  }, [value]);
  return (
    <span className="shortcut-choice" role="group" aria-label="Dictation shortcut">
      <button type="button" className={`hotkey fn-key${isFn ? " selected" : ""}`} aria-pressed={isFn} onClick={() => onChange(FN_HOTKEY)}>
        <kbd>fn</kbd>
      </button>
      <span className="or">or</span>
      <HotkeyInput
        value={custom}
        selected={!isFn}
        onStart={() => isFn && onChange(custom)}
        onChange={onChange}
      />
    </span>
  );
}

/**
 * When fn is the shortcut and macOS also acts on 🌐 (emoji picker, input
 * source, Apple dictation), says so and links to the setting.
 */
function GlobeKeyHint() {
  const [action, setAction] = useState<Awaited<ReturnType<typeof api.globeKeyAction>>>(null);
  useEffect(() => {
    const check = () => void api.globeKeyAction().then(setAction);
    check();
    // Re-check when coming back from System Settings.
    window.addEventListener("focus", check);
    return () => window.removeEventListener("focus", check);
  }, []);
  if (action === null || action === "nothing") return null;
  const what = {
    change_input_source: "switches the input source",
    show_emoji: "opens the emoji picker",
    start_dictation: "starts Apple's dictation",
    default: "may open the emoji picker or switch the input source",
  }[action];
  return (
    <Row
      label={
        <span className="row-gap">
          <Icon name="alert" size={14} /> macOS also uses fn
        </span>
      }
      hint={
        <>
          Pressing fn on its own also {what}. To use it only for Quill, set <strong>Press 🌐 key to</strong> to{" "}
          <strong>Do Nothing</strong> in Keyboard settings.
        </>
      }
    >
      <button onClick={() => void api.openKeyboardSettings()}>Open Keyboard Settings</button>
    </Row>
  );
}

/** Click, then press the new shortcut. Esc cancels. */
function HotkeyInput({
  value,
  onChange,
  selected,
  onStart,
}: {
  value: string;
  onChange: (hotkey: string) => void;
  /** In a choice (ShortcutChoice): whether this is the one in use. */
  selected?: boolean;
  onStart?: () => void;
}) {
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
      className={`hotkey${capturing ? " capturing" : ""}${selected ? " selected" : ""}`}
      aria-pressed={selected}
      title={capturing ? undefined : `${hotkeyLabel(value)}: click to change`}
      onClick={() => {
        onStart?.();
        setCapturing(true);
      }}
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
  spellCheck = false,
}: {
  value: string;
  onChange: (text: string) => void;
  rows: number;
  placeholder: string;
  spellCheck?: boolean;
}) {
  const [text, setText] = useState(value);
  return (
    <textarea
      rows={rows}
      spellCheck={spellCheck}
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
