import { useState, type FormEvent, type ReactNode } from "react";
import { ApiError, setUp, signIn, type AuthStatus } from "./api";

function errorText(err: unknown): string {
  return err instanceof ApiError ? err.message : "The server could not be reached.";
}

/** Small, non-blocking: plain HTTP works, it just is not encrypted. */
export function InsecureNotice() {
  if (window.isSecureContext) return null;
  return (
    <p className="notice" data-testid="insecure-notice">
      This page is not using HTTPS, so the network between you and the server can see what is sent. That is
      fine on your home network or tailnet. HTTPS through a reverse proxy is optional (see the README).
    </p>
  );
}

function DemoNotice({ status }: { status: AuthStatus }) {
  if (!status.demo) return null;
  return (
    <p className="notice demo" data-testid="demo-notice">
      Demo archive: every person and message here is invented. The password is <strong>demo1234</strong>.
    </p>
  );
}

function Card({ title, children }: { title: string; children: ReactNode }) {
  return (
    <main className="auth">
      <div className="card">
        <h1>{title}</h1>
        {children}
      </div>
    </main>
  );
}

export function SetUp({ status, onDone }: { status: AuthStatus; onDone: () => void }) {
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");
  const [again, setAgain] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (password !== again) {
      setError("The two passwords differ.");
      return;
    }
    if (password.length < status.min_password_chars) {
      setError(`Use at least ${status.min_password_chars} characters.`);
      return;
    }
    setBusy(true);
    setError(null);
    setUp(code, password)
      .then(onDone)
      .catch((err: unknown) => setError(errorText(err)))
      .finally(() => setBusy(false));
  };

  return (
    <Card title="Set a password">
      <p className="muted">
        First run. Enter the setup code from the server's log (Unraid: the container's log; Docker:{" "}
        <code>docker logs imessage-ap</code>), then choose the password for this archive.
      </p>
      <DemoNotice status={status} />
      <form onSubmit={submit}>
        <label>
          Setup code
          <input
            name="code"
            autoComplete="one-time-code"
            autoCapitalize="characters"
            spellCheck={false}
            value={code}
            onChange={(e) => setCode(e.target.value)}
            placeholder="XXXX-XXXX"
            required
          />
        </label>
        <label>
          Password
          <input
            name="password"
            type="password"
            autoComplete="new-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
        </label>
        <label>
          Password again
          <input
            name="password-again"
            type="password"
            autoComplete="new-password"
            value={again}
            onChange={(e) => setAgain(e.target.value)}
            required
          />
        </label>
        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}
        <button type="submit" disabled={busy}>
          {busy ? "Saving…" : "Set password"}
        </button>
      </form>
      <InsecureNotice />
    </Card>
  );
}

export function SignIn({ status, onDone }: { status: AuthStatus; onDone: () => void }) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    signIn(password)
      .then(onDone)
      .catch((err: unknown) => {
        setError(errorText(err));
        setPassword("");
      })
      .finally(() => setBusy(false));
  };

  return (
    <Card title="imessage-ap">
      <DemoNotice status={status} />
      <form onSubmit={submit}>
        <label>
          Password
          <input
            name="password"
            type="password"
            autoComplete="current-password"
            autoFocus
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
        </label>
        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}
        <button type="submit" disabled={busy}>
          {busy ? "Signing in…" : "Sign in"}
        </button>
      </form>
      <p className="muted small">
        Forgot it? Run <code>imessage-ap set-password</code> in the container.
      </p>
      <InsecureNotice />
    </Card>
  );
}
