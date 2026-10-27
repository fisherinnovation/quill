import { useEffect, useState } from "react";
import { type AssistantServer, api, errorText, events, type Settings } from "../api";
import { Banner, Row, Switch } from "./ui";

/**
 * Settings → Meetings: letting AI assistants such as Claude read the
 * library (quill_mcp). Turning it on asks first; then Quill serves them on
 * a localhost URL, shown here with the token and a command to connect
 * Claude Code, until it's turned off.
 */
export function AssistantRows({
  settings,
  update,
}: {
  settings: Settings;
  update: (patch: Partial<Settings>) => void;
}) {
  const [server, setServer] = useState<AssistantServer | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [showToken, setShowToken] = useState(false);
  const [confirmRenew, setConfirmRenew] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const [port, setPort] = useState(String(settings.assistant_port));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void api
      .assistantServer()
      .then(setServer)
      .catch((e) => setError(errorText(e)));
    const off = events.onAssistantServer(setServer);
    return () => void off.then((f) => f());
  }, []);

  const copy = async (what: string, text: string) => {
    await navigator.clipboard.writeText(text);
    setCopied(what);
    setTimeout(() => setCopied((c) => (c === what ? null : c)), 1500);
  };

  const renew = async () => {
    try {
      setServer(await api.renewAssistantToken());
    } catch (e) {
      setError(errorText(e));
    }
    setConfirmRenew(false);
  };

  /** Saves the port if it's one an app may use; otherwise puts the saved one back. */
  const commitPort = () => {
    const n = Number(port);
    if (Number.isInteger(n) && n >= 1024 && n <= 65535) {
      if (n !== settings.assistant_port) update({ assistant_port: n });
    } else {
      setPort(String(settings.assistant_port));
    }
  };

  const running = settings.assistant_access && server?.running && server.url && server.token;
  const command = running
    ? `claude mcp add --transport http --scope user quill ${server.url} --header "Authorization: Bearer ${server.token}"`
    : "";

  return (
    <>
      <Row
        label="Let AI assistants read your meetings"
        hint="An assistant such as Claude Code can search your meetings and read their notes, transcripts and action items, to answer what you ask it. Dictations are never shared."
      >
        <Switch
          label="Let AI assistants read your meetings"
          checked={settings.assistant_access || confirming}
          onChange={(on) => {
            if (on) {
              setConfirming(true);
            } else {
              setConfirming(false);
              update({ assistant_access: false });
            }
          }}
        />
      </Row>

      {confirming && !settings.assistant_access && (
        <Banner
          kind="warning"
          icon="lock"
          action={
            <div className="assistant-confirm">
              <button
                className="primary"
                onClick={() => {
                  setConfirming(false);
                  update({ assistant_access: true });
                }}
              >
                Turn on
              </button>
              <button onClick={() => setConfirming(false)}>Cancel</button>
            </div>
          }
        >
          <strong>Share your meetings with AI assistants?</strong> Assistants you connect will be able to read every
          meeting's notes, transcript and action items. What they read is sent to that assistant's provider (for
          Claude, Anthropic). While Quill is open it serves them at an address on this computer only, with a token they
          must have, until you turn this off.
        </Banner>
      )}

      {settings.assistant_access && server?.error && (
        <Row label="Not available" hint={server.error}>
          {null}
        </Row>
      )}

      {running && (
        <>
          <Row label="Address" hint="Where assistants connect. Only this computer can reach it.">
            <code className="assistant-value">{server.url}</code>
            <button onClick={() => void copy("url", server.url ?? "")}>{copied === "url" ? "Copied" : "Copy"}</button>
          </Row>
          <Row
            label="Token"
            hint="Assistants send it with every request. Anyone with it can read your meetings while this is on: a new token locks out everything set up with the old one."
          >
            {confirmRenew ? (
              <div className="confirm">
                Lock out connected assistants?
                <button className="danger" onClick={() => void renew()}>
                  New token
                </button>
                <button onClick={() => setConfirmRenew(false)}>Cancel</button>
              </div>
            ) : (
              <>
                <code className="assistant-value">{showToken ? server.token : "•".repeat(16)}</code>
                <button onClick={() => setShowToken((s) => !s)}>{showToken ? "Hide" : "Show"}</button>
                <button onClick={() => void copy("token", server.token ?? "")}>
                  {copied === "token" ? "Copied" : "Copy"}
                </button>
                <button onClick={() => setConfirmRenew(true)}>New token…</button>
              </>
            )}
          </Row>
          <Row
            label="Connect Claude Code"
            hint="Run this once in a terminal. It includes the token. Other assistants take the address and the token as an Authorization: Bearer header; see Help for Claude Desktop."
          >
            <button onClick={() => void copy("command", command)}>
              {copied === "command" ? "Copied" : "Copy command"}
            </button>
          </Row>
        </>
      )}

      {settings.assistant_access && (
        <Row label="Port" hint="Change it if another app uses this one. Assistants need the new address.">
          <input
            className="assistant-port"
            inputMode="numeric"
            aria-label="Port for AI assistants"
            value={port}
            onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
            onBlur={commitPort}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitPort();
            }}
          />
        </Row>
      )}

      {error && (
        <Row label="AI assistants" hint={error}>
          {null}
        </Row>
      )}
    </>
  );
}
