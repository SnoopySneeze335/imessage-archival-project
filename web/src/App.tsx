import { useCallback, useEffect, useState } from "react";
import { ApiError, fetchStatus, type AuthStatus } from "./api";
import { Home } from "./Home";
import { SignIn, SetUp } from "./SignIn";

type State =
  | { kind: "loading" }
  | { kind: "ready"; status: AuthStatus }
  | { kind: "error"; message: string };

export default function App() {
  const [state, setState] = useState<State>({ kind: "loading" });

  const refresh = useCallback((signal?: AbortSignal) => {
    fetchStatus(signal)
      .then((status) => setState({ kind: "ready", status }))
      .catch((err: unknown) => {
        if (signal?.aborted) return;
        const message = err instanceof ApiError ? err.message : "The server could not be reached.";
        setState({ kind: "error", message });
      });
  }, []);

  useEffect(() => {
    const ctrl = new AbortController();
    refresh(ctrl.signal);
    return () => ctrl.abort();
  }, [refresh]);

  const onChange = useCallback(() => refresh(), [refresh]);

  if (state.kind === "loading") {
    return (
      <main className="auth">
        <p className="muted" role="status">
          Connecting…
        </p>
      </main>
    );
  }
  if (state.kind === "error") {
    return (
      <main className="auth">
        <p className="error" role="alert">
          {state.message}
        </p>
        <button type="button" onClick={() => refresh()}>
          Try again
        </button>
      </main>
    );
  }
  const { status } = state;
  switch (status.state) {
    case "setup":
      return <SetUp status={status} onDone={onChange} />;
    case "signed_out":
      return <SignIn status={status} onDone={onChange} />;
    case "signed_in":
      return <Home onSignedOut={onChange} />;
  }
}
