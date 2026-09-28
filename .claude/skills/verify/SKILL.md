---
name: verify
description: Build and run Doris (release binary with embedded frontend) and drive it through the browser and the gRPC-Web socket to verify a change at runtime.
---

# Verifying Doris at runtime

## Build and launch
```bash
make dist                                   # target/dist/doris, frontend embedded
D=$(mktemp -d)
DORIS_DATABASE=sqlite://$D/doris.db DORIS_LISTEN=127.0.0.1:38400 \
  DORIS_RP_ORIGIN=http://localhost:38400 target/dist/doris &
until curl -s -o /dev/null http://localhost:38400/; do :; done
```
- Use `localhost` in URLs (it is the WebAuthn RP origin; Secure cookies work there).
- Use your own port and a temp DB; stop with `kill -TERM <pid>` (graceful).
- Foreground `sleep` may be blocked: poll with `until curl …` instead.

## Drive the UI
Write a standalone Playwright script (not the e2e suite) and run it from
`e2e/` so it finds `node_modules` (`node e2e/<script>.mjs`; delete it after).
For each person, open a new browser context and give it a virtual passkey:
```js
const cdp = await context.newCDPSession(page);
await cdp.send("WebAuthn.enable");
await cdp.send("WebAuthn.addVirtualAuthenticator", { options: {
  protocol: "ctap2", transport: "internal", hasResidentKey: true,
  hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true } });
```
Select elements by their Swedish labels and roles: `E-post`, `Namn`, `Passkeyns namn`,
`Skapa konto med passkey`, `Logga in med passkey`, `Logga ut`, `Skapa inbjudan`,
`Inbjudningslänk`, `Lägg till passkey`; errors are `role="alert"`.
To switch a person to another device, remove the authenticator and add a new one.

Flows worth driving:
- The first visit redirects to `/register`, and that user becomes admin.
- Invite a member; they register via the link in a new context.
- Reuse the link, then open `/admin/invitations` as a member.
- Add a passkey, log out, and log in with it.
- Try an unknown email and a wrong device: both must read "Inloggningen misslyckades."

## Socket and database probes
- gRPC-Web by hand: `printf '\x00\x00\x00\x00\x00'` (an empty frame) with
  `content-type: application/grpc-web+proto`, `x-grpc-web: 1`, POST to
  `/doris.auth.v1.AuthService/<Rpc>`. Read `grpc-status`/`grpc-message` from the headers.
- Static files: check `content-encoding` (send `accept-encoding: br`), weak `etag` → 304,
  `cache-control`, CSP, `referrer-policy`.
- Database: `sqlite3 $D/doris.db "SELECT … FROM events"`. `UPDATE`/`DELETE` on
  `events` must fail with "events are append-only".

## Gotchas
- `make dist` fails if the wasm exceeds `WASM_BUDGET`, or if the tarball lacks
  `index.html` or the wasm.
- If a debug `trunk build` serves a stale wasm, `touch crates/web/src/main.rs`.
