# 0008. Access over plain HTTP or HTTPS, and sign-in

- Status: accepted
- Date: 2026-11-16
- Milestone: 4a-1, Viewer

## Context

The viewer is used from an iPhone (Safari), a Windows PC (Chrome) and a
laptop, over the LAN and over Tailscale. The spec asks for one password set
on first run, hashed with Argon2, and a session cookie. The user does not
want to depend on `tailscale serve` or any HTTPS setup for now, but other
users may prefer HTTPS behind a reverse proxy. Neither way may block the
other.

Browsers treat plain HTTP on a LAN address or a tailnet name as an insecure
context. A cookie marked `Secure` is dropped there, and APIs such as the
clipboard API, `crypto.subtle` and service workers are unavailable.

## Decision

**Both ways work, and neither is required.**

- The container publishes port 8080 on all interfaces by default:
  `http://<lan-ip>:8080`, `http://snoopyserver:8080` and the tailnet name
  all work. Publishing on loopback only (`-p 127.0.0.1:8080:8080`) is a
  documented option for people who put a proxy in front.
- HTTPS through a reverse proxy (`tailscale serve`, Nginx Proxy Manager, ...)
  is optional and documented. The proxy's address goes in
  `IMESSAGE_AP_TRUSTED_PROXIES` (addresses or CIDR ranges, comma-separated).
  `X-Forwarded-Proto`, `-For` and `-Host` are believed only from those
  addresses; from anyone else they are ignored.

**The cookie follows the scheme of the request.**

| Request | Cookie |
| --- | --- |
| HTTPS (through a trusted proxy that says so) | `__Host-session=…; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=90 days` |
| Plain HTTP | `session=…; Path=/; HttpOnly; SameSite=Strict; Max-Age=90 days` (no `Secure`, or the browser would drop it) |

- The token is 256 random bits. user.db stores only its SHA-256, so a copy
  of user.db cannot be used to sign in.
- A session lasts 90 days from its last use (extended at most once an
  hour). Setting a new password ends every session.
- Sign-in never refuses because the page is not a secure context. The page
  shows a small, non-blocking notice instead.

**Cross-site requests.** Every request that changes something (anything but
GET, HEAD and OPTIONS) must carry an `Origin` whose host and port equal the
request's own `Host` (or `X-Forwarded-Host` from a trusted proxy), after
removing the scheme's default port. So the LAN address, the server name and
the tailnet name all work without a list of allowed names. Without an
`Origin` header, `Sec-Fetch-Site: same-origin` is accepted. Together with
`SameSite=Strict` this stops other sites from acting with the session.

**First run.** With no password set, the server makes a one-time setup code
(eight characters such as `K7QM-3XRP`, about 40 bits) at startup and writes
it to its log. The setup page asks for that code and the new password, so
nobody else on the network can claim the archive first. The code is held in
memory only and is retired when the password is set; a restart before then
makes a new one.

**Passwords.** Argon2id with the `argon2` crate's defaults (OWASP's
recommended parameters), stored as a PHC string in user.db. At least 8
characters, at most 1,024 bytes. `imessage-ap set-password` sets or
replaces it from a terminal (hidden input, asked twice) or from standard
input with `--stdin`, and signs out every browser. It is the way back in
after a forgotten password: `docker exec -it imessage-ap imessage-ap
set-password`.

**Guessing is rate limited** per client address (the peer, or the last
`X-Forwarded-For` entry from a trusted proxy): five free failures, then a
lockout that doubles with each further failure, up to 15 minutes, answered
with 429 and `Retry-After`. Wrong setup codes count the same way. A success
clears the record.

**What needs a session.** Static files do not: the app's code holds no
data. `/api/health` (Docker's health check) and `/api/auth/*` do not.
Everything else under `/api`, including every future media URL, does.
API responses carry `Cache-Control: no-store`.

**Logs.** One line per API request: method, route pattern, status and
time. Never the query string or the body. Sign-in failures log the client
address only.

**Secure-context APIs.** Nothing before Milestone 9 may need a secure
context: no clipboard API, `crypto.subtle` or service workers. The PWA
install in Milestone 9 may require HTTPS; it is the one feature that does.

## Consequences

- Plain HTTP on the LAN or tailnet sends the password and messages
  unencrypted over that network. That is the user's choice for now; the
  notice says so, and HTTPS through a proxy is one setting away.
- Tailscale already encrypts traffic between devices on the tailnet, so
  plain HTTP over the tailnet is encrypted in transit anyway.
- A proxy that is not listed in `IMESSAGE_AP_TRUSTED_PROXIES` still works,
  but the server sees plain HTTP: the cookie is the non-`Secure` one and
  rate limiting counts every client as the proxy.
- Changing the cookie rules, the Origin check or what needs a session needs
  a new decision record.
