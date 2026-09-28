# Plan 4: Leptos Frontend, E2E and Distribution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The browser app. Users can:
- register the first admin with a passkey
- sign in and out
- invite members
- add passkeys

All of it is styled after shadcn preset `b1Gdz9bFY` and tested end to end in Chromium with a virtual authenticator. `make dist` produces one binary with the frontend embedded, plus the same frontend as a tarball.

**Architecture:**
- `crates/web` (`doris-web`) is a Leptos 0.8 CSR app built by Trunk 0.21 with Tailwind v4 (Trunk's standalone CLI, no Node).
  - `api.rs`: a `tonic-web-wasm-client` gRPC-Web client (cookies included; base URL from `<meta name="doris-api">` or the page origin).
  - `passkey.rs`: `navigator.credentials.create/get` via `webauthn-rs-proto`'s `wasm` conversions.
  - `ui.rs`: the shadcn "mira" components the app needs.
  - `app.rs`: the router, session state and a sign-in guard.
  - `pages/`: the five pages.
  - `errors.rs`: translates the API's stable error codes to Swedish.
- `e2e/` holds Playwright tests. Each test spawns its own `doris` server on a fresh database and a free port, and each page gets a Chrome DevTools virtual WebAuthn authenticator.
- The Makefile ties it together: `test`, `web`, `e2e`, `dev`, `dist`, `e2e-dist`.

**Tech Stack:**
- Leptos 0.8, leptos_router 0.8
- tonic-web-wasm-client 0.9
- webauthn-rs-proto 0.5 (`wasm`)
- web-sys / wasm-bindgen
- Trunk 0.21.14, Tailwind 4.3.3 (standalone, pinned in `Trunk.toml`)
- Inter variable font (self-hosted woff2, SIL OFL)
- Playwright 1.63 (dev only; Node is never needed at runtime)

**Spec:** `docs/superpowers/specs/2026-09-27-inloggning-webauthn-design.md`

**Plan series:** Plan 4 of 4. Plans 1–3 are done: event store, identity, WebAuthn, sessions, and the gRPC-Web server with embedded assets.

## Global Constraints
- TDD: behavior starts from a failing test that was run and seen to fail. For the UI the red tests are Playwright specs; pure logic gets Rust unit tests.
- **URLs, query parameters, identifiers and code are English.** Only visible UI text is Swedish. Routes:
  - `/register` (optionally `?invitation=<token>`)
  - `/login`
  - `/`
  - `/settings/passkeys`
  - `/admin/invitations`
- **Style:** shadcn preset `b1Gdz9bFY` (style mira, base stone, theme amber, font Inter, radius small, icons lucide). The tokens and component classes come from the preset's generated shadcn output, copied below. There are no runtime CDN or font requests: Inter is served from `/fonts`. Dark mode follows `prefers-color-scheme`.
- **API:** errors arrive as stable codes in the gRPC status message and the UI translates them. Every failed login reads "Inloggningen misslyckades.", whether or not the email exists.
- **Dev:** in dev and e2e the page origin must be `http://localhost:<port>` (not `127.0.0.1`), because it is the WebAuthn RP origin and Chromium's Secure-cookie exception applies only to localhost.
- **Self-contained:**
  - The server binary embeds `crates/web/dist`.
  - `make dist` also produces `doris-web-<version>.tar.gz` for CDN or nginx.
  - `crates/web/dist`, `e2e/node_modules`, `e2e/test-results` and `e2e/playwright-report` are never committed.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` cover `doris-web` too, because it also compiles for the host. Also lint the wasm build: `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`.
- **Tools:** `trunk` (0.21.14) and the `wasm32-unknown-unknown` target are installed, and so are Node and npm. `npx playwright install chromium` downloads the test browser.

---

### Task 1: Frontend skeleton, styles and the e2e harness

**Files:**
- Modify: `Cargo.toml` (add `crates/web` to `members`)
- Create: `crates/web/Cargo.toml`, `crates/web/Trunk.toml`, `crates/web/index.html`, `crates/web/style/input.css`, `crates/web/src/main.rs` (skeleton)
- Create: `crates/web/fonts/inter-latin-wght-normal.woff2`, `crates/web/fonts/inter-latin-ext-wght-normal.woff2`, `crates/web/fonts/OFL.txt`
- Create: `e2e/package.json`, `e2e/package-lock.json` (generated), `e2e/playwright.config.ts`, `e2e/tests/fixtures.ts`, `e2e/tests/assets.spec.ts`
- Modify: `Makefile` (targets `web`, `e2e`), `.gitignore`
- Modify: `crates/server/tests/web_dist.rs` (panic-safe cleanup, a residual from Plan 3)

**Interfaces:**
- Consumes (Plan 3):
  - The `doris` binary and its env config (`DORIS_DATABASE`, `DORIS_LISTEN`, `DORIS_RP_ID`, `DORIS_RP_ORIGIN`).
  - It serves `crates/web/dist` (debug builds read it at runtime; `crates/server/build.rs` rebuilds when it changes).
  - `_bg.wasm` is served as `application/wasm` with an immutable cache header, and HTML gets `no-cache` and `frame-ancestors 'none'`.
- Produces (Task 2 uses these):
  - The `doris-web` crate with its dependencies.
  - `style/input.css` with the preset tokens.
  - The e2e fixtures:
    - The `app` fixture is the base URL of a fresh server.
    - The auto `authenticator` fixture is the page's virtual authenticator id.
    - `newPerson()` opens a new context with its own authenticator.
    - `addAuthenticator(page)` and `removeAuthenticator(page, id)` add and remove authenticators.
    - `register(page, app, {email, name, passkey?, invitationLink?})` and `logIn(page, app, email)` drive the UI.
    - `DORIS_BIN` picks the binary.

- [ ] **Step 1: Make the `crates/server/tests/web_dist.rs` cleanup panic-safe**

Its old version left a stand-in `crates/web/dist/index.html` behind whenever its assertion failed. Replace the file with:

```rust
//! `WebDist` must resolve to the real `crates/web/dist` (relative to
//! `crates/server/Cargo.toml`), not a path literally containing
//! `$CARGO_MANIFEST_DIR` (rust-embed only expands that with the
//! `interpolate-folder-path` feature, which we don't enable).

use doris_server::assets::WebDist;
use std::fs;
use std::path::{Path, PathBuf};

/// Removes the stand-in build output again, even when an assertion fails.
struct Cleanup {
    index: PathBuf,
    dir: Option<PathBuf>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.index);
        if let Some(dir) = &self.dir {
            let _ = fs::remove_dir(dir);
        }
    }
}

#[test]
fn web_dist_embeds_the_real_frontend_build_dir() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/dist");
    let index = dist.join("index.html");
    // Without a frontend build, stand one in for the duration of the test.
    let _cleanup = (!index.exists()).then(|| {
        let created_dir = !dist.exists();
        fs::create_dir_all(&dist).unwrap();
        fs::write(&index, "<!doctype html><title>test</title>").unwrap();
        Cleanup {
            index: index.clone(),
            dir: created_dir.then(|| dist.clone()),
        }
    });

    assert!(WebDist::get("index.html").is_some());
}
```

Run: `cargo test -p doris-server --test web_dist`
Expected: PASS.

If `crates/web/dist/index.html` contains exactly `<!doctype html><title>test</title>`, it is debris from the old test. Delete that one file and the directory if it is then empty: `rm crates/web/dist/index.html && rmdir crates/web/dist`.

- [ ] **Step 2: Write the e2e harness and the failing asset tests**

`e2e/package.json`:

```json
{
  "name": "doris-e2e",
  "private": true,
  "scripts": {
    "test": "playwright test"
  },
  "devDependencies": {
    "@playwright/test": "1.63.0"
  }
}
```

`e2e/playwright.config.ts`:

```ts
import { defineConfig, devices } from "@playwright/test";

