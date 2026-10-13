import { useEffect, useState } from "react";
import { api, errorText, isMac, type KeySource, keyStore } from "../api";
import { Icon } from "./Icon";

/**
 * Onboarding's optional OpenRouter step: paste a key, or skip it and use
 * a local Ollama model later. Settings → OpenRouter has the full controls.
 */
export function KeySetup() {
  const [source, setSource] = useState<KeySource | null>(null);
  const [draft, setDraft] = useState("");
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .openrouterKeyStatus()
      .then(setSource)
      .catch(() => setSource("unknown"));
  }, []);

  async function save() {
    if (!draft.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await api.setOpenrouterKey(draft);
      setDraft("");
      setSaved(true);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  if (saved || source === "keychain" || source === "environment") {
    return (
      <div className="model-download">
        <Icon name="check" size={16} />
        <div>
          <strong>{saved ? `Key saved to your ${keyStore}` : "OpenRouter key already set up"}</strong>
        </div>
      </div>
    );
  }
  return (
    <div className="model-download key-setup">
      <div>
        <strong>Add an OpenRouter key</strong>
        <p className="muted small">
          Notes and dictation clean-up use AI models through OpenRouter, which needs a key. It's kept in your{" "}
          {keyStore}. Or skip this and choose a local Ollama model in Settings → Models, so nothing leaves this{" "}
          {isMac ? "Mac" : "PC"}.
        </p>
        <div className="key-setup-form">
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            placeholder="sk-or-…"
            aria-label="OpenRouter API key"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void save();
            }}
          />
          <button className="primary" disabled={busy || !draft.trim()} onClick={() => void save()}>
            Save key
          </button>
          <button className="link" onClick={() => void api.openLink("openrouter_keys")}>
            Get a key <Icon name="external" size={12} />
          </button>
        </div>
        {error && <p className="error-text small">{error}</p>}
      </div>
    </div>
  );
}
