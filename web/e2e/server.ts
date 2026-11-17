// Starts the real imessage-ap server for one test, on a free port, and keeps
// its log so a test can read the first-run setup code from it.
import { spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..", "..");
const bin = process.env.IMESSAGE_AP_BIN ?? path.join(root, "target", "debug", "imessage-ap");
const web = path.join(root, "web", "dist");

/** Demo data prepared by `scripts/demo.sh --prepare`. */
export const demoData = process.env.DEMO_DATA ?? path.join(root, "fixtures", "out", "demo-data");

export interface Server {
  url: string;
  log: () => string;
  stop: () => Promise<void>;
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const s = createServer();
    s.once("error", reject);
    s.listen(0, "127.0.0.1", () => {
      const address = s.address();
      const port = typeof address === "object" && address ? address.port : 0;
      s.close(() => resolve(port));
    });
  });
}

/** A new, empty data directory, removed when the server stops. */
export function emptyData(): string {
  return mkdtempSync(path.join(tmpdir(), "imessage-ap-e2e-"));
}

export async function startServer(opts: { data: string; demo?: boolean; cleanup?: boolean }): Promise<Server> {
  const port = await freePort();
  let log = "";
  const env: NodeJS.ProcessEnv = { ...process.env, RUST_LOG: "info", NO_COLOR: "1" };
  for (const k of ["IMESSAGE_AP_DB_DIR", "IMESSAGE_AP_BLOBS_DIR", "IMESSAGE_AP_DERIVED_DIR", "IMESSAGE_AP_EXPORTS_DIR"]) {
    delete env[k];
  }
  const args = ["serve", "--bind", "127.0.0.1", "--port", String(port), "--data", opts.data, "--web", web];
  if (opts.demo) args.push("--demo");
  const child: ChildProcess = spawn(bin, args, { env, stdio: ["ignore", "pipe", "pipe"] });
  child.stdout?.on("data", (d: Buffer) => (log += d.toString()));
  child.stderr?.on("data", (d: Buffer) => (log += d.toString()));
  const url = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + 20_000;
  for (;;) {
    if (child.exitCode !== null) throw new Error(`server exited:\n${log}`);
    try {
      const res = await fetch(`${url}/api/health`);
      if (res.ok) break;
    } catch {
      // Not listening yet.
    }
    if (Date.now() > deadline) throw new Error(`server did not start:\n${log}`);
    await new Promise((r) => setTimeout(r, 100));
  }
  return {
    url,
    log: () => log,
    stop: async () => {
      child.kill("SIGTERM");
      await new Promise((r) => child.once("exit", r));
      if (opts.cleanup) rmSync(opts.data, { recursive: true, force: true });
    },
  };
}

/** The setup code the server printed. */
export function setupCode(log: string): string {
  const m = /setup code: ([A-Z0-9]{4}-[A-Z0-9]{4})/.exec(log);
  if (!m?.[1]) throw new Error(`no setup code in the log:\n${log}`);
  return m[1];
}