// Each test starts its own `doris` server on a fresh database (see fixtures.ts),
// so there is no global webServer here. Build first: `make e2e`.
export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  reporter: "list",
  use: { ...devices["Desktop Chrome"], trace: "retain-on-failure" },
});
```

`e2e/tests/fixtures.ts`:

```ts
import { test as base, expect, type BrowserContext, type CDPSession, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

// DORIS_BIN (relative to e2e/) selects another build, e.g. the release binary.
const binary = resolve(__dirname, "..", process.env.DORIS_BIN ?? "../target/debug/doris");

async function freePort(): Promise<number> {
  return new Promise((done) => {
    const server = createServer().listen(0, () => {
      const { port } = server.address() as { port: number };
      server.close(() => done(port));
    });
  });
}

async function waitUntilUp(url: string) {
  for (let i = 0; i < 100; i++) {
    try {
      await fetch(url);
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  throw new Error(`server at ${url} did not start`);
}

const devtools = new WeakMap<Page, CDPSession>();

async function cdpFor(page: Page): Promise<CDPSession> {
  let cdp = devtools.get(page);
  if (!cdp) {
    cdp = await page.context().newCDPSession(page);
    await cdp.send("WebAuthn.enable");
    devtools.set(page, cdp);
  }
  return cdp;
}

/** Gives the page a virtual passkey authenticator (Chrome DevTools WebAuthn),
 * like a laptop's or phone's built-in one. Returns its id. */
export async function addAuthenticator(page: Page): Promise<string> {
  const cdp = await cdpFor(page);
  const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
  return authenticatorId;
}

export async function removeAuthenticator(page: Page, authenticatorId: string) {
  await (await cdpFor(page)).send("WebAuthn.removeVirtualAuthenticator", { authenticatorId });
}

type Fixtures = {
  /** The id of the page's own virtual authenticator. */
  authenticator: string;
  /** Base URL of a fresh server with an empty database. */
  app: string;
  /** Opens a new browser context (a second person) with its own authenticator. */
  newPerson: () => Promise<Page>;
};

export const test = base.extend<Fixtures>({
  app: async ({}, use) => {
    const port = await freePort();
    const dir = mkdtempSync(join(tmpdir(), "doris-e2e-"));
    const origin = `http://localhost:${port}`;
    const server: ChildProcess = spawn(binary, [], {
      env: {
        ...process.env,
        DORIS_DATABASE: `sqlite://${join(dir, "doris.db")}`,
        DORIS_LISTEN: `127.0.0.1:${port}`,
        DORIS_RP_ID: "localhost",
        DORIS_RP_ORIGIN: origin,
      },
      stdio: "inherit",
    });
    await waitUntilUp(origin);
    await use(origin);
    server.kill();
    rmSync(dir, { recursive: true, force: true });
  },
  newPerson: async ({ browser }, use) => {
    const contexts: BrowserContext[] = [];
    await use(async () => {
      const context = await browser.newContext();
      contexts.push(context);
      const page = await context.newPage();
      await addAuthenticator(page);
      return page;
    });
    await Promise.all(contexts.map((c) => c.close()));
  },
  // Every test page gets an authenticator, whether or not it asks for its id.
  authenticator: [
    async ({ page }, use) => {
      await use(await addAuthenticator(page));
    },
    { auto: true },
  ],
});

export { expect };

/** Registers through the UI and waits until signed in. */
export async function register(page: Page, app: string, opts: { email: string; name: string; passkey?: string; invitationLink?: string }) {
  await page.goto(opts.invitationLink ?? `${app}/register`);
  if (!opts.invitationLink) await page.getByLabel("E-post").fill(opts.email);
  else await expect(page.getByLabel("E-post")).toHaveValue(opts.email);
  await page.getByLabel("Namn", { exact: true }).fill(opts.name);
  await page.getByLabel("Passkeyns namn").fill(opts.passkey ?? "Laptop");
  await page.getByRole("button", { name: "Skapa konto med passkey" }).click();
  await expect(page.getByText(`Inloggad som ${opts.name}`)).toBeVisible();
}

export async function logIn(page: Page, app: string, email: string) {
  await page.goto(`${app}/login`);
  await page.getByLabel("E-post").fill(email);
  await page.getByRole("button", { name: "Logga in med passkey" }).click();
}
```

`e2e/tests/assets.spec.ts`:

```ts
import { expect, test } from "./fixtures";

test("the app's wasm is served as wasm and cached forever", async ({ page, app }) => {
  const wasm = page.waitForResponse((r) => r.url().endsWith("_bg.wasm"));
  await page.goto(app);

  const response = await wasm;
  expect(response.status()).toBe(200);
  expect(response.headers()["content-type"]).toBe("application/wasm");
  expect(response.headers()["cache-control"]).toBe("public, max-age=31536000, immutable");
});

test("the page is revalidated and never framed", async ({ page, app }) => {
  const response = await page.goto(app);

  expect(response!.headers()["cache-control"]).toBe("no-cache");
  expect(response!.headers()["content-security-policy"]).toBe("frame-ancestors 'none'");
});
```

Install the tools. This creates `e2e/package-lock.json`, which gets committed:

```bash
cd e2e && npm install && npx playwright install chromium
```

Add to `.gitignore`:

```
/e2e/node_modules
/e2e/test-results
/e2e/playwright-report
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo build -p doris-server && cd e2e && npx playwright test tests/assets.spec.ts`
Expected: both tests FAIL.
- There is no frontend build yet, so `GET /` is `404`.
- The first test times out waiting for a `_bg.wasm` response.
- The second test fails on `cache-control`, because a 404 has no such header.

- [ ] **Step 4: Create the frontend skeleton**

In the root `Cargo.toml`, set:

```toml
members = ["crates/eventstore", "crates/identity", "crates/proto", "crates/server", "crates/web"]
```

`crates/web/Cargo.toml` (the full dependency list; Task 2 uses all of it):

```toml
[package]
name = "doris-web"
version.workspace = true
edition.workspace = true

[dependencies]
console_error_panic_hook = "0.1"
doris-proto.workspace = true
leptos = { version = "0.8", features = ["csr"] }
leptos_router = "0.8"
serde_json.workspace = true
tonic.workspace = true
tonic-web-wasm-client = "0.9"
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
webauthn-rs-proto = { workspace = true, features = ["wasm"] }
web-sys = { version = "0.3", features = ["CredentialsContainer", "CredentialCreationOptions", "CredentialRequestOptions", "PublicKeyCredential", "Document", "Element", "Location", "Navigator", "Window"] }
```

`crates/web/Trunk.toml`:

```toml
[build]
target = "index.html"
dist = "dist"

[tools]
tailwindcss = "4.3.3"

# `trunk serve` proxies API calls to the server, so dev is same-origin.
[[proxy]]
backend = "http://localhost:3000/doris.auth.v1.AuthService"
```

`crates/web/index.html`:

```html
<!doctype html>
<html lang="sv">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <!-- Base URL of the API when the frontend is served from another origin
         (CDN). Empty means same origin. -->
    <meta name="doris-api" content="" />
    <title>Doris</title>
    <link data-trunk rel="tailwind-css" href="style/input.css" />
    <link data-trunk rel="copy-dir" href="fonts" />
    <link data-trunk rel="rust" data-wasm-opt="z" />
  </head>
  <body></body>
</html>
```

Fonts (Inter variable from `@fontsource-variable/inter` 5.3.0, SIL Open Font License):

```bash
mkdir -p crates/web/fonts && cd /tmp && npm pack @fontsource-variable/inter@5.3.0 && tar -xzf fontsource-variable-inter-5.3.0.tgz
cp package/files/inter-latin-wght-normal.woff2 package/files/inter-latin-ext-wght-normal.woff2 "$OLDPWD/crates/web/fonts/"
cp package/LICENSE "$OLDPWD/crates/web/fonts/OFL.txt"
cd "$OLDPWD" && shasum -a 256 crates/web/fonts/*.woff2
```

Expected hashes:
- `3100e775e8616cd2611beecfa23a4263d7037586789b43f035236a2e6fbd4c62` for `inter-latin-wght-normal.woff2`
- `34b9c504cab7a73e37b746343a449132e56cf7b5481af2cb81dc74dcff25c956` for `inter-latin-ext-wght-normal.woff2`

`crates/web/style/input.css`. The tokens are copied from `npx shadcn init -t vite -b radix -p b1Gdz9bFY`; the sidebar and chart tokens are dropped, and dark mode is switched from a `.dark` class to `prefers-color-scheme`:

```css
/* Design tokens from shadcn preset b1Gdz9bFY: style mira, base stone,
   theme amber, font Inter (self-hosted), radius small, icons lucide. */
@import "tailwindcss";
@source "../src";

@font-face {
  font-family: "Inter Variable";
  font-style: normal;
  font-display: swap;
  font-weight: 100 900;
  src: url("/fonts/inter-latin-ext-wght-normal.woff2") format("woff2-variations");
  unicode-range: U+0100-02BA, U+02BD-02C5, U+02C7-02CC, U+02CE-02D7, U+02DD-02FF, U+0304, U+0308, U+0329, U+1D00-1DBF, U+1E00-1E9F, U+1EF2-1EFF, U+2020, U+20A0-20AB, U+20AD-20C0, U+2113, U+2C60-2C7F, U+A720-A7FF;
}

@font-face {
  font-family: "Inter Variable";
  font-style: normal;
  font-display: swap;
  font-weight: 100 900;
  src: url("/fonts/inter-latin-wght-normal.woff2") format("woff2-variations");
  unicode-range: U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0304, U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD;
}

@custom-variant dark (@media (prefers-color-scheme: dark));

@theme inline {
    --font-heading: var(--font-sans);
    --font-sans: 'Inter Variable', ui-sans-serif, system-ui, sans-serif;
    --color-ring: var(--ring);
    --color-input: var(--input);
    --color-border: var(--border);
    --color-destructive: var(--destructive);
    --color-accent-foreground: var(--accent-foreground);
    --color-accent: var(--accent);
    --color-muted-foreground: var(--muted-foreground);
    --color-muted: var(--muted);
    --color-secondary-foreground: var(--secondary-foreground);
    --color-secondary: var(--secondary);
    --color-primary-foreground: var(--primary-foreground);
    --color-primary: var(--primary);
    --color-popover-foreground: var(--popover-foreground);
    --color-popover: var(--popover);
    --color-card-foreground: var(--card-foreground);
    --color-card: var(--card);
    --color-foreground: var(--foreground);
    --color-background: var(--background);
    --radius-sm: calc(var(--radius) * 0.6);
    --radius-md: calc(var(--radius) * 0.8);
    --radius-lg: var(--radius);
    --radius-xl: calc(var(--radius) * 1.4);
    --radius-2xl: calc(var(--radius) * 1.8);
    --radius-3xl: calc(var(--radius) * 2.2);
    --radius-4xl: calc(var(--radius) * 2.6);
}

:root {
    --background: oklch(1 0 0);
    --foreground: oklch(0.147 0.004 49.25);
    --card: oklch(1 0 0);
    --card-foreground: oklch(0.147 0.004 49.25);
    --popover: oklch(1 0 0);
    --popover-foreground: oklch(0.147 0.004 49.25);
    --primary: oklch(0.555 0.163 48.998);
    --primary-foreground: oklch(0.987 0.022 95.277);
    --secondary: oklch(0.967 0.001 286.375);
    --secondary-foreground: oklch(0.21 0.006 285.885);
    --muted: oklch(0.97 0.001 106.424);
    --muted-foreground: oklch(0.553 0.013 58.071);
    --accent: oklch(0.97 0.001 106.424);
    --accent-foreground: oklch(0.216 0.006 56.043);
    --destructive: oklch(0.577 0.245 27.325);
    --border: oklch(0.923 0.003 48.717);
    --input: oklch(0.923 0.003 48.717);
    --ring: oklch(0.709 0.01 56.259);
    --radius: 0.45rem;
}

@media (prefers-color-scheme: dark) {
  :root {
  --background: oklch(0.147 0.004 49.25);
      --foreground: oklch(0.985 0.001 106.423);
      --card: oklch(0.216 0.006 56.043);
      --card-foreground: oklch(0.985 0.001 106.423);
      --popover: oklch(0.216 0.006 56.043);
      --popover-foreground: oklch(0.985 0.001 106.423);
      --primary: oklch(0.473 0.137 46.201);
      --primary-foreground: oklch(0.987 0.022 95.277);
      --secondary: oklch(0.274 0.006 286.033);
      --secondary-foreground: oklch(0.985 0 0);
      --muted: oklch(0.268 0.007 34.298);
      --muted-foreground: oklch(0.709 0.01 56.259);
      --accent: oklch(0.268 0.007 34.298);
      --accent-foreground: oklch(0.985 0.001 106.423);
      --destructive: oklch(0.704 0.191 22.216);
      --border: oklch(1 0 0 / 10%);
      --input: oklch(1 0 0 / 15%);
      --ring: oklch(0.553 0.013 58.071);
  }
}

@layer base {
  * {
    @apply border-border outline-ring/50;
  }
  body {
    @apply bg-background text-foreground;
  }
  html {
    @apply font-sans;
  }
}
```

`crates/web/src/main.rs` (skeleton; Task 2 replaces it):

```rust
//! Doris web app: Leptos CSR, talking to the server over gRPC-Web.

use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(|| view! { <main class="p-4 text-sm">"Doris"</main> });
}
```

In the `Makefile`, replace its contents with:

```make
.PHONY: test web e2e

# Unit and integration tests (Rust, all crates).
test:
	cargo test --workspace

# Debug build of the frontend into crates/web/dist.
web:
	cd crates/web && trunk build

# Browser tests against the debug server (frontend embedded from crates/web/dist).
e2e: web
	cargo build -p doris-server
	cd e2e && npm ci && npx playwright install chromium && npx playwright test
```

- [ ] **Step 5: Run to verify they pass**

Run: `make e2e`
Expected: the first build downloads Tailwind 4.3.3 and wasm-bindgen, then `2 passed`.

Also run:
- `cargo test --workspace`, which must still pass (the web crate has no tests yet)
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`

- [ ] **Step 6: Commit**

Check with `git status` that `crates/web/dist` and `e2e/node_modules` are not listed, then:

```bash
git add Cargo.toml Cargo.lock Makefile .gitignore crates/web crates/server/tests/web_dist.rs e2e
git commit -m "Add Leptos frontend skeleton with preset styles and Playwright harness"
```

---

### Task 2: The app — pages, passkeys and API client

**Files:**
- Replace: `crates/web/src/main.rs`
- Create: `crates/web/src/api.rs`, `crates/web/src/errors.rs`, `crates/web/src/passkey.rs`, `crates/web/src/ui.rs`, `crates/web/src/app.rs`
- Create: `crates/web/src/pages/{mod,register,login,home,passkeys,invitations}.rs`
- Test: `e2e/tests/auth.spec.ts`, and the unit test inside `crates/web/src/errors.rs`

**Interfaces:**
- Consumes:
  - From Task 1: the crate, the styles and the e2e fixtures.
  - From Plan 3: `doris_proto::auth::v1` (client, messages, `Role`).
  - The API's stable error codes: `invalid_email`, `invalid_display_name`, `invalid_passkey_name`, `invitation_required`, `invitation_expired`, `invitation_already_used`, `invitation_email_mismatch`, `invitation_not_found`, `already_exists`, `duplicate_passkey`, `ceremony_expired`, `login_failed`, `credential_rejected`, `not_signed_in`, `not_admin`, `internal`.
- Produces: the finished UI. Its labels and buttons are what the e2e tests (and later features) target:

  | Element | Swedish text |
  |---|---|
  | Labels | `E-post`, `Namn`, `Passkeyns namn`, `Inbjudningslänk` |
  | Buttons | `Skapa konto med passkey`, `Logga in med passkey`, `Logga ut`, `Lägg till passkey`, `Skapa inbjudan` |
  | Links | `Passkeys`, `Inbjudningar` |

- [ ] **Step 1: Write the failing e2e tests**

`e2e/tests/auth.spec.ts`:

```ts
import { addAuthenticator, expect, logIn, register, removeAuthenticator, test } from "./fixtures";

test("the first user registers with a passkey and becomes admin", async ({ page, app }) => {
  await page.goto(app);
  await expect(page).toHaveURL(`${app}/register`);
  await expect(page.getByRole("heading", { name: "Skapa administratörskonto" })).toBeVisible();

  await register(page, app, { email: "anna@example.se", name: "Anna" });

  await expect(page.getByText("administratör")).toBeVisible();
  await expect(page.getByRole("link", { name: "Inbjudningar" })).toBeVisible();
});

test("a user signs out and back in with the passkey", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });

  await page.getByRole("button", { name: "Logga ut" }).click();
  await expect(page).toHaveURL(`${app}/login`);
  await page.goto(app);
  await expect(page).toHaveURL(`${app}/login`);

  await logIn(page, app, "anna@example.se");
  await expect(page.getByText("Inloggad som Anna")).toBeVisible();
  await page.reload();
  await expect(page.getByText("Inloggad som Anna")).toBeVisible();
});

test("an unknown email fails like any other failed login", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await page.getByRole("button", { name: "Logga ut" }).click();

  await logIn(page, app, "nobody@example.se");

  await expect(page.getByRole("alert")).toHaveText("Inloggningen misslyckades.");
});

test("an admin invites a member who registers through the link", async ({ page, app, newPerson }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await page.getByRole("link", { name: "Inbjudningar" }).click();
  await page.getByLabel("E-post").fill("bo@example.se");
  await page.getByRole("button", { name: "Skapa inbjudan" }).click();
  const link = await page.getByLabel("Inbjudningslänk").inputValue();
  expect(link).toContain("/register?invitation=");

  const bo = await newPerson();
  await register(bo, app, { email: "bo@example.se", name: "Bo", passkey: "Telefon", invitationLink: link });

  await expect(bo.getByText("användare")).toBeVisible();
  await expect(bo.getByRole("link", { name: "Inbjudningar" })).toHaveCount(0);
  await bo.goto(`${app}/admin/invitations`);
  await expect(bo.getByRole("alert")).toHaveText("Du saknar behörighet.");
  await page.reload();
  await expect(page.getByText("Använd", { exact: true })).toBeVisible();
});

test("registration without an invitation is closed after the first user", async ({ page, app, newPerson }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });

  const stranger = await newPerson();
  await stranger.goto(`${app}/register`);

  await expect(stranger.getByText("Registrering kräver en inbjudan.")).toBeVisible();
  await expect(stranger.getByRole("button", { name: "Skapa konto med passkey" })).toHaveCount(0);
});

test("a user adds a second passkey and signs in with it", async ({ page, app, authenticator: laptop }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna", passkey: "Laptop" });
  await page.getByRole("link", { name: "Passkeys" }).click();
  await expect(page.getByText("Laptop")).toBeVisible();

  // Switch to another device: only the "phone" authenticator is present now.
  await removeAuthenticator(page, laptop);
  await addAuthenticator(page);
  await page.getByLabel("Passkeyns namn").fill("Telefon");
  await page.getByRole("button", { name: "Lägg till passkey" }).click();
  await expect(page.getByText("Telefon")).toBeVisible();

  await page.getByRole("button", { name: "Logga ut" }).click();
  await logIn(page, app, "anna@example.se");
  await expect(page.getByText("Inloggad som Anna")).toBeVisible();
});
```

Run: `make e2e`
Expected: the 6 auth tests FAIL (for example, waiting for `getByLabel('E-post')` times out, because the skeleton renders only "Doris"). The 2 asset tests still pass.

- [ ] **Step 2: Error translation, test first**

Replace `crates/web/src/main.rs` with the final entry point:

```rust
//! Doris web app: Leptos CSR, talking to the server over gRPC-Web.

mod api;
mod app;
mod errors;
mod pages;
mod passkey;
mod ui;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
```

Create `crates/web/src/errors.rs`. Write the test module first; the `message` function below it is the implementation:

```rust
//! Swedish messages for the API's stable error codes.

/// The text to show for a failed API call.
pub fn describe(status: &tonic::Status) -> String {
    message(status.message()).to_owned()
}

fn message(code: &str) -> &'static str {
    match code {
        "invalid_email" => "Ange en giltig e-postadress.",
        "invalid_display_name" => "Namnet måste vara 1–100 tecken.",
        "invalid_passkey_name" => "Passkeyns namn måste vara 1–64 tecken.",
        "invitation_required" => "Du behöver en inbjudan för att registrera dig.",
        "invitation_expired" => "Inbjudan har gått ut. Be om en ny.",
        "invitation_already_used" => "Inbjudan har redan använts.",
        "invitation_email_mismatch" => "E-postadressen matchar inte inbjudan.",
        "invitation_not_found" => "Inbjudan finns inte eller har redan använts.",
        "already_exists" => "E-postadressen är redan registrerad eller inbjuden.",
        "duplicate_passkey" => "Den här passkeyn är redan registrerad.",
        "ceremony_expired" => "Tiden gick ut. Försök igen.",
        "login_failed" => "Inloggningen misslyckades.",
        "credential_rejected" => "Passkeyn godkändes inte. Försök igen.",
        "not_signed_in" => "Du är inte inloggad.",
        "not_admin" => "Du saknar behörighet.",
        _ => "Något gick fel. Försök igen.",
    }
}

#[cfg(test)]
mod tests {
    use super::message;

    #[test]
    fn known_codes_have_their_own_message_and_unknown_ones_a_generic_one() {
        assert_eq!(message("login_failed"), "Inloggningen misslyckades.");
        assert_eq!(message("not_admin"), "Du saknar behörighet.");
        assert_eq!(message("internal"), "Något gick fel. Försök igen.");
        assert_eq!(message("something_new"), "Något gick fel. Försök igen.");
    }
}
```

To see it fail first, give `message` a body of `todo!()` and run `cargo test -p doris-web` → FAIL (panic: not yet implemented). Then restore the match above.

The crate won't build until the modules `main.rs` declares exist, so do Steps 2–4 before running anything that compiles the whole crate. Alternatively, comment out the other `mod` lines while doing this step.

- [ ] **Step 3: API client, passkey calls and UI components**

`crates/web/src/api.rs`:

```rust
//! gRPC-Web client for `doris.auth.v1.AuthService`.

use doris_proto::auth::v1::auth_service_client::AuthServiceClient;
use leptos::prelude::window;
use tonic_web_wasm_client::Client;
use tonic_web_wasm_client::options::{Credentials, FetchOptions};

pub use doris_proto::auth::v1 as pb;

pub type Api = AuthServiceClient<Client>;

/// A client for the API. Cookies are always sent, so the session also works
/// when the frontend is served from another origin (CDN) on the same site.
pub fn api() -> Api {
    let options = FetchOptions::new().credentials(Credentials::Include);
    AuthServiceClient::new(Client::new_with_options(base_url(), options))
}

/// `<meta name="doris-api" content="…">` when set, otherwise this page's origin.
fn base_url() -> String {
    window()
        .document()
        .and_then(|doc| doc.query_selector("meta[name=doris-api]").ok().flatten())
        .and_then(|meta| meta.get_attribute("content"))
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| window().location().origin().expect("page has an origin"))
}
```

`crates/web/src/passkey.rs`:

```rust
//! The browser side of WebAuthn: `navigator.credentials.create/get`, with
//! options and results as the webauthn-rs JSON the API speaks.

use leptos::prelude::window;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use webauthn_rs_proto::{
    CreationChallengeResponse, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse,
};

/// Shown when the user cancels or the authenticator fails. The browser does
/// not tell which, on purpose.
pub const FAILED: &str = "Passkey-åtgärden avbröts eller misslyckades.";

/// Creates a passkey; returns the credential as JSON for `Finish…`.
pub async fn create(options_json: &str) -> Result<String, String> {
    let options: CreationChallengeResponse =
        serde_json::from_str(options_json).map_err(|_| FAILED.to_owned())?;
    let promise = window()
        .navigator()
        .credentials()
        .create_with_options(&options.into())
        .map_err(|_| FAILED.to_owned())?;
    let credential: web_sys::PublicKeyCredential = JsFuture::from(promise)
        .await
        .map_err(|_| FAILED.to_owned())?
        .unchecked_into();
    serde_json::to_string(&RegisterPublicKeyCredential::from(credential))
        .map_err(|_| FAILED.to_owned())
}

/// Signs a login challenge; returns the assertion as JSON for `FinishLogin`.
pub async fn get(options_json: &str) -> Result<String, String> {
    let options: RequestChallengeResponse =
        serde_json::from_str(options_json).map_err(|_| FAILED.to_owned())?;
    let promise = window()
        .navigator()
        .credentials()
        .get_with_options(&options.into())
        .map_err(|_| FAILED.to_owned())?;
    let credential: web_sys::PublicKeyCredential = JsFuture::from(promise)
        .await
        .map_err(|_| FAILED.to_owned())?
        .unchecked_into();
    serde_json::to_string(&PublicKeyCredential::from(credential)).map_err(|_| FAILED.to_owned())
}
```

`crates/web/src/ui.rs` (class lists copied from the preset's generated shadcn components):

```rust
//! Components in the style of shadcn preset b1Gdz9bFY (radix-mira). Class
//! lists are copied from the generated shadcn components; only what the app
//! uses is here.

use leptos::prelude::*;

const BUTTON: &str = "inline-flex shrink-0 items-center justify-center gap-1 rounded-md border border-transparent bg-clip-padding text-xs/relaxed font-medium whitespace-nowrap transition-all outline-none select-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30 active:translate-y-px disabled:pointer-events-none disabled:opacity-50 h-7 px-2 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg]:size-3.5";
const BUTTON_DEFAULT: &str = "bg-primary text-primary-foreground hover:bg-primary/80";
const BUTTON_GHOST: &str = "hover:bg-muted hover:text-foreground dark:hover:bg-muted/50";
const INPUT: &str = "h-7 w-full min-w-0 rounded-md border border-input bg-input/20 px-2 py-0.5 text-sm transition-colors outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30 disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 read-only:bg-muted read-only:text-muted-foreground md:text-xs/relaxed dark:bg-input/30";
const LABEL: &str = "flex items-center gap-2 text-xs/relaxed leading-none font-medium select-none";
const CARD: &str = "flex flex-col gap-4 overflow-hidden rounded-lg bg-card py-4 text-xs/relaxed text-card-foreground ring-1 ring-foreground/10";
const ALERT: &str =
    "relative grid w-full gap-0.5 rounded-lg border px-2 py-1.5 text-left text-xs/relaxed";

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Variant {
    #[default]
    Default,
    Ghost,
}

#[component]
pub fn Button(
    #[prop(optional)] variant: Variant,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(default = "submit")] kind: &'static str,
    children: Children,
) -> impl IntoView {
    let look = match variant {
        Variant::Default => BUTTON_DEFAULT,
        Variant::Ghost => BUTTON_GHOST,
    };
    view! {
        <button type=kind class=format!("{BUTTON} {look}") disabled=disabled>
            {children()}
        </button>
    }
}

/// A labelled text input bound to `value`.
#[component]
pub fn Field(
    label: &'static str,
    id: &'static str,
    value: RwSignal<String>,
    #[prop(default = "text")] kind: &'static str,
    #[prop(optional)] autocomplete: &'static str,
    #[prop(optional)] placeholder: &'static str,
    #[prop(optional, into)] readonly: Signal<bool>,
) -> impl IntoView {
    view! {
        <div class="grid gap-1.5">
            <label for=id class=LABEL>
                {label}
            </label>
            <input
                id=id
                name=id
                type=kind
                class=INPUT
                required
                autocomplete=autocomplete
                placeholder=placeholder
                readonly=readonly
                bind:value=value
            />
        </div>
    }
}

#[component]
pub fn Card(
    title: &'static str,
    #[prop(optional)] description: &'static str,
    children: Children,
) -> impl IntoView {
    view! {
        <section class=CARD>
            <header class="grid gap-1 px-4">
                <h1 class="text-sm font-medium">{title}</h1>
                {(!description.is_empty())
                    .then(|| view! { <p class="text-xs/relaxed text-muted-foreground">{description}</p> })}
            </header>
            <div class="px-4">{children()}</div>
        </section>
    }
}

/// An error message, shown while `message` is `Some`.
#[component]
pub fn ErrorAlert(message: RwSignal<Option<String>>) -> impl IntoView {
    move || {
        message.get().map(|text| {
            view! {
                <div role="alert" class=format!("{ALERT} bg-card text-destructive")>
                    {text}
                </div>
            }
        })
    }
}
```

- [ ] **Step 4: App shell and pages**

`crates/web/src/app.rs`:

```rust
//! Routes, the session state and the page shell.

use crate::api::{api, pb};
use crate::pages::{Home, Invitations, Login, Passkeys, Register};
use crate::ui::{Button, Variant};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::{A, Redirect, Route, Router, Routes};
use leptos_router::hooks::use_navigate;
use leptos_router::path;

/// Who is signed in, as last reported by the server.
#[derive(Clone, Copy)]
pub struct Session {
    pub loaded: RwSignal<bool>,
    pub bootstrap_required: RwSignal<bool>,
    pub user: RwSignal<Option<pb::User>>,
}

impl Session {
    pub fn signed_in(&self, user: pb::User) {
        self.bootstrap_required.set(false);
        self.user.set(Some(user));
    }

    pub fn is_admin(&self) -> bool {
        self.user
            .get()
            .is_some_and(|user| user.role() == pb::Role::Admin)
    }
}

#[component]
pub fn App() -> impl IntoView {
    let session = Session {
        loaded: RwSignal::new(false),
        bootstrap_required: RwSignal::new(false),
        user: RwSignal::new(None),
    };
    provide_context(session);
    spawn_local(async move {
        if let Ok(status) = api().get_status(pb::GetStatusRequest {}).await {
            let status = status.into_inner();
            session.bootstrap_required.set(status.bootstrap_required);
            session.user.set(status.current_user);
        }
        session.loaded.set(true);
    });

    view! {
        <Router>
            <Header />
            <main class="mx-auto w-full max-w-md px-4 py-10">
                <Show when=move || session.loaded.get() fallback=|| view! { <p class="text-muted-foreground">"Laddar…"</p> }>
                    <Routes fallback=|| view! { <p>"Sidan finns inte."</p> }>
                        <Route path=path!("/register") view=Register />
                        <Route path=path!("/login") view=Login />
                        <Route path=path!("/") view=|| view! { <SignedIn><Home /></SignedIn> } />
                        <Route path=path!("/settings/passkeys") view=|| view! { <SignedIn><Passkeys /></SignedIn> } />
                        <Route path=path!("/admin/invitations") view=|| view! { <SignedIn admin=true><Invitations /></SignedIn> } />
                    </Routes>
                </Show>
            </main>
        </Router>
    }
}

/// Renders `children` only for a signed-in user (an admin, if `admin`);
/// everyone else is sent to registration (first run) or login.
#[component]
fn SignedIn(#[prop(optional)] admin: bool, children: ChildrenFn) -> impl IntoView {
    let session = expect_context::<Session>();
    move || match session.user.get() {
        None if session.bootstrap_required.get() => {
            view! { <Redirect path="/register" /> }.into_any()
        }
        None => view! { <Redirect path="/login" /> }.into_any(),
        Some(_) if admin && !session.is_admin() => {
            view! { <p role="alert" class="text-destructive">"Du saknar behörighet."</p> }
                .into_any()
        }
        Some(_) => children().into_any(),
    }
}

#[component]
fn Header() -> impl IntoView {
    let session = expect_context::<Session>();
    let navigate = use_navigate();
    let log_out = move |_| {
        let navigate = navigate.clone();
        spawn_local(async move {
            let _ = api().logout(pb::LogoutRequest {}).await;
            session.user.set(None);
            navigate("/login", Default::default());
        });
    };
    view! {
        <header class="border-b">
            <nav class="mx-auto flex h-12 max-w-3xl items-center gap-4 px-4 text-xs/relaxed">
                <A href="/" attr:class="text-sm font-semibold">"Doris"</A>
                <Show when=move || session.user.get().is_some()>
                    <A href="/settings/passkeys" attr:class="text-muted-foreground hover:text-foreground">"Passkeys"</A>
                    <Show when=move || session.is_admin()>
                        <A href="/admin/invitations" attr:class="text-muted-foreground hover:text-foreground">"Inbjudningar"</A>
                    </Show>
                    <span class="ml-auto" />
                    <Button variant=Variant::Ghost kind="button" on:click=log_out.clone()>"Logga ut"</Button>
                </Show>
            </nav>
        </header>
    }
}
```

`crates/web/src/pages/mod.rs`:

```rust
mod home;
mod invitations;
mod login;
mod passkeys;
mod register;

pub use home::Home;
pub use invitations::Invitations;
pub use login::Login;
pub use passkeys::Passkeys;
pub use register::Register;
```

`crates/web/src/pages/register.rs`:

```rust
use crate::api::{api, pb};
use crate::app::Session;
use crate::errors::describe;
use crate::passkey;
use crate::ui::{Button, Card, ErrorAlert, Field};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::{A, Redirect};
use leptos_router::hooks::use_query_map;

#[component]
pub fn Register() -> impl IntoView {
    let session = expect_context::<Session>();
    let query = use_query_map();
    let invitation = move || query.read().get("invitation");
    let email = RwSignal::new(String::new());
    let display_name = RwSignal::new(String::new());
    let passkey_name = RwSignal::new(String::new());
    let email_locked = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    // An invitation decides the email; show it and lock the field.
    Effect::new(move |_| {
        if let Some(token) = invitation() {
            spawn_local(async move {
                match api()
                    .get_invitation(pb::GetInvitationRequest { token })
                    .await
                {
                    Ok(found) => {
                        email.set(found.into_inner().email);
                        email_locked.set(true);
                    }
                    Err(status) => error.set(Some(describe(&status))),
                }
            });
        }
    });

    let done = RwSignal::new(false);
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let request = pb::BeginRegistrationRequest {
                email: email.get_untracked(),
                display_name: display_name.get_untracked(),
                invitation_token: invitation(),
                passkey_name: passkey_name.get_untracked(),
            };
            match register(request).await {
                Ok(user) => {
                    session.signed_in(user);
                    done.set(true);
                }
                Err(message) => error.set(Some(message)),
            }
            busy.set(false);
        });
    };

    let open = move || session.bootstrap_required.get() || invitation().is_some();
    let title = if session.bootstrap_required.get_untracked() {
        "Skapa administratörskonto"
    } else {
        "Registrera dig"
    };
    view! {
        {move || done.get().then(|| view! { <Redirect path="/" /> })}
        <Card title=title description="Du loggar in med en passkey – inget lösenord behövs.">
            <Show
                when=open
                fallback=|| {
                    view! {
                        <p class="text-muted-foreground">
                            "Registrering kräver en inbjudan. " <A href="/login" attr:class="underline">"Logga in"</A>
                        </p>
                    }
                }
            >
                <form class="grid gap-3" on:submit=submit>
                    <Field label="E-post" id="email" kind="email" autocomplete="username" value=email readonly=email_locked />
                    <Field label="Namn" id="display_name" autocomplete="name" value=display_name />
                    <Field label="Passkeyns namn" id="passkey_name" placeholder="t.ex. MacBook" value=passkey_name />
                    <ErrorAlert message=error />
                    <Button disabled=busy>"Skapa konto med passkey"</Button>
                </form>
            </Show>
        </Card>
    }
}

async fn register(request: pb::BeginRegistrationRequest) -> Result<pb::User, String> {
    let mut api = api();
    let invitation_token = request.invitation_token.clone();
    let begin = api
        .begin_registration(request)
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    let credential_json = passkey::create(&begin.options_json).await?;
    let user = api
        .finish_registration(pb::FinishRegistrationRequest {
            ceremony_id: begin.ceremony_id,
            invitation_token,
            credential_json,
        })
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    Ok(user)
}
```

`crates/web/src/pages/login.rs`:

```rust
use crate::api::{api, pb};
use crate::app::Session;
use crate::errors::describe;
use crate::passkey;
use crate::ui::{Button, Card, ErrorAlert, Field};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::{A, Redirect};

#[component]
pub fn Login() -> impl IntoView {
    let session = expect_context::<Session>();
    let email = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let done = RwSignal::new(false);

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match log_in(email.get_untracked()).await {
                Ok(user) => {
                    session.signed_in(user);
                    done.set(true);
                }
                Err(message) => error.set(Some(message)),
            }
            busy.set(false);
        });
    };

    view! {
        {move || done.get().then(|| view! { <Redirect path="/" /> })}
        <Show when=move || !session.bootstrap_required.get() fallback=|| view! { <Redirect path="/register" /> }>
            <Card title="Logga in" description="Använd din passkey.">
                <form class="grid gap-3" on:submit=submit>
                    <Field label="E-post" id="email" kind="email" autocomplete="username" value=email />
                    <ErrorAlert message=error />
                    <Button disabled=busy>"Logga in med passkey"</Button>
                </form>
                <p class="mt-4 text-muted-foreground">
                    "Har du en inbjudan? Öppna länken du fått. "
                    <A href="/register" attr:class="underline">"Registrera"</A>
                </p>
            </Card>
        </Show>
    }
}

async fn log_in(email: String) -> Result<pb::User, String> {
    let mut api = api();
    let begin = api
        .begin_login(pb::BeginLoginRequest { email })
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    // Every login failure reads the same, whether or not the email exists.
    let credential_json = passkey::get(&begin.options_json)
        .await
        .map_err(|_| "Inloggningen misslyckades.".to_owned())?;
    let user = api
        .finish_login(pb::FinishLoginRequest {
            ceremony_id: begin.ceremony_id,
            credential_json,
        })
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    Ok(user)
}
```

`crates/web/src/pages/home.rs`:

```rust
use crate::api::pb;
use crate::app::Session;
use crate::ui::Card;
use leptos::prelude::*;

#[component]
pub fn Home() -> impl IntoView {
    let session = expect_context::<Session>();
    let user = move || session.user.get().unwrap_or_default();
    let role = move || match user().role() {
        pb::Role::Admin => "administratör",
        _ => "användare",
    };
    view! {
        <Card title="Välkommen">
            <p>
                "Inloggad som " <strong>{move || user().display_name}</strong> " (" {move || user().email} "), " {role} "."
            </p>
        </Card>
    }
}
```

`crates/web/src/pages/passkeys.rs`:

```rust
use crate::api::{api, pb};
use crate::errors::describe;
use crate::passkey;
use crate::ui::{Button, Card, ErrorAlert, Field};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn Passkeys() -> impl IntoView {
    let passkeys = RwSignal::new(Vec::<pb::Passkey>::new());
    let name = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let refresh = move || {
        spawn_local(async move {
            match api().list_passkeys(pb::ListPasskeysRequest {}).await {
                Ok(list) => passkeys.set(list.into_inner().passkeys),
                Err(status) => error.set(Some(describe(&status))),
            }
        })
    };
    refresh();

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match add_passkey(name.get_untracked()).await {
                Ok(()) => {
                    name.set(String::new());
                    refresh();
                }
                Err(message) => error.set(Some(message)),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <Card title="Dina passkeys" description="Lägg till fler enheter så att du inte blir utelåst.">
                <ul class="grid gap-2">
                    <For each=move || passkeys.get() key=|p| p.credential_id.clone() let(passkey)>
                        <li class="flex justify-between gap-2">
                            <span class="font-medium">{passkey.name}</span>
                            <span class="text-muted-foreground">
                                {passkey
                                    .last_used_at
                                    .map(|at| format!("Senast använd {}", &at[..10]))
                                    .unwrap_or_else(|| "Aldrig använd".to_owned())}
                            </span>
                        </li>
                    </For>
                </ul>
            </Card>
            <Card title="Lägg till passkey">
                <form class="grid gap-3" on:submit=submit>
                    <Field label="Passkeyns namn" id="passkey_name" placeholder="t.ex. iPhone" value=name />
                    <ErrorAlert message=error />
                    <Button disabled=busy>"Lägg till passkey"</Button>
                </form>
            </Card>
        </div>
    }
}

async fn add_passkey(passkey_name: String) -> Result<(), String> {
    let mut api = api();
    let begin = api
        .begin_add_passkey(pb::BeginAddPasskeyRequest { passkey_name })
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    let credential_json = passkey::create(&begin.options_json).await?;
    api.finish_add_passkey(pb::FinishAddPasskeyRequest {
        ceremony_id: begin.ceremony_id,
        credential_json,
    })
    .await
    .map_err(|s| describe(&s))?;
    Ok(())
}
```

`crates/web/src/pages/invitations.rs`:

```rust
use crate::api::{api, pb};
use crate::errors::describe;
use crate::ui::{Button, Card, ErrorAlert, Field};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn Invitations() -> impl IntoView {
    let invitations = RwSignal::new(Vec::<pb::Invitation>::new());
    let email = RwSignal::new(String::new());
    let link = RwSignal::new(None::<String>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let refresh = move || {
        spawn_local(async move {
            match api().list_invitations(pb::ListInvitationsRequest {}).await {
                Ok(list) => invitations.set(list.into_inner().invitations),
                Err(status) => error.set(Some(describe(&status))),
            }
        })
    };
    refresh();

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        link.set(None);
        spawn_local(async move {
            let request = pb::CreateInvitationRequest {
                email: email.get_untracked(),
            };
            match api().create_invitation(request).await {
                Ok(created) => {
                    let origin = window().location().origin().unwrap_or_default();
                    let token = created.into_inner().token;
                    link.set(Some(format!("{origin}/register?invitation={token}")));
                    email.set(String::new());
                    refresh();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <Card title="Bjud in" description="Länken gäller i 7 dagar och kan användas en gång.">
                <form class="grid gap-3" on:submit=submit>
                    <Field label="E-post" id="email" kind="email" value=email />
                    <ErrorAlert message=error />
                    <Button disabled=busy>"Skapa inbjudan"</Button>
                </form>
                {move || {
                    link.get().map(|url| {
                        view! {
                            <div class="mt-4 grid gap-1.5">
                                <label for="invitation_link" class="text-xs/relaxed font-medium">"Inbjudningslänk"</label>
                                <input id="invitation_link" readonly value=url class="h-7 w-full rounded-md border border-input bg-muted px-2 text-xs/relaxed" />
                            </div>
                        }
                    })
                }}
            </Card>
            <Card title="Inbjudningar">
                <ul class="grid gap-2">
                    <For each=move || invitations.get() key=|i| i.id.clone() let(invitation)>
                        <li class="flex justify-between gap-2">
                            <span>{invitation.email}</span>
                            <span class="text-muted-foreground">
                                {if invitation.accepted {
                                    "Använd".to_owned()
                                } else {
                                    format!("Giltig till {}", &invitation.expires_at[..10])
                                }}
                            </span>
                        </li>
                    </For>
                </ul>
            </Card>
        </div>
    }
}
```

- [ ] **Step 5: Run everything to verify it passes**

Run:
- `cargo test --workspace`: all pass, including `doris-web`'s 1 unit test
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
- `make e2e`: `8 passed`

If the e2e page still shows the skeleton's plain "Doris", cargo reused an old wasm build. Run `touch crates/web/src/main.rs` and `make e2e` again.

- [ ] **Step 6: Commit**

```bash
git add crates/web e2e/tests/auth.spec.ts
git commit -m "Add web app: passkey registration, login, invitations and passkey management"
```

---

### Task 3: Distribution and developer workflow

**Files:**
- Modify: `Cargo.toml` (`[profile.release.build-override]`)
- Modify: `Makefile` (targets `dev`, `dist`, `e2e-dist`)
- Modify: `AGENTS.md` (Frontend section; Commands)

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `make dist` → `target/dist/doris` (release, frontend embedded) and `target/dist/doris-web-<version>.tar.gz`
  - `make e2e-dist` → the e2e suite against that binary
  - `make dev` → server on :3000 plus `trunk serve` on :8080

- [ ] **Step 1: Add the Makefile targets and see `make dist` fail**

Replace `Makefile` with:

```make
VERSION := $(shell cargo pkgid -p doris-server | sed 's/.*@//')
DIST := target/dist

.PHONY: test web e2e e2e-dist dev dist

# Unit and integration tests (Rust, all crates).
test:
	cargo test --workspace

# Debug build of the frontend into crates/web/dist.
web:
	cd crates/web && trunk build

# Browser tests against the debug server (frontend embedded from crates/web/dist).
e2e: web
	cargo build -p doris-server
	cd e2e && npm ci && npx playwright install chromium && npx playwright test

# The same browser tests against the release binary from `make dist`.
e2e-dist: dist
	cd e2e && npm ci && npx playwright install chromium && DORIS_BIN=../$(DIST)/doris npx playwright test

# Server on :3000 and `trunk serve` on :8080 (proxying the API). Open
# http://localhost:8080 — WebAuthn needs the page's origin as RP origin.
dev:
	@trap 'kill 0' EXIT; \
	DORIS_RP_ORIGIN=http://localhost:8080 cargo run -p doris-server & \
	cd crates/web && trunk serve --port 8080

# Release binary with the frontend embedded, plus the same frontend as a
# tarball for serving from a CDN or nginx.
dist:
	cd crates/web && trunk build --release
	cargo build --release -p doris-server
	mkdir -p $(DIST)
	cp target/release/doris $(DIST)/doris
	tar -czf $(DIST)/doris-web-$(VERSION).tar.gz -C crates/web/dist .
	@echo "built $(DIST)/doris and $(DIST)/doris-web-$(VERSION).tar.gz"
```

Run: `make dist`
Expected on macOS 27 (Darwin 27): FAIL. `cargo build --release` stops with `could not compile sqlx … dlopen(…libsqlx_macros-….dylib) … (mis-aligned LINKEDIT string pool …)`. The cause: release builds strip debuginfo by default, and the stripped proc-macro dylibs don't load on macOS 27. (On other platforms this step may already pass; record what you see.)

- [ ] **Step 2: Don't strip build-time artifacts**

Append to the root `Cargo.toml`:

```toml

# Release builds strip debuginfo by default. On macOS 27 the stripped
# proc-macro dylibs (e.g. sqlx-macros) fail to load ("mis-aligned LINKEDIT
# string pool"), so don't strip what only runs at build time.
[profile.release.build-override]
strip = "none"
```

Run: `make dist`
Expected: `built target/dist/doris and target/dist/doris-web-0.1.0.tar.gz`. The binary is about 12 MB and the tarball about 0.5 MB.

Check with `tar -tzf target/dist/doris-web-0.1.0.tar.gz`, which lists these files:
- `index.html`
- `doris-web-<hash>.js`
- `doris-web-<hash>_bg.wasm`
- `input-<hash>.css`
- `fonts/`

- [ ] **Step 3: Run the e2e suite against the release binary**

Run: `make e2e-dist`
Expected: `8 passed`. This proves the release binary embeds and serves the frontend with the right headers.

- [ ] **Step 4: Check the dev workflow**

Run `make dev` in one terminal. Then check:
- `curl -s -o /dev/null -w '%{http_code}\n' http://localhost:8080/` gives `200`.
- A gRPC-Web call through Trunk's proxy works:

```bash
printf '\x00\x00\x00\x00\x00' > /tmp/empty.bin
curl -s -D - -o /dev/null -X POST -H 'content-type: application/grpc-web+proto' -H 'x-grpc-web: 1' \
  --data-binary @/tmp/empty.bin http://localhost:8080/doris.auth.v1.AuthService/GetStatus
```

Expected: `HTTP/1.1 200 OK` and `content-type: application/grpc-web+proto`. Stop `make dev` with Ctrl-C, and delete `doris.db*` if it was created in the repo root (it is gitignored).

- [ ] **Step 5: Document in AGENTS.md**

In `AGENTS.md`, add before `## Style`:

```markdown
## Frontend
- `crates/web` is a Leptos 0.8 CSR app built with Trunk (`crates/web/Trunk.toml`
  pins Tailwind 4.3.3, the standalone CLI, so no Node is needed). Output goes to
  `crates/web/dist`, which the server embeds; it is never committed.
- `src/api.rs` holds the gRPC-Web client (cookies always included). A
  `<meta name="doris-api" content="https://api…">` in `index.html` points a
  CDN-hosted frontend at the API; empty means same origin.
- `src/passkey.rs` does the browser half of WebAuthn: webauthn-rs JSON in,
  `navigator.credentials.*`, JSON out.
- `src/errors.rs` maps API error codes to Swedish text. Add a line there for
  every new code.
- `src/ui.rs` holds the preset's components, with class lists copied from
  shadcn's generated output. Add more by generating them with
  `npx shadcn init -t vite -b radix -p b1Gdz9bFY` in a scratch directory and
  copying the classes.
- The crate also compiles for the host, so `cargo test`/`clippy --workspace`
  include it. Also lint the wasm build:
  `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`.
- E2E tests live in `e2e/` (Playwright). Every test spawns its own server on
  a fresh database, and pages get a Chrome DevTools virtual WebAuthn
  authenticator. Select elements by their Swedish label or role.
```

In the `## Commands` block, replace the four `make` lines with:

```
make dev       # server :3000 + `trunk serve` :8080 (open http://localhost:8080)
make test      # cargo test --workspace (all crates, incl. doris-web unit tests)
make web       # debug frontend build into crates/web/dist
make e2e       # frontend + debug server, then Playwright
make dist      # target/dist/doris (frontend embedded) + doris-web-<ver>.tar.gz
make e2e-dist  # Playwright against the release binary from `make dist`
```

- [ ] **Step 6: Commit**

Check with `git status` that no build output, `node_modules`, test results or `.db` files are listed, then:

```bash
git add Cargo.toml Makefile AGENTS.md
git commit -m "Add make dist, dev and e2e-dist; fix release builds on macOS 27"
```

---

## Verification (whole of step 1)
- `make test`: every Rust test passes (event store, identity, WebAuthn, server, web).
- `make e2e` and `make e2e-dist`: 8 Playwright tests pass.
- Manually:
  1. Run `make dist && DORIS_DATABASE=sqlite://doris.db ./target/dist/doris` and open `http://localhost:3000`.
  2. Register the first admin with a real passkey (Touch ID).
  3. Sign out and back in.
  4. Invite a second email and open the link in another browser profile.
- `sqlite3 doris.db "UPDATE events SET payload='{}'"` is refused by the append-only trigger.
