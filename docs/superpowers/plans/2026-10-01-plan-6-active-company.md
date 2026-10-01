# Plan 6: Active Company Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A signed-in user picks their active company in a dropdown in the header. The choice is remembered in the browser per user and shown on the start page.

**Architecture:**
- This is frontend only; there are no server changes.
- A new module, `crates/web/src/active_company.rs`, holds the pure rule `resolve_active`, the `localStorage` helpers and a `Companies` context. The context holds the list, the active id and a loaded flag.
- `app.rs` provides the context and reloads it whenever the signed-in user changes.
- The header shows the switcher, built from the existing `Select` component with a visually hidden label. The start page shows the active company, and the new-company form makes the new company active.

**Tech Stack:** Leptos 0.8 CSR, web-sys (`Storage`), Playwright.

**Spec:** `docs/superpowers/specs/2026-10-01-aktivt-foretag-design.md`

## Global Constraints
- The `localStorage` key is exactly `doris.active_company.{user id}`. The value is the company UUID and nothing else.
- Storage errors are ignored, for example in private mode or when storage is blocked. The choice then lives only for the current page view.
- Automatic choice: the stored or current id if it is in the list, otherwise the first company in `ListCompanies` order, otherwise none.
- A newly created company becomes active.
- The choice grants nothing. The server keeps checking membership on every company RPC. Document this in AGENTS.md.
- User-visible text is Swedish and must match exactly:
  - "Aktivt företag" (the select's hidden label and the start-page card title)
  - "Lägg till företag"
  - "Du har inga företag än."
  - "Visa företaget"
- The UI follows shadcn preset b1Gdz9bFY: reuse `Select`, `SELECT_OPTION` and `Card`, and invent no classes except layout utilities (`w-48`, `sr-only`).
- Lints: `cargo clippy --workspace --all-targets -- -D warnings` and `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`.
- The wasm stays under `WASM_BUDGET` (800 000 bytes). It is 765 398 bytes before this plan.
- TDD: a failing test first for every behavior, and a commit per task.

## Review Focus
1. **A stored company the user has lost access to**, or a company from another user on the same browser, must not stay active. The next company in the list takes over. *Test: Task 1, `a_stored_company_that_is_no_longer_listed_falls_back_to_the_first`.*
2. **Log out as Anna and log in as Bo in the same tab.** Bo must not see Anna's companies or her active choice, and a slow list response for Anna must not overwrite Bo's list. *Code: Task 2, the user-id guard in `Companies::load`. Test: Task 2 e2e `a colleague sees a company only after being added as a member`, which already switches users in separate contexts, plus the guard, which the reviewer checks by reading.*
3. **Private mode or blocked storage** must not break the header or the start page. *Code: Task 1, where every storage call is `Option`/`Result` and ignored on failure. The reviewer checks that no `unwrap`/`expect` touches storage.*
4. **Pre-existing e2e selectors** now meet a second "Lägg till företag" link, in the header. Every test must still pass unambiguously. *Test: Task 2 scopes `addCompany`'s link to `main`.*
5. **Just after saving a company,** the header must show that company as active, not the previous one. *Test: Task 2 e2e checks the selected option right after the second company is saved.*

---

## File Structure
```
crates/web/Cargo.toml                    web-sys feature "Storage"
crates/web/src/active_company.rs         NEW: resolve_active, storage, Companies context, ActiveCompanySelect
crates/web/src/main.rs                   mod active_company
crates/web/src/ui.rs                     Select: optional hide_label
crates/web/src/app.rs                    provide Companies, reload on user change, switcher in Header
crates/web/src/pages/home.rs             "Aktivt företag" card
crates/web/src/pages/new_company.rs      activate the new company
e2e/tests/companies.spec.ts              scope link to main, new active-company test
AGENTS.md                                one paragraph on the active company
```

---

### Task 1: `resolve_active` and browser storage

**Files:**
- Create: `crates/web/src/active_company.rs`
- Modify: `crates/web/src/main.rs` (add `mod active_company;` in alphabetical order, before `mod api;`), `crates/web/Cargo.toml` (add `"Storage"` to the web-sys features list)

**Interfaces:**
- Produces:
  - `pub fn resolve_active(preferred: Option<&str>, companies: &[cpb::CompanySummary]) -> Option<String>`
  - private `fn remembered(user_id: &str) -> Option<String>`
  - private `fn remember(user_id: &str, company_id: &str)`
  - private `fn storage_key(user_id: &str) -> String`
- Task 2 adds the `Companies` context to this same file.

- [ ] **Step 1: Write the failing tests** (the new file, with a stub)

```rust
//! The active company: the one the user is keeping the books for right now.
//! Chosen in the header and remembered per user in this browser's
//! localStorage. It grants nothing: every company RPC still checks
//! membership on the server.

use crate::api::cpb;

/// Which company is active: `preferred` (the current or remembered choice)
/// if the user still has it, otherwise the first one listed.
pub fn resolve_active(preferred: Option<&str>, companies: &[cpb::CompanySummary]) -> Option<String> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn company(id: &str) -> cpb::CompanySummary {
        cpb::CompanySummary { id: id.into(), org_nr: "556016-0680".into(), name: id.into() }
    }

    #[test]
    fn a_stored_company_that_is_still_listed_stays_active() {
        let list = [company("a"), company("b")];
        assert_eq!(resolve_active(Some("b"), &list).as_deref(), Some("b"));
    }

    #[test]
    fn a_stored_company_that_is_no_longer_listed_falls_back_to_the_first() {
        let list = [company("a"), company("b")];
        assert_eq!(resolve_active(Some("gone"), &list).as_deref(), Some("a"));
    }

    #[test]
    fn without_a_stored_choice_the_first_company_is_active() {
        let list = [company("a"), company("b")];
        assert_eq!(resolve_active(None, &list).as_deref(), Some("a"));
        assert_eq!(resolve_active(Some(""), &list).as_deref(), Some("a"));
    }

    #[test]
    fn no_companies_means_no_active_company() {
        assert_eq!(resolve_active(Some("a"), &[]), None);
        assert_eq!(resolve_active(None, &[]), None);
    }

    #[test]
    fn the_storage_key_is_per_user() {
        assert_eq!(storage_key("u-1"), "doris.active_company.u-1");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-web active_company`
Expected: compile error, because `storage_key` is not found. Once that is stubbed, the tests panic at `todo!()`.

- [ ] **Step 3: Implement** (replace the stub and add the storage helpers above the tests)

```rust
pub fn resolve_active(preferred: Option<&str>, companies: &[cpb::CompanySummary]) -> Option<String> {
    companies
        .iter()
        .find(|c| Some(c.id.as_str()) == preferred)
        .or(companies.first())
        .map(|c| c.id.clone())
}

fn storage_key(user_id: &str) -> String {
    format!("doris.active_company.{user_id}")
}

/// `None` in private mode or when storage is blocked: the choice then lives
/// only as long as the page.
fn storage() -> Option<web_sys::Storage> {
    leptos::prelude::window().local_storage().ok().flatten()
}

fn remembered(user_id: &str) -> Option<String> {
    storage()?.get_item(&storage_key(user_id)).ok().flatten()
}

fn remember(user_id: &str, company_id: &str) {
    if let Some(storage) = storage() {
        let _ = storage.set_item(&storage_key(user_id), company_id);
    }
}
```

`remembered` and `remember` are unused until Task 2. So that the wasm clippy passes in this task, mark them `#[allow(dead_code)] // used from Task 2` and remove the attribute in Task 2. Only the attribute line changes.

- [ ] **Step 4: Run the tests and lints**

Run:
```bash
cargo test -p doris-web active_company
cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: 5 passed and both lints clean. `resolve_active` itself is used by the tests only; if clippy flags it as dead code in the non-test build, give it the same temporary `#[allow(dead_code)]`.

- [ ] **Step 5: Commit**

```bash
git add crates/web
git commit -m "Choose the active company: remembered if still listed, else the first"
```

---

### Task 2: `Companies` context, header switcher, start page, new-company activation

**Files:**
- Modify:
  - `crates/web/src/active_company.rs`
  - `crates/web/src/ui.rs`
  - `crates/web/src/app.rs`
  - `crates/web/src/pages/home.rs`
  - `crates/web/src/pages/new_company.rs`
  - `e2e/tests/companies.spec.ts`
  - `AGENTS.md`

**Interfaces:**
- Consumes from Task 1: `resolve_active`, `remembered`, `remember`. Also `company_api()` and `cpb` from `crate::api`, and `Select` and `SELECT_OPTION` from `crate::ui`.
- Produces the `Companies` context (`Clone + Copy`) with:
  - `pub list: RwSignal<Vec<cpb::CompanySummary>>`
  - `pub active: RwSignal<String>`, where `""` means none
  - `pub loaded: RwSignal<bool>`
  - private `user_id: RwSignal<Option<String>>`
- Methods:
  - `Companies::new() -> Self`, which also installs the remember-effect
  - `load(self, user_id: String)`
  - `reload(self)`
  - `clear(self)`
  - `active_company(&self) -> Option<cpb::CompanySummary>` (tracked)
- Components: `ActiveCompanySelect`, and `Select` with a new `#[prop(optional)] hide_label: bool`.

- [ ] **Step 1: Write the failing e2e tests** (`e2e/tests/companies.spec.ts`)

In `addCompany`, scope the list link to the page body. The header gets its own "Lägg till företag" link when the user has no companies:
```ts
  await page.getByRole("main").getByRole("link", { name: "Lägg till företag" }).click();
```

Append:
```ts
test("the active company is chosen in the header and remembered", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  const header = page.getByRole("banner");
  await expect(header.getByRole("link", { name: "Lägg till företag" })).toBeVisible();
  await expect(page.getByLabel("Aktivt företag")).toHaveCount(0);
  await expect(page.getByText("Du har inga företag än.")).toBeVisible();

  await addCompany(page, app, "5560160680", "Exempel AB");
  await addCompany(page, app, "5560360793", "Bolaget AB");
  const active = () => page.getByLabel("Aktivt företag");
  await expect(active().locator("option:checked")).toHaveText("Bolaget AB"); // the new one

  await active().selectOption({ label: "Exempel AB" });
  await page.reload();
  await expect(active().locator("option:checked")).toHaveText("Exempel AB");
  await page.goto(app);
  const card = page.getByRole("main");
  await expect(card.getByRole("heading", { name: "Aktivt företag" })).toBeVisible();
  await expect(card.getByText("Exempel AB")).toBeVisible();
  await expect(card.getByText("556016-0680")).toBeVisible();
  await card.getByRole("link", { name: "Visa företaget" }).click();
  await expect(page.getByRole("heading", { name: "Exempel AB" })).toBeVisible();
});
```

- [ ] **Step 2: Run them to verify they fail**

Run: `make e2e`
Expected: the new test fails, because there is no "Lägg till företag" link in the banner. The existing tests still pass, since the `main` scoping matches what is on the page today.

- [ ] **Step 3: `Select` gets a visually hidden label option** (`ui.rs`)

Add `#[prop(optional)] hide_label: bool` after `id`, and render the label with `class=if hide_label { "sr-only" } else { LABEL }`. Nothing else changes. Keep the doc comment and add "`hide_label` keeps the label for screen readers only."

- [ ] **Step 4: The `Companies` context and the switcher** (append to `active_company.rs` above the tests; remove Task 1's `#[allow(dead_code)]` lines)

```rust
use crate::api::company_api;
use crate::ui::{SELECT_OPTION, Select};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

/// The signed-in user's companies and which one is active. Provided as
/// context by `App`, which loads it whenever the signed-in user changes.
#[derive(Clone, Copy)]
pub struct Companies {
    pub list: RwSignal<Vec<cpb::CompanySummary>>,
    /// The active company's id; empty when there is none.
    pub active: RwSignal<String>,
    /// False until the list has been fetched for the current user.
    pub loaded: RwSignal<bool>,
    user_id: RwSignal<Option<String>>,
}

impl Companies {
    pub fn new() -> Self {
        let companies = Self {
            list: RwSignal::new(Vec::new()),
            active: RwSignal::new(String::new()),
            loaded: RwSignal::new(false),
            user_id: RwSignal::new(None),
        };
        // Remember every choice, whoever made it: the header, a new company,
        // or the automatic fallback.
        Effect::new(move |_| {
            let active = companies.active.get();
            if let Some(user_id) = companies.user_id.get_untracked() {
                if !active.is_empty() {
                    remember(&user_id, &active);
                }
            }
        });
        companies
    }

    /// Fetches `user_id`'s companies and settles the active one: the current
    /// choice, else the remembered one, if still listed; else the first.
    pub fn load(self, user_id: String) {
        if self.user_id.get_untracked().as_deref() != Some(user_id.as_str()) {
            self.clear();
            self.user_id.set(Some(user_id.clone()));
        }
        spawn_local(async move {
            let Ok(response) = company_api().list_companies(cpb::ListCompaniesRequest {}).await else {
                return;
            };
            // Another user signed in while this was in flight.
            if self.user_id.get_untracked().as_deref() != Some(user_id.as_str()) {
                return;
            }
            let list = response.into_inner().companies;
            let current = self.active.get_untracked();
            let preferred = if current.is_empty() { remembered(&user_id) } else { Some(current) };
            self.active.set(resolve_active(preferred.as_deref(), &list).unwrap_or_default());
            self.list.set(list);
            self.loaded.set(true);
        });
    }

    /// Fetches the list again for the same user, e.g. after adding a company.
    pub fn reload(self) {
        if let Some(user_id) = self.user_id.get_untracked() {
            self.load(user_id);
        }
    }

    pub fn clear(self) {
        self.user_id.set(None);
        self.list.set(Vec::new());
        self.active.set(String::new());
        self.loaded.set(false);
    }

    pub fn active_company(&self) -> Option<cpb::CompanySummary> {
        let active = self.active.get();
        self.list.with(|list| list.iter().find(|c| c.id == active).cloned())
    }
}

/// The header's company switcher, or a link to add the first company.
#[component]
pub fn ActiveCompanySelect() -> impl IntoView {
    let companies = expect_context::<Companies>();
    move || {
        if !companies.loaded.get() {
            return ().into_any();
        }
        if companies.list.with(Vec::is_empty) {
            return view! {
                <A href="/companies/new" attr:class="text-muted-foreground hover:text-foreground">"Lägg till företag"</A>
            }
            .into_any();
        }
        view! {
            <div class="w-48">
                <Select label="Aktivt företag" id="active_company" hide_label=true value=companies.active>
                    {move || {
                        companies
                            .list
                            .get()
                            .into_iter()
                            .map(|c| view! { <option class=SELECT_OPTION value=c.id>{c.name}</option> })
                            .collect_view()
                    }}
                </Select>
            </div>
        }
        .into_any()
    }
}
```

`Select` takes `children: Children`, which is `FnOnce`. The option list must stay reactive, so it is passed as one reactive closure child, as above. If `Select`'s `prop:value` is applied before the options render and the browser shows the wrong option, fix it in `Select` by re-applying the value after the children change. One way is `prop:value` reading `value.get()` inside an effect that also tracks the options. Report what you did.

- [ ] **Step 5: Wire it into the app** (`app.rs`)

- `use crate::active_company::{ActiveCompanySelect, Companies};`
- In `App`, after `provide_context(session);`:
```rust
    let companies = Companies::new();
    provide_context(companies);
    Effect::new(move |_| match session.user.get() {
        Some(user) => companies.load(user.id),
        None => companies.clear(),
    });
```
- In `Header`, inside the signed-in `<Show>`, put `<ActiveCompanySelect />` first, before the "Företag" link.

- [ ] **Step 6: The start page** (`pages/home.rs`)

Keep the "Välkommen" card and add an "Aktivt företag" card under it. Wrap both in `<div class="grid gap-6">`:
```rust
    let companies = expect_context::<Companies>();
    // …
        <Card title="Aktivt företag">
            {move || match companies.active_company() {
                Some(c) => view! {
                    <div class="grid gap-2">
                        <p><strong>{c.name.clone()}</strong> " " <span class="text-muted-foreground">{c.org_nr.clone()}</span></p>
                        <A href=format!("/companies/{}", c.id) attr:class="font-medium underline-offset-4 hover:underline">"Visa företaget"</A>
                    </div>
                }.into_any(),
                None if companies.loaded.get() => view! {
                    <div class="grid gap-2">
                        <p class="text-muted-foreground">"Du har inga företag än."</p>
                        <A href="/companies/new" attr:class="font-medium underline-offset-4 hover:underline">"Lägg till företag"</A>
                    </div>
                }.into_any(),
                None => ().into_any(),
            }}
        </Card>
```

The "Lägg till företag" link on the start page and the one in the header both exist when the user has no companies. The e2e test scopes its link to `banner`, so both are fine.

- [ ] **Step 7: A new company becomes active** (`pages/new_company.rs`)

Get `let companies = expect_context::<Companies>();` and, in the `Ok(created)` arm of the submit before navigating:
```rust
                Ok(created) => {
                    let id = created.into_inner().company_id;
                    companies.active.set(id.clone());
                    companies.reload();
                    navigate(&format!("/companies/{id}"), Default::default());
                }
```

- [ ] **Step 8: Document it** (`AGENTS.md`, Frontend section; add one bullet)

> - The active company (`src/active_company.rs`) is chosen in the header and remembered in `localStorage` as `doris.active_company.{user id}`. Pages that work on "the" company read it from the `Companies` context and send its `company_id` with every RPC. It grants nothing: the server checks membership on every call.

- [ ] **Step 9: Run everything**

Run:
```bash
cargo test -p doris-web
cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings
make e2e
make dist
```
Expected: all tests pass, 19 of 19 e2e, and the wasm under 800 000 bytes (report the size).

- [ ] **Step 10: Commit**

```bash
git add crates/web e2e AGENTS.md
git commit -m "Pick the active company in the header"
```
