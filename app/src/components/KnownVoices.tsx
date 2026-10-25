import { useEffect, useState } from "react";
import { api, errorText, isMac, type KnownVoice } from "../api";
import { Icon } from "./Icon";
import { Row, Switch } from "./ui";

/**
 * Settings → Meetings: recognizing people by voice across meetings, and
 * the voices Quill remembers, to forget one or all (quill_core::voices).
 */
export function KnownVoicesRows({ enabled, onChange }: { enabled: boolean; onChange: (on: boolean) => void }) {
  const [voices, setVoices] = useState<KnownVoice[] | null>(null);
  const [confirmForget, setConfirmForget] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .knownVoices()
      .then(setVoices)
      .catch((e) => setError(errorText(e)));
  }, []);

  const forget = async (voice: KnownVoice) => {
    try {
      await api.deleteKnownVoice(voice.id);
      setVoices((vs) => vs && vs.filter((v) => v.id !== voice.id));
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <>
      <Row
        label="Recognize voices"
        hint={`When you name someone in a transcript, Quill remembers their voice on this ${isMac ? "Mac" : "PC"} and names them in later meetings, marked so you can check. It names someone only when it's confident; otherwise they stay Speaker 1, 2…`}
      >
        <Switch label="Recognize voices" checked={enabled} onChange={onChange} />
      </Row>
      {voices && voices.length > 0 && (
        <div className="row stack">
          <div className="row-label">
            <span>Known voices</span>
            <p className="row-hint">
              Voice fingerprints, kept only in your library. Remove one to forget it; transcripts keep the name.
            </p>
          </div>
          <div className="learned-list">
            {voices.map((v) => (
              <span
                key={v.id}
                className="learned-chip"
                title={`Learned from ${v.meetings} meeting${v.meetings === 1 ? "" : "s"}`}
              >
                {v.name}
                <button
                  className="ghost icon-btn"
                  aria-label={`Forget ${v.name}'s voice`}
                  title="Forget"
                  onClick={() => void forget(v)}
                >
                  <Icon name="x" size={12} />
                </button>
              </span>
            ))}
          </div>
        </div>
      )}
      {voices && voices.length > 0 && (
        <Row label="Forget all voices" hint="Deletes every known voice. Names already in transcripts stay.">
          {confirmForget ? (
            <div className="confirm">
              Forget all?
              <button
                className="danger"
                onClick={async () => {
                  try {
                    await api.forgetKnownVoices();
                    setVoices([]);
                  } catch (e) {
                    setError(errorText(e));
                  }
                  setConfirmForget(false);
                }}
              >
                Forget
              </button>
              <button onClick={() => setConfirmForget(false)}>Cancel</button>
            </div>
          ) : (
            <button onClick={() => setConfirmForget(true)}>Forget…</button>
          )}
        </Row>
      )}
      {error && (
        <Row label="Known voices" hint={error}>
          {null}
        </Row>
      )}
    </>
  );
}
