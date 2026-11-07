import { useEffect, useState } from "react";
import { fetchHealth, type Health } from "./api";

type State =
  | { kind: "loading" }
  | { kind: "ok"; health: Health }
  | { kind: "error"; message: string };

export default function App() {
  const [state, setState] = useState<State>({ kind: "loading" });

  useEffect(() => {
    const ctrl = new AbortController();
    fetchHealth(ctrl.signal)
      .then((health) => setState({ kind: "ok", health }))
      .catch((err: unknown) => {
        if (ctrl.signal.aborted) return;
        setState({ kind: "error", message: err instanceof Error ? err.message : String(err) });
      });
    return () => ctrl.abort();
  }, []);

  return (
    <main className="shell">
      <h1>imessage-ap</h1>
      <p className="muted">A private archive of your iMessage history.</p>
      <p className="status" role="status">
        {state.kind === "loading" && "Connecting…"}
        {state.kind === "ok" && `Server ${state.health.status} · v${state.health.version}`}
        {state.kind === "error" && `Server unreachable (${state.message})`}
      </p>
    </main>
  );
}
