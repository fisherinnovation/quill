import { useEffect, useState } from "react";
import { api, type DictationSettings, type DictationState, type Suggestion } from "../api";
import { useDictationHistory, dictationTitle } from "./DictationHistory";
import { PastePermission, withRule } from "./DictationSettings";
import { Icon } from "./Icon";
import { ModelDownload } from "./Models";
import { Banner, Group, Keys, Row, groupByDay, timeOfDay } from "./ui";

/**
 * The Dictations page: whether dictation is ready, fixes waiting for the
 * user's OK, and the history. Its settings are under Settings → Dictation.
 */
export function DictationsView({
  state,
  onOpen,
  onSettings,
}: {
  state: DictationState;
  onOpen: (id: number) => void;
  /** Shows Settings → Dictation. */
  onSettings: () => void;
}) {
  const history = useDictationHistory();
  const [settings, setSettings] = useState<DictationSettings | null>(null);
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);

  useEffect(() => {
    void api.getSettings().then((s) => setSettings(s.dictation));
    void api.listSuggestions().then(setSuggestions);
  }, []);

  async function decide(s: Suggestion, accept: boolean) {
    setSuggestions(await api.setSuggestionStatus(s.id, accept ? "accepted" : "dismissed"));
    if (accept) {
      // Read fresh: the settings may have changed since this page opened.
      const dictation = withRule((await api.getSettings()).dictation, s, true);
      await api.saveDictationSettings(dictation);
      setSettings(dictation);
    }
  }

  if (!settings) return null;
  const pending = suggestions.filter((s) => s.status === "pending");

  return (
    <div className="page dictations">
      <header className="page-head">
        <div>
          <h1 className="page-title">Dictations</h1>
          <p className="page-subtitle">Speak instead of typing, in any app.</p>
        </div>
        <span className="spacer" />
        <button onClick={onSettings}>
          <Icon name="settings" size={14} /> Settings
        </button>
      </header>

      <Hero state={state} hotkey={settings.hotkey} enabled={settings.enabled} onSettings={onSettings} />
      <ModelDownload variant="banner" />
      {state.error && (
        <Banner kind="error" icon="alert">
          {state.error}
        </Banner>
      )}
      <PastePermission />

      {pending.length > 0 && (
        <Group title="Learn from your fixes">
          {pending.map((s) => (
            <Row
              key={s.id}
              label={
                s.kind === "vocabulary" ? (
                  <>
                    Always spell <strong>{s.to}</strong> this way
                  </>
                ) : (
                  <>
                    Type <strong>{s.to}</strong> when you say “{s.from}”
                  </>
                )
              }
              hint={`${s.kind === "vocabulary" ? "Adds it to your vocabulary" : "Adds a replacement"}. From your fix${s.app ? ` in ${s.app}` : ""}${s.seen > 1 ? `, seen ${s.seen} times` : ""}.`}
            >
              <span className="row-gap">
                <button className="ghost" onClick={() => void decide(s, false)}>
                  Dismiss
                </button>
                <button className="primary" onClick={() => void decide(s, true)}>
                  Add
                </button>
              </span>
            </Row>
          ))}
        </Group>
      )}

      {history.entries.length === 0 ? (
        <div className="card home-empty">
          <div className="home-card-icon">
            <Icon name="mic" size={17} />
          </div>
          {settings.keep_history ? (
            <div>
              <strong>Your dictations will appear here</strong>
              <p className="muted small">
                Open one to copy it, or to correct it so Quill learns the names and terms it got wrong.
              </p>
            </div>
          ) : (
            <>
              <div>
                <strong>History is off</strong>
                <p className="muted small">Dictation still works; what you say just isn't kept here.</p>
              </div>
              <span className="spacer" />
              <button onClick={onSettings}>Turn on…</button>
            </>
          )}
        </div>
      ) : (
        <>
          {groupByDay(history.entries, (d) => d.created_at).map(([label, group]) => (
            <section className="group" key={label}>
              <h2 className="group-title">{label}</h2>
              <ul className="card item-list">
                {group.map((d) => (
                  <li key={d.id}>
                    <button className="item-row" onClick={() => onOpen(d.id)}>
                      <span className="item-text">{dictationTitle(d)}</span>
                      <span className="item-meta">
                        {timeOfDay(d.created_at)}
                        {d.app && <span className="dot-sep">{d.app}</span>}
                        {d.corrected && <span className="dot-sep">corrected</span>}
                        {!d.pasted && <span className="dot-sep">left on clipboard</span>}
                      </span>
                      <Icon name="external" size={13} />
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          ))}
          {history.more && (
            <button className="show-more" onClick={history.loadMore}>
              Show more dictations
            </button>
          )}
        </>
      )}
    </div>
  );
}

function Hero({
  state,
  hotkey,
  enabled,
  onSettings,
}: {
  state: DictationState;
  hotkey: string;
  enabled: boolean;
  onSettings: () => void;
}) {
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
                ? "Turn it on to dictate in any app."
                : "Let go and the text is typed where your cursor is. Or tap to start and tap again to stop."}
        </p>
      </div>
      {phase === "idle" && (
        <span className="pill green">
          <span className="rec-dot" style={{ width: 7, height: 7 }} /> Ready
        </span>
      )}
      {phase === "off" && (
        <button className="primary" onClick={onSettings}>
          Turn on…
        </button>
      )}
    </div>
  );
}
