import { useEffect, useState } from "react";
import { ApiError, fetchArchive, signOut, type ArchiveSummary } from "./api";
import { InsecureNotice } from "./SignIn";

const count = new Intl.NumberFormat("en-US");

/** Until the conversation list arrives (Milestone 4a-2): what the archive holds. */
export function Home({ onSignedOut }: { onSignedOut: () => void }) {
  const [summary, setSummary] = useState<ArchiveSummary | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const ctrl = new AbortController();
    fetchArchive(ctrl.signal)
      .then(setSummary)
      .catch((err: unknown) => {
        if (ctrl.signal.aborted) return;
        if (err instanceof ApiError && err.status === 401) {
          onSignedOut();
          return;
        }
        setError(err instanceof ApiError ? err.message : "The server could not be reached.");
      });
    return () => ctrl.abort();
  }, [onSignedOut]);

  const out = () => {
    signOut()
      .catch(() => undefined)
      .finally(onSignedOut);
  };

  return (
    <main className="home">
      <header>
        <h1>imessage-ap</h1>
        <button type="button" className="secondary" onClick={out}>
          Sign out
        </button>
      </header>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      {summary && !summary.present && (
        <p className="muted" data-testid="no-archive">
          No archive yet. Import a backup with <code>imessage-ap ingest</code>.
        </p>
      )}
      {summary?.present && (
        <dl className="counts" data-testid="archive-counts">
          <div>
            <dt>Messages</dt>
            <dd>{count.format(summary.messages)}</dd>
          </div>
          <div>
            <dt>Conversations</dt>
            <dd>{count.format(summary.chats)}</dd>
          </div>
          <div>
            <dt>Attachments</dt>
            <dd>{count.format(summary.attachments)}</dd>
          </div>
          <div>
            <dt>Backups</dt>
            <dd>{count.format(summary.backups)}</dd>
          </div>
        </dl>
      )}
      <p className="muted">The conversation list and conversation view come in the next update.</p>
      <InsecureNotice />
    </main>
  );
}
