import { useEffect, useState } from "react";
import { api, events, fileSize, type SemanticStatus } from "../api";

/** Search by meaning's status, kept up to date. */
export function useSemanticStatus(): SemanticStatus | null {
  const [status, setStatus] = useState<SemanticStatus | null>(null);
  useEffect(() => {
    api
      .semanticStatus()
      .then(setStatus)
      .catch(() => setStatus(null));
    const unlisten = events.onSemanticIndex(setStatus);
    return () => void unlisten.then((f) => f());
  }, []);
  return status;
}

/** The search model's download size, as the catalog has it. */
const MODEL_BYTES = 470_641_600 + 17_082_730;

/**
 * A line under the Meetings search box about search by meaning: offers the
 * model while it isn't downloaded (it never downloads by itself), shows
 * download and indexing progress, and is otherwise silent. Nothing when
 * the setting is off.
 */
export function SemanticSearchNote({ status }: { status: SemanticStatus | null }) {
  const [download, setDownload] = useState<Record<string, [number, number]>>({});
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const unlisten = events.onModelDownload((p) => {
      if (!p.name.startsWith("search")) return;
      if (p.error) setError(p.error);
      setDownload((d) => ({ ...d, [p.name]: [p.finished ? p.total : p.done, p.total] }));
    });
    return () => void unlisten.then((f) => f());
  }, []);

  if (!status?.enabled) return null;
  const parts = Object.values(download);
  const downloading = !status.model_ready && parts.length > 0 && !error;
  if (!status.model_ready) {
    const done = parts.reduce((n, [d]) => n + d, 0);
    const percent = Math.round((done / MODEL_BYTES) * 100);
    return (
      <div className="search-note">
        {downloading ? (
          <>
            <span>Downloading the search model · {percent}%</span>
            <div className="progress">
              <div style={{ width: `${percent}%` }} />
            </div>
          </>
        ) : (
          <span>
            Find moments by what was meant, not only the exact words, with a model that runs on this computer.{" "}
            <button
              className="link-button"
              onClick={() => {
                setError(null);
                setDownload({});
                void api.downloadSearchModel();
              }}
            >
              Download it ({fileSize(MODEL_BYTES)})
            </button>
            {error && <span className="error-text"> Couldn't download it: {error}</span>}
          </span>
        )}
      </div>
    );
  }
  if (status.indexed < status.total) {
    return (
      <div className="search-note">
        <span>
          Preparing meetings for search by meaning · {status.indexed} of {status.total}
          {status.paused && " · waits while you record or dictate"}
        </span>
      </div>
    );
  }
  if (status.error) {
    return (
      <div className="search-note">
        <span className="error-text">Search by meaning: {status.error}</span>
      </div>
    );
  }
  return null;
}
