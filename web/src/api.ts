// The server's JSON API. Same origin only; the session cookie is HttpOnly,
// so scripts never see it.

export interface Health {
  status: string;
  version: string;
}

export type AuthState = "setup" | "signed_out" | "signed_in";

export interface AuthStatus {
  state: AuthState;
  /** The request reached the server over HTTPS (directly or via a trusted proxy). */
  https: boolean;
  /** The demo archive of invented data. */
  demo: boolean;
  min_password_chars: number;
}

export interface ArchiveSummary {
  present: boolean;
  messages: number;
  chats: number;
  attachments: number;
  backups: number;
}

/** An error the server explained; `message` is safe to show. */
export class ApiError extends Error {
  readonly status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    ...init,
    credentials: "same-origin",
    headers: init?.body ? { "Content-Type": "application/json" } : undefined,
  });
  if (!res.ok) {
    let message = `Request failed (${res.status})`;
    try {
      const body = (await res.json()) as { error?: unknown };
      if (typeof body.error === "string") message = body.error;
    } catch {
      // No JSON body: keep the generic message.
    }
    throw new ApiError(res.status, message);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

export function fetchHealth(signal?: AbortSignal): Promise<Health> {
  return request<Health>("/api/health", { signal });
}

export function fetchStatus(signal?: AbortSignal): Promise<AuthStatus> {
  return request<AuthStatus>("/api/auth/status", { signal });
}

export function setUp(code: string, password: string): Promise<void> {
  return request<void>("/api/auth/setup", {
    method: "POST",
    body: JSON.stringify({ code, password }),
  });
}

export function signIn(password: string): Promise<void> {
  return request<void>("/api/auth/login", {
    method: "POST",
    body: JSON.stringify({ password }),
  });
}

export function signOut(): Promise<void> {
  return request<void>("/api/auth/logout", { method: "POST" });
}

export function fetchArchive(signal?: AbortSignal): Promise<ArchiveSummary> {
  return request<ArchiveSummary>("/api/archive", { signal });
}
