import { useEffect, useId, useState } from "react";
import { api, type ModelChoices } from "../api";

const OTHER = "__other__";

/** "$0.002" for small amounts, "$0.12" otherwise. */
function dollars(x: number): string {
  return x < 0.01 ? `$${x.toFixed(3)}` : `$${x.toFixed(2)}`;
}

/**
 * Chooses a model for summaries or clean-up: a recommended one, one
 * installed in Ollama, or any OpenRouter model (checked against its
 * catalog), instead of typing an id from memory.
 */
export function ModelPicker({
  purpose,
  value,
  onChange,
}: {
  purpose: "summary" | "cleanup";
  value: string;
  onChange: (model: string) => void;
}) {
  const [choices, setChoices] = useState<ModelChoices | null>(null);
  // Typing another OpenRouter id: the draft, until it's committed.
  const [custom, setCustom] = useState<string | null>(null);
  const listId = useId();

  useEffect(() => {
    void api.modelChoices(purpose).then(setChoices);
  }, [purpose]);

  const recommended = choices?.recommended ?? [];
  const isRecommended = recommended.some((c) => c.id === value);
  const isLocal = value.startsWith("ollama:");
  const selected = custom !== null ? OTHER : isRecommended || isLocal ? value : OTHER;
  const typing = selected === OTHER;
  const otherId = custom ?? (isRecommended || isLocal ? "" : value);
  const listing = choices?.catalog.find((m) => m.id === otherId.trim());

  function pick(next: string) {
    if (next === OTHER) {
      setCustom(isRecommended || isLocal ? "" : value);
      return;
    }
    setCustom(null);
    onChange(next);
  }

  function commit() {
    const id = (custom ?? "").trim();
    if (id) onChange(id);
  }

  const installed = choices ? choices.ollama : null;
  const localMissing = isLocal && installed && !installed.includes(value);

  let note: string;
  if (typing) {
    note = !otherId.trim()
      ? "Type an OpenRouter model id, e.g. mistralai/mistral-large-4-0."
      : listing
        ? `${listing.name}${
            listing.input_per_million != null && listing.output_per_million != null
              ? ` · $${listing.input_per_million.toFixed(2)} in / $${listing.output_per_million.toFixed(2)} out per million tokens`
              : ""
          }`
        : choices && choices.catalog.length > 0
          ? "⚠ Not in OpenRouter's model list. Check the id."
          : "Couldn't load OpenRouter's model list to check this id.";
  } else if (isLocal) {
    note = localMissing
      ? "⚠ Not installed in Ollama. Run: ollama pull " + value.slice("ollama:".length)
      : "Runs on this computer with Ollama: nothing leaves it.";
  } else {
    const choice = recommended.find((c) => c.id === value);
    note = choice
      ? `${choice.note}${choice.per_hour != null ? `. About ${dollars(choice.per_hour)} per hour of meeting.` : "."}`
      : "";
  }

  return (
    <div className="model-picker">
      <select value={selected} onChange={(e) => pick(e.target.value)} disabled={!choices}>
        {!choices && <option value={selected}>{value}</option>}
        {choices && (
          <>
            <optgroup label="Recommended">
              {recommended.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.label}
                  {c.per_hour != null ? ` · ${dollars(c.per_hour)}/hour` : ""}
                </option>
              ))}
            </optgroup>
            <optgroup label="On this computer (Ollama)">
              {installed === null ? (
                <option disabled value="">
                  Ollama isn't running
                </option>
              ) : installed.length === 0 ? (
                <option disabled value="">
                  No models installed
                </option>
              ) : (
                installed.map((id) => (
                  <option key={id} value={id}>
                    {id.slice("ollama:".length)}
                  </option>
                ))
              )}
              {localMissing && <option value={value}>{value.slice("ollama:".length)} (not installed)</option>}
            </optgroup>
            <option value={OTHER}>Other OpenRouter model…</option>
          </>
        )}
      </select>
      {typing && (
        <>
          <input
            list={listId}
            value={otherId}
            spellCheck={false}
            placeholder="provider/model"
            autoFocus={custom !== null && custom === ""}
            onChange={(e) => setCustom(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
            }}
          />
          <datalist id={listId}>
            {choices?.catalog.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </datalist>
        </>
      )}
      {note && <p className="model-picker-note">{note}</p>}
    </div>
  );
}
