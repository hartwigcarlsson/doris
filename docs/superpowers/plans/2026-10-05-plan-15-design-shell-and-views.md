# Plan 15: New Design, Part 1 (Shell and Views) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every view in `crates/web` uses the approved "A2" design: a one-row header with grouped menus, a 72rem page, a shared page header, tables in cards and status badges.

**Architecture:**
- `crates/web/src/ui.rs` gains the shared pieces: `Icon`, `Badge`, `Variant::Outline`, `LinkButton`, `PageHeader`, `TableCard`, and `Card` gets `narrow`.
- A new `crates/web/src/nav.rs` owns the header. Menus are native `<details name="doris-nav">`; one document listener closes them.
- Each view swaps its hand-written heading row and bare `<Table>` for the shared components. No view gains a feature, and nothing outside `crates/web` and `e2e/` changes.

**Tech Stack:** Rust, Leptos 0.8 CSR, leptos_router 0.8, Tailwind v4 (standalone CLI via Trunk), Playwright 1.63.

**Spec:** `docs/superpowers/specs/2026-10-05-ny-design-skal-och-vyer-design.md`. The visual reference is the canvas https://claude.ai/artifact/FpdLiRfFggm3ZVSuM6nyUL (pages "Runda 2 · Meny", artboard A2, and "Verifikationer").

## Global Constraints
- Frontend only: no events, commands, projections, migrations, protos or RPCs.
- No view gains or loses a feature. Every Swedish text that exists today stays unless a task says otherwise.
- No new dependency. Icons are inlined SVG from lucide-static 1.52.0. Only new `web-sys` features may be added.
- Values come from shadcn preset `b1Gdz9bFY`. `crates/web/style/input.css` is not changed.
- Code, identifiers, comments and commit messages are English; UI text is Swedish.
- Header and `main` are `max-w-6xl` (72rem). A form card is at most 352px wide (`max-w-[22rem]`) and left-aligned. Login and registration stay centered.
- TDD: a failing test first, then the code, then a commit. Each task ends green on `cargo test --workspace`, both clippy commands, and the Playwright suite.
- Commands used throughout:
  - Unit: `cargo test -p doris-web`
  - Lint: `cargo clippy --workspace -- -D warnings` and `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
  - E2E build: `make web && cargo build -p doris-server`
  - E2E run: `cd e2e && npx playwright test` (one file: `npx playwright test design.spec.ts`; one test: add `-g "name"`)
- Commits end with:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01JHpfH7CxMW86ZHVLURbK65
  ```

## Review Focus
1. **A menu link to the page you are already on.** The path does not change, so the menu must still close. Pinned in Task 2 ("a menu closes when its current page is chosen").
2. **A user with no company.** The header shows Doris, "Lägg till företag" and the account menu, and no main menu. Pinned in Task 2 ("without a company there is no main menu").
3. **A display name of one word, or with surrounding spaces.** Initials must not panic or be empty. Pinned in Task 2 (`initials` unit tests).
4. **A 390px screen.** The header wraps and an open menu must not push the page wider than the screen. Pinned in Task 2 ("nothing spills").
5. **Dark mode.** The panel uses `bg-popover`, not a hard-coded white. Pinned in Task 2 ("the menu panel follows the colour scheme").

---

## File Structure
| File | Responsibility |
|---|---|
| `crates/web/src/ui.rs` (modify) | Preset components. Adds `Icon`/`IconName`, `Badge`/`BadgeVariant`, `Variant::Outline`, `LinkButton`, `PageHeader`, `TableCard`; `Card` gets `narrow` and an `<h2>`; `PaperclipIcon` is removed. |
| `crates/web/src/nav.rs` (create) | `Header`, `NavMenu`, `NavItem`, the pure `section_of` and `initials`, and the listener that closes menus. |
| `crates/web/src/app.rs` (modify) | Loses `Header`; `main` gets the new width. |
| `crates/web/src/main.rs` (modify) | `mod nav;` next to the other modules. |
| `crates/web/Cargo.toml` (modify) | `web-sys` features `NodeList`, `Node`, `Event`, `EventTarget`, `KeyboardEvent`. |
| `crates/web/src/pages/*.rs` (modify) | Each view moves to the shared components. |
| `e2e/tests/fixtures.ts` (modify) | `openMenu`, `goTo`. |
| `e2e/tests/design.spec.ts` (modify) | The design tests in the spec. |
| `e2e/tests/*.spec.ts` (modify) | Header clicks go through `goTo`. |
| `AGENTS.md` (modify) | Frontend and Style sections describe the new shell. |

---

### Task 1: Shared components in `ui.rs`

**Files:**
- Modify: `crates/web/src/ui.rs`
- Modify: `crates/web/src/pages/vouchers.rs` (only the `PaperclipIcon` import and use)
- Test: `crates/web/src/ui.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `pub enum IconName { ReceiptText, Scale, ChartColumn, ListTree, CalendarRange, FileText, Building2, Banknote, Users, Building, KeyRound, MailPlus, LogOut, Plus, ChevronDown, ChevronRight, Paperclip }`
  - `#[component] pub fn Icon(name: IconName, #[prop(optional)] class: &'static str)` (default class `size-3.5`)
  - `pub enum BadgeVariant { Secondary (default), Outline, Destructive }`; `#[component] pub fn Badge(#[prop(optional)] variant: BadgeVariant, children: Children)`
  - `Variant::Outline` on the existing `Variant`
  - `#[component] pub fn LinkButton(#[prop(into)] href: String, #[prop(optional)] variant: Variant, #[prop(optional)] icon: Option<IconName>, children: Children)`
  - `#[component] pub fn PageHeader(#[prop(into)] title: Signal<String>, #[prop(optional, into)] description: Signal<String>, #[prop(optional)] children: Option<Children>)`
  - `#[component] pub fn TableCard(children: Children)`
  - `Card` gains `#[prop(optional)] narrow: bool`
  - `pub const NARROW: &str = "w-full max-w-[22rem]";`

- [ ] **Step 1: Write the failing tests**

Append to `crates/web/src/ui.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_shapes() {
        for name in IconName::ALL {
            let shapes = name.shapes();
            assert!(shapes.starts_with('<'), "{name:?}");
            // Leptos' `inner_html` takes the markup as is: no self-closing tags.
            assert!(!shapes.contains("/>"), "{name:?}");
        }
    }

    #[test]
    fn badge_variants_have_distinct_looks() {
        let looks = [
            BadgeVariant::Secondary.class(),
            BadgeVariant::Outline.class(),
            BadgeVariant::Destructive.class(),
        ];
        assert_ne!(looks[0], looks[1]);
        assert_ne!(looks[1], looks[2]);
        assert!(looks[2].contains("text-destructive"));
    }

    #[test]
    fn the_outline_button_has_a_border() {
        assert!(Variant::Outline.class().contains("border-border"));
        assert!(Variant::Default.class().contains("bg-primary"));
    }
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p doris-web ui::`
Expected: does not compile (`IconName`, `BadgeVariant`, `Variant::class` are missing).

- [ ] **Step 3: Implement**

In `crates/web/src/ui.rs`:

Replace the `Variant` enum and `Button` with:

```rust
const BUTTON_OUTLINE: &str = "border-border hover:bg-muted hover:text-foreground dark:bg-input/30";

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Variant {
    #[default]
    Default,
    Ghost,
    Outline,
}

impl Variant {
    fn class(self) -> &'static str {
        match self {
            Variant::Default => BUTTON_DEFAULT,
            Variant::Ghost => BUTTON_GHOST,
            Variant::Outline => BUTTON_OUTLINE,
        }
    }
}

#[component]
pub fn Button(
    #[prop(optional)] variant: Variant,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(default = "submit")] kind: &'static str,
    children: Children,
) -> impl IntoView {
    view! {
        <button type=kind class=format!("{BUTTON} {}", variant.class()) disabled=disabled>
            {children()}
        </button>
    }
}

/// A link that looks like a button, for "Ny …" actions.
#[component]
pub fn LinkButton(
    #[prop(into)] href: String,
    #[prop(optional)] variant: Variant,
    #[prop(optional)] icon: Option<IconName>,
    children: Children,
) -> impl IntoView {
    view! {
        <A href=href attr:class=format!("{BUTTON} {}", variant.class())>
            {icon.map(|name| view! { <Icon name=name /> })}
            {children()}
        </A>
    }
}
```

Add `use leptos_router::components::A;` at the top.

Add the icons (shapes copied from lucide-static 1.52.0, closing tags written out):

```rust
/// The lucide icons the app uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconName {
    ReceiptText,
    Scale,
    ChartColumn,
    ListTree,
    CalendarRange,
    FileText,
    Building2,
    Banknote,
    Users,
    Building,
    KeyRound,
    MailPlus,
    LogOut,
    Plus,
    ChevronDown,
    ChevronRight,
    Paperclip,
}

impl IconName {
    #[cfg(test)]
    const ALL: [IconName; 17] = [
        IconName::ReceiptText,
        IconName::Scale,
        IconName::ChartColumn,
        IconName::ListTree,
        IconName::CalendarRange,
        IconName::FileText,
        IconName::Building2,
        IconName::Banknote,
        IconName::Users,
        IconName::Building,
        IconName::KeyRound,
        IconName::MailPlus,
        IconName::LogOut,
        IconName::Plus,
        IconName::ChevronDown,
        IconName::ChevronRight,
        IconName::Paperclip,
    ];

    fn shapes(self) -> &'static str {
        match self {
            IconName::ReceiptText => r#"<path d="M13 16H8"></path><path d="M14 8H8"></path><path d="M16 12H8"></path><path d="M4 3a1 1 0 0 1 1-1 1.3 1.3 0 0 1 .7.2l.933.6a1.3 1.3 0 0 0 1.4 0l.934-.6a1.3 1.3 0 0 1 1.4 0l.933.6a1.3 1.3 0 0 0 1.4 0l.933-.6a1.3 1.3 0 0 1 1.4 0l.934.6a1.3 1.3 0 0 0 1.4 0l.933-.6A1.3 1.3 0 0 1 19 2a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1 1.3 1.3 0 0 1-.7-.2l-.933-.6a1.3 1.3 0 0 0-1.4 0l-.934.6a1.3 1.3 0 0 1-1.4 0l-.933-.6a1.3 1.3 0 0 0-1.4 0l-.933.6a1.3 1.3 0 0 1-1.4 0l-.934-.6a1.3 1.3 0 0 0-1.4 0l-.933.6a1.3 1.3 0 0 1-.7.2 1 1 0 0 1-1-1z"></path>"#,
            IconName::Scale => r#"<path d="M12 3v18"></path><path d="m19 8 3 8a5 5 0 0 1-6 0zV7"></path><path d="M3 7h1a17 17 0 0 0 8-2 17 17 0 0 0 8 2h1"></path><path d="m5 8 3 8a5 5 0 0 1-6 0zV7"></path><path d="M7 21h10"></path>"#,
            IconName::ChartColumn => r#"<path d="M3 3v16a2 2 0 0 0 2 2h16"></path><path d="M18 17V9"></path><path d="M13 17V5"></path><path d="M8 17v-3"></path>"#,
            IconName::ListTree => r#"<path d="M8 5h13"></path><path d="M13 12h8"></path><path d="M13 19h8"></path><path d="M3 10a2 2 0 0 0 2 2h3"></path><path d="M3 5v12a2 2 0 0 0 2 2h3"></path>"#,
            IconName::CalendarRange => r#"<rect x="3" y="3" width="18" height="18" rx="2"></rect><path d="M16 2v3"></path><path d="M3 9h18"></path><path d="M8 2v3"></path><path d="M17 13h-6"></path><path d="M13 17H7"></path><path d="M7 13h.01"></path><path d="M17 17h.01"></path>"#,
            IconName::FileText => r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 9H8"></path><path d="M16 13H8"></path><path d="M16 17H8"></path>"#,
            IconName::Building2 => r#"<path d="M10 12h4"></path><path d="M10 8h4"></path><path d="M14 21v-3a2 2 0 0 0-4 0v3"></path><path d="M6 10H4a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-2"></path><path d="M6 21V5a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v16"></path>"#,
            IconName::Banknote => r#"<rect width="20" height="12" x="2" y="6" rx="2"></rect><circle cx="12" cy="12" r="2"></circle><path d="M6 12h.01M18 12h.01"></path>"#,
            IconName::Users => r#"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"></path><path d="M16 3.128a4 4 0 0 1 0 7.744"></path><path d="M22 21v-2a4 4 0 0 0-3-3.87"></path><circle cx="9" cy="7" r="4"></circle>"#,
            IconName::Building => r#"<path d="M12 10h.01"></path><path d="M12 14h.01"></path><path d="M12 6h.01"></path><path d="M16 10h.01"></path><path d="M16 14h.01"></path><path d="M16 6h.01"></path><path d="M8 10h.01"></path><path d="M8 14h.01"></path><path d="M8 6h.01"></path><path d="M9 22v-3a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v3"></path><rect x="4" y="2" width="16" height="20" rx="2"></rect>"#,
            IconName::KeyRound => r#"<path d="M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z"></path><circle cx="16.5" cy="7.5" r=".5" fill="currentColor"></circle>"#,
            IconName::MailPlus => r#"<path d="M22 13V6a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v12c0 1.1.9 2 2 2h8"></path><path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"></path><path d="M19 16v6"></path><path d="M16 19h6"></path>"#,
            IconName::LogOut => r#"<path d="m16 17 5-5-5-5"></path><path d="M21 12H9"></path><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"></path>"#,
            IconName::Plus => r#"<path d="M5 12h14"></path><path d="M12 5v14"></path>"#,
            IconName::ChevronDown => r#"<path d="m6 9 6 6 6-6"></path>"#,
            IconName::ChevronRight => r#"<path d="m9 18 6-6-6-6"></path>"#,
            IconName::Paperclip => r#"<path d="m16 6-8.414 8.586a2 2 0 0 0 2.829 2.829l8.414-8.586a4 4 0 1 0-5.657-5.657l-8.379 8.551a6 6 0 1 0 8.485 8.485l8.379-8.551"></path>"#,
        }
    }
}

/// An inlined lucide icon. Decorative: the text next to it names the thing.
#[component]
pub fn Icon(name: IconName, #[prop(default = "size-3.5")] class: &'static str) -> impl IntoView {
    view! {
        <svg
            class=format!("shrink-0 {class}")
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
            inner_html=name.shapes()
        />
    }
}
```

Delete `PaperclipIcon`. In `crates/web/src/pages/vouchers.rs` replace its import with `Icon, IconName` and `<PaperclipIcon />` with `<Icon name=IconName::Paperclip />`. Replace the two hand-written chevron/check `<svg>` blocks in `Select` with `<Icon name=IconName::ChevronDown class="pointer-events-none absolute top-1/2 right-1.5 size-3.5 -translate-y-1/2 text-muted-foreground select-none" />`; leave the checkbox's check mark as it is.

Add the badge, page header and table card:

```rust
const BADGE: &str = "inline-flex h-5 w-fit shrink-0 items-center justify-center gap-1 overflow-hidden rounded-full border border-transparent px-2 py-0.5 text-[0.625rem] font-medium whitespace-nowrap";

#[derive(Clone, Copy, Default, PartialEq)]
pub enum BadgeVariant {
    #[default]
    Secondary,
    Outline,
    Destructive,
}

impl BadgeVariant {
    fn class(self) -> &'static str {
        match self {
            BadgeVariant::Secondary => "bg-secondary text-secondary-foreground",
            BadgeVariant::Outline => "border-border text-muted-foreground",
            BadgeVariant::Destructive => "bg-destructive/10 text-destructive dark:bg-destructive/20",
        }
    }
}

/// A status label. The text carries the meaning; the colour only helps.
#[component]
pub fn Badge(#[prop(optional)] variant: BadgeVariant, children: Children) -> impl IntoView {
    view! { <span class=format!("{BADGE} {}", variant.class())>{children()}</span> }
}

/// The page's one `<h1>`, an optional line under it, and actions to the right.
#[component]
pub fn PageHeader(
    #[prop(into)] title: Signal<String>,
    #[prop(optional, into)] description: Signal<String>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <div class="flex flex-wrap items-end justify-between gap-4">
            <div>
                <h1 class="text-sm font-medium">{move || title.get()}</h1>
                <Show when=move || !description.get().is_empty()>
                    <p class="text-xs/relaxed text-muted-foreground">{move || description.get()}</p>
                </Show>
            </div>
            {children.map(|actions| view! { <div class="flex flex-wrap items-center gap-2">{actions()}</div> })}
        </div>
    }
}

/// A card around a `Table`.
#[component]
pub fn TableCard(children: Children) -> impl IntoView {
    view! {
        <section class="overflow-hidden rounded-lg bg-card px-2 py-2 text-xs/relaxed text-card-foreground ring-1 ring-foreground/10">
            {children()}
        </section>
    }
}
```

Change `Card`: the page's `<h1>` now lives in `PageHeader`.

```rust
/// A form card's width in the preset: 352px.
pub const NARROW: &str = "w-full max-w-[22rem]";

#[component]
pub fn Card(
    title: &'static str,
    #[prop(optional)] description: &'static str,
    /// The preset's form width, left-aligned.
    #[prop(optional)]
    narrow: bool,
    children: Children,
) -> impl IntoView {
    let class = if narrow { format!("{CARD} {NARROW}") } else { CARD.to_owned() };
    view! {
        <section class=class>
            <header class="grid gap-1 px-4">
                <h2 class="text-sm font-medium">{title}</h2>
                {(!description.is_empty())
                    .then(|| view! { <p class="text-xs/relaxed text-muted-foreground">{description}</p> })}
            </header>
            <div class="px-4">{children()}</div>
        </section>
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-web` and both clippy commands from Global Constraints.
Expected: PASS, no warnings. (`register.rs` passes `title=title`: if `title` there is not `&'static str`, leave `Card`'s `title` type exactly as it is today.)

- [ ] **Step 5: Run the browser tests**

Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test`
Expected: PASS. Headings are found by name, so `<h2>` changes nothing.

- [ ] **Step 6: Commit**

```bash
git add crates/web/src/ui.rs crates/web/src/pages/vouchers.rs
git commit -m "Add the shared components for the new design"
```

---

### Task 2: The header (`nav.rs`), the page width and the e2e helpers

**Files:**
- Create: `crates/web/src/nav.rs`
- Modify: `crates/web/src/app.rs` (remove `Header`, change `main`), the crate root (`mod nav;`), `crates/web/Cargo.toml`
- Modify: `crates/web/src/active_company.rs:132` (the "Lägg till företag" link's class), `crates/web/src/pages/login.rs`, `crates/web/src/pages/register.rs`
- Modify: `e2e/tests/fixtures.ts`, `e2e/tests/design.spec.ts`, and every spec that clicks a header link
- Test: `crates/web/src/nav.rs` (unit), `e2e/tests/design.spec.ts`

**Interfaces:**
- Consumes: `Icon`, `IconName`, `NARROW` from Task 1; `Session`, `Companies`, `ActiveCompanySelect` as they are.
- Produces:
  - `pub fn Header() -> impl IntoView` in `crate::nav`
  - `pub enum Section { Bookkeeping, Purchases, Customers, Payroll }` and `pub fn section_of(path: &str) -> Option<Section>`
  - `pub fn initials(name: &str) -> String`
  - e2e: `openMenu(page: Page, menu: Menu): Promise<Locator>` and `goTo(page: Page, link: string): Promise<void>` in `fixtures.ts`, where `type Menu = "Bokföring" | "Inköp" | "Lön" | "Konto"`

- [ ] **Step 1: Write the failing unit tests**

Create `crates/web/src/nav.rs` with only:

```rust
//! The header: the company picker, the grouped main menu and the account menu.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_belongs_to_the_menu_that_lists_it() {
        use Section::*;
        for (path, section) in [
            ("/vouchers", Some(Bookkeeping)),
            ("/vouchers/new", Some(Bookkeeping)),
            ("/trial-balance", Some(Bookkeeping)),
            ("/trial-balance/1930", Some(Bookkeeping)),
            ("/financial-statements", Some(Bookkeeping)),
            ("/accounts", Some(Bookkeeping)),
            ("/fiscal-years", Some(Bookkeeping)),
            ("/opening-balances", Some(Bookkeeping)),
            ("/supplier-invoices", Some(Purchases)),
            ("/supplier-invoices/new", Some(Purchases)),
            ("/suppliers", Some(Purchases)),
            ("/customers", Some(Customers)),
            ("/payroll-runs", Some(Payroll)),
            ("/payroll-runs/abc", Some(Payroll)),
            ("/employees", Some(Payroll)),
            ("/", None),
            ("/companies", None),
            ("/companies/abc", None),
            ("/settings/passkeys", None),
            ("/admin/invitations", None),
            // A longer word that only starts the same is another page.
            ("/suppliers-old", None),
        ] {
            assert_eq!(section_of(path), section, "{path}");
        }
    }

    #[test]
    fn initials_come_from_the_first_two_words() {
        assert_eq!(initials("Erik Berg"), "EB");
        assert_eq!(initials("anna"), "A");
        assert_eq!(initials("  Åsa   von Ö  "), "ÅV");
        assert_eq!(initials(""), "");
    }
}
```

Add `mod nav;` between `mod format;` and `mod pages;` in `crates/web/src/main.rs`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p doris-web nav::`
Expected: does not compile (`Section`, `section_of`, `initials` missing).

- [ ] **Step 3: Implement the pure functions**

Above the tests in `nav.rs`:

```rust
/// The main menu's groups, for marking where the current page lives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Section {
    Bookkeeping,
    Purchases,
    Customers,
    Payroll,
}

const SECTIONS: [(&str, Section); 10] = [
    ("/vouchers", Section::Bookkeeping),
    ("/trial-balance", Section::Bookkeeping),
    ("/financial-statements", Section::Bookkeeping),
    ("/accounts", Section::Bookkeeping),
    ("/fiscal-years", Section::Bookkeeping),
    ("/opening-balances", Section::Bookkeeping),
    ("/supplier-invoices", Section::Purchases),
    ("/suppliers", Section::Purchases),
    ("/customers", Section::Customers),
    ("/payroll-runs", Section::Payroll),
];

/// The menu a path belongs to: the page itself or a page under it.
pub fn section_of(path: &str) -> Option<Section> {
    if path == "/employees" || path.starts_with("/employees/") {
        return Some(Section::Payroll);
    }
    SECTIONS
        .iter()
        .find(|(prefix, _)| {
            path.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
        .map(|(_, section)| *section)
}

/// Up to two initials for the account button: "Erik Berg" → "EB".
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .take(2)
        .filter_map(|word| word.chars().next())
        .flat_map(char::to_uppercase)
        .collect()
}
```

(Put `/employees` in `SECTIONS` as an eleventh row instead of the early return if you prefer; then the array length is 11.)

Run: `cargo test -p doris-web nav::` — Expected: PASS.

- [ ] **Step 4: Write the failing browser tests**

In `e2e/tests/fixtures.ts`, add `type Locator` to the Playwright import and append:

```ts
export type Menu = "Bokföring" | "Inköp" | "Lön" | "Konto";

const MENU_OF: Record<string, Menu | null> = {
  Översikt: null,
  Kunder: null,
  Verifikationer: "Bokföring",
  Saldobalans: "Bokföring",
  Rapporter: "Bokföring",
  Kontoplan: "Bokföring",
  Räkenskapsår: "Bokföring",
  Leverantörsfakturor: "Inköp",
  Leverantörer: "Inköp",
  Lönekörningar: "Lön",
  Anställda: "Lön",
  Företag: "Konto",
  Passkeys: "Konto",
  Inbjudningar: "Konto",
};

/** Opens one of the header's menus (if it is closed) and returns its panel. */
export async function openMenu(page: Page, menu: Menu): Promise<Locator> {
  const details = page.getByRole("banner").locator("details").filter({ has: page.locator("summary", { hasText: menu }) });
  if (!(await details.evaluate((d: HTMLDetailsElement) => d.open))) await details.locator("summary").click();
  return details.getByRole("list");
}

/** Follows a link in the header, opening the menu that holds it first. */
export async function goTo(page: Page, link: string) {
  const menu = MENU_OF[link];
  if (menu === undefined) throw new Error(`no header link called ${link}`);
  const scope = menu ? await openMenu(page, menu) : page.getByRole("banner");
  await scope.getByRole("link", { name: link, exact: true }).click();
}
```

In `e2e/tests/design.spec.ts`, import `goTo, openMenu` too, replace the test "the header keeps the company picker readable" and add the rest:

```ts
const linksIn = async (panel: import("@playwright/test").Locator) =>
  (await panel.getByRole("link").allInnerTexts()).map((t) => t.trim());

test("the header is one row with grouped menus", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna Lind" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.setViewportSize({ width: 1280, height: 800 });
  const banner = page.getByRole("banner");

  const height = (await banner.boundingBox())!.height;
  expect(height).toBeGreaterThanOrEqual(48);
  expect(height).toBeLessThanOrEqual(49);
  await expect(banner.getByRole("link", { name: "Översikt" })).toBeVisible();
  await expect(banner.getByRole("link", { name: "Kunder" })).toBeVisible();
  await expect(banner.getByRole("link", { name: "Verifikationer" })).toBeHidden();

  expect(await linksIn(await openMenu(page, "Bokföring"))).toEqual(["Verifikationer", "Saldobalans", "Rapporter", "Kontoplan", "Räkenskapsår"]);
  expect(await linksIn(await openMenu(page, "Inköp"))).toEqual(["Leverantörsfakturor", "Leverantörer"]);
  // Opening one menu closed the one before it.
  await expect(banner.getByRole("link", { name: "Verifikationer" })).toBeHidden();
  expect(await linksIn(await openMenu(page, "Lön"))).toEqual(["Lönekörningar", "Anställda"]);
  const account = await openMenu(page, "Konto");
  expect(await linksIn(account)).toEqual(["Företag", "Passkeys", "Inbjudningar"]);
  await expect(account.getByRole("button", { name: "Logga ut" })).toBeVisible();
  await expect(banner.getByText("AL", { exact: true })).toBeVisible();
});

test("a menu closes on Escape, on a click outside and when its current page is chosen", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  const banner = page.getByRole("banner");
  const link = banner.getByRole("link", { name: "Verifikationer" });

  await openMenu(page, "Bokföring");
  await page.keyboard.press("Escape");
  await expect(link).toBeHidden();

  await openMenu(page, "Bokföring");
  await page.getByRole("main").click({ position: { x: 5, y: 5 } });
  await expect(link).toBeHidden();

  await goTo(page, "Verifikationer");
  await expect(page).toHaveURL(/\/vouchers$/);
  await expect(link).toBeHidden();
  // Already on the page: the path does not change, and the menu still closes.
  await goTo(page, "Verifikationer");
  await expect(link).toBeHidden();
});

test("the header marks where the current page lives", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/vouchers/new`);
  const banner = page.getByRole("banner");
  await expect(banner.locator("summary", { hasText: "Bokföring" })).toHaveAttribute("data-current", "true");
  await expect(banner.locator("summary", { hasText: "Inköp" })).not.toHaveAttribute("data-current", "true");
  await page.goto(`${app}/vouchers`);
  const panel = await openMenu(page, "Bokföring");
  await expect(panel.getByRole("link", { name: "Verifikationer" })).toHaveAttribute("aria-current", "page");
});

test("without a company there is no main menu", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  const banner = page.getByRole("banner");
  await expect(banner.getByRole("link", { name: "Lägg till företag" })).toBeVisible();
  await expect(banner.locator("summary", { hasText: "Bokföring" })).toHaveCount(0);
  await expect(banner.locator("summary", { hasText: "Konto" })).toHaveCount(1);
});

test("nothing spills out of the header, with a menu open or not", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 800 });
    const picker = await page.getByLabel("Aktivt företag").boundingBox();
    expect(picker!.width, `picker at ${width}px`).toBeGreaterThanOrEqual(150);
    for (const menu of [null, "Bokföring", "Konto"] as const) {
      if (menu) await openMenu(page, menu);
      const wider = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
      expect(wider, `page scrolls sideways at ${width}px with ${menu ?? "no menu"} open`).toBe(false);
      await page.keyboard.press("Escape");
    }
  }
});

test("the menu panel follows the colour scheme", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  const background = async () => (await openMenu(page, "Bokföring")).evaluate((el) => getComputedStyle(el).backgroundColor);
  await page.emulateMedia({ colorScheme: "light" });
  const light = await background();
  await page.keyboard.press("Escape");
  await page.emulateMedia({ colorScheme: "dark" });
  expect(await background()).not.toBe(light);
});
```

- [ ] **Step 5: Run them and see them fail**

Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test design.spec.ts`
Expected: the six new tests FAIL (no `details` in the banner); "forms follow the preset's spacing" still passes.

- [ ] **Step 6: Implement the header**

`crates/web/Cargo.toml`: add `"Event", "EventTarget", "KeyboardEvent", "Node", "NodeList"` to the `web-sys` features.

Append to `crates/web/src/nav.rs` (above the tests), and put the `use` lines at the top of the file:

```rust
use crate::active_company::{ActiveCompanySelect, Companies};
use crate::api::{api, pb};
use crate::app::Session;
use crate::ui::{Icon, IconName};
use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::{use_location, use_navigate};
use wasm_bindgen::JsCast;

/// Every menu shares this name, so the browser keeps one open at a time.
const MENU_NAME: &str = "doris-nav";
const TOP: &str = "inline-flex h-7 items-center gap-1 rounded-md px-2 text-muted-foreground hover:bg-muted hover:text-foreground aria-[current=page]:bg-muted aria-[current=page]:font-medium aria-[current=page]:text-foreground data-[current=true]:font-medium data-[current=true]:text-foreground";
const PANEL: &str = "absolute top-8 z-10 grid min-w-46 gap-0 rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10";
const ITEM: &str = "flex h-7 w-full items-center gap-2 rounded-sm px-2 whitespace-nowrap hover:bg-muted aria-[current=page]:bg-muted aria-[current=page]:font-medium";

/// Closes every open menu, except the one `keep` is inside.
fn close_menus(keep: Option<&web_sys::Element>) {
    let Ok(open) = document().query_selector_all(&format!("details[name='{MENU_NAME}'][open]")) else {
        return;
    };
    for i in 0..open.length() {
        let Some(menu) = open.item(i).and_then(|n| n.dyn_into::<web_sys::Element>().ok()) else {
            continue;
        };
        if keep.is_none_or(|el| !menu.contains(Some(el))) {
            let _ = menu.remove_attribute("open");
        }
    }
}

/// One group in the header. `label` is the menu's visible name unless
/// `summary` draws its own (the account menu); it is always its accessible one.
#[component]
fn NavMenu(
    label: &'static str,
    #[prop(optional, into)] current: Signal<bool>,
    #[prop(optional)] right: bool,
    #[prop(optional)] summary: Option<Children>,
    children: Children,
) -> impl IntoView {
    let side = if right { "right-0" } else { "left-0" };
    view! {
        <details name=MENU_NAME class="relative">
            <summary
                class=format!("{TOP} cursor-pointer list-none select-none [&::-webkit-details-marker]:hidden")
                data-current=move || current.get().to_string()
            >
                {match summary {
                    Some(own) => view! { <span class="sr-only">{label}</span> {own()} }.into_any(),
                    None => view! { <span>{label}</span> }.into_any(),
                }}
                <Icon name=IconName::ChevronDown />
            </summary>
            <ul class=format!("{PANEL} {side}")>{children()}</ul>
        </details>
    }
}

#[component]
fn NavItem(href: &'static str, icon: IconName, label: &'static str) -> impl IntoView {
    view! {
        <li>
            <A href=href attr:class=ITEM>
                <Icon name=icon class="size-3.5 text-muted-foreground" />
                {label}
            </A>
        </li>
    }
}

#[component]
pub fn Header() -> impl IntoView {
    let session = expect_context::<Session>();
    let companies = expect_context::<Companies>();
    let navigate = use_navigate();
    let path = use_location().pathname;
    let section = Memo::new(move |_| section_of(&path.get()));
    let in_section = move |wanted: Section| Signal::derive(move || section.get() == Some(wanted));

    // A click outside a menu closes it; so does a click on one of its
    // links, since choosing the current page changes no path.
    let on_click = window_event_listener(ev::click, |event| {
        let target = event.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        let on_link = target.as_ref().is_some_and(|el| el.closest("details a").ok().flatten().is_some());
        close_menus(if on_link { None } else { target.as_ref() });
    });
    let on_key = window_event_listener(ev::keydown, |event| {
        if event.key() == "Escape" {
            close_menus(None);
        }
    });
    on_cleanup(move || {
        on_click.remove();
        on_key.remove();
    });
    Effect::new(move |_| {
        path.track();
        close_menus(None);
    });

    let log_out = move |_| {
        let navigate = navigate.clone();
        spawn_local(async move {
            let _ = api().logout(pb::LogoutRequest {}).await;
            session.user.set(None);
            navigate("/login", Default::default());
        });
    };
    let name = move || session.user.get().map(|u| u.display_name).unwrap_or_default();

    view! {
        <header class="border-b">
            <div class="mx-auto flex min-h-12 max-w-6xl flex-wrap items-center gap-x-4 gap-y-2 px-4 py-2 text-xs/relaxed">
                <A href="/" attr:class="text-sm font-semibold">"Doris"</A>
                <Show when=move || session.user.get().is_some()>
                    <ActiveCompanySelect />
                    <Show when=move || !companies.active.get().is_empty()>
                        <nav aria-label="Huvudmeny" class="flex flex-wrap items-center gap-1">
                            <A href="/" exact=true attr:class=TOP>"Översikt"</A>
                            <NavMenu label="Bokföring" current=in_section(Section::Bookkeeping)>
                                <NavItem href="/vouchers" icon=IconName::ReceiptText label="Verifikationer" />
                                <NavItem href="/trial-balance" icon=IconName::Scale label="Saldobalans" />
                                <NavItem href="/financial-statements" icon=IconName::ChartColumn label="Rapporter" />
                                <NavItem href="/accounts" icon=IconName::ListTree label="Kontoplan" />
                                <NavItem href="/fiscal-years" icon=IconName::CalendarRange label="Räkenskapsår" />
                            </NavMenu>
                            <NavMenu label="Inköp" current=in_section(Section::Purchases)>
                                <NavItem href="/supplier-invoices" icon=IconName::FileText label="Leverantörsfakturor" />
                                <NavItem href="/suppliers" icon=IconName::Building2 label="Leverantörer" />
                            </NavMenu>
                            <A href="/customers" attr:class=TOP>"Kunder"</A>
                            <NavMenu label="Lön" current=in_section(Section::Payroll)>
                                <NavItem href="/payroll-runs" icon=IconName::Banknote label="Lönekörningar" />
                                <NavItem href="/employees" icon=IconName::Users label="Anställda" />
                            </NavMenu>
                        </nav>
                    </Show>
                    <div class="ml-auto">
                        <NavMenu
                            label="Konto"
                            right=true
                            summary=Box::new(move || {
                                view! {
                                    <span aria-hidden="true" class="flex size-5 items-center justify-center rounded-full bg-muted text-[0.625rem] font-medium text-foreground">
                                        {move || initials(&name())}
                                    </span>
                                    <span class="font-medium text-foreground">{name}</span>
                                }
                                    .into_any()
                            })
                        >
                            <NavItem href="/companies" icon=IconName::Building label="Företag" />
                            <NavItem href="/settings/passkeys" icon=IconName::KeyRound label="Passkeys" />
                            <Show when=move || session.is_admin()>
                                <NavItem href="/admin/invitations" icon=IconName::MailPlus label="Inbjudningar" />
                            </Show>
                            <li aria-hidden="true" class="-mx-1 my-1 h-px bg-border"></li>
                            <li>
                                <button type="button" class=ITEM on:click=log_out.clone()>
                                    <Icon name=IconName::LogOut class="size-3.5 text-muted-foreground" />
                                    "Logga ut"
                                </button>
                            </li>
                        </NavMenu>
                    </div>
                </Show>
            </div>
        </header>
    }
}
```

Notes for the implementer:
- `<A>` in leptos_router 0.8 sets `aria-current="page"` on the link whose `href` matches the location. `/trial-balance` must not mark itself on `/trial-balance/1930` as *current page*; the `Bokföring` summary carries that through `data-current`. If `<A>` marks prefix matches by default, pass `exact=true` on every `NavItem` link.
- If `summary=Box::new(…)` does not satisfy `Option<Children>`, type the prop as `Option<ChildrenFn>` and wrap with `Arc::new`; the call site is the only user.
- A wide display name must not push the header past 390px: add `max-w-32 truncate` to the name's `<span>` if "nothing spills" fails at 390px.

`crates/web/src/app.rs`: delete `fn Header` and its now-unused imports (`ActiveCompanySelect`, `Button`, `Variant`, `use_navigate`, `A` if unused); add `use crate::nav::Header;`; change `main` to:

```rust
<main class="mx-auto w-full max-w-6xl px-4 py-10">
```

and delete the two comment lines above it about `data-wide`.

`crates/web/src/active_company.rs`: the wrapper `<div class="w-48 shrink-0">` stays. Nothing else changes.

`crates/web/src/pages/login.rs` and `register.rs`: wrap the `<Card …>…</Card>` in `<div class=format!("mx-auto {NARROW}")>…</div>` and import `NARROW` from `crate::ui`.

- [ ] **Step 7: Move the other specs to `goTo`**

In every spec, import `goTo` from `./fixtures` and replace each header click. The complete list:

| File | Lines | Before | After |
|---|---|---|---|
| `attachments.spec.ts` | 53 | `page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click()` | `goTo(page, "Räkenskapsår")` |
| `fiscal_year.spec.ts` | 24, 41, 60, 79 | same shape, `Räkenskapsår` | `goTo(page, "Räkenskapsår")` |
| `invoicing.spec.ts` | 10, 42 | `Kunder`, `Leverantörer` | `goTo(page, "Kunder")`, `goTo(page, "Leverantörer")` |
| `ledger.spec.ts` | 6, 60, 66, 167, 185, 189, 203, 224, 248, 265, 294 | `Kontoplan`, `Verifikationer`, `Saldobalans`, `Rapporter` | `goTo(page, "<same name>")` |
| `payroll.spec.ts` | 13, 24, 39, 57, 65, 74, 92, 104, 109, 116 | `Anställda`, `Lönekörningar`, `Verifikationer` | `goTo(page, "<same name>")` |
| `tax.spec.ts` | 11, 34, 53 | `Anställda`, `Lönekörningar` | `goTo(page, "<same name>")` |
| `supplier_invoices.spec.ts` | 28, 37, 40, 96, 98, 101, 106, 108 | `Leverantörsfakturor`, `Verifikationer`, `Räkenskapsår` | `goTo(page, "<same name>")` |
| `auth.spec.ts` | 39, 78, 95, 122, 130, 140 | `page.getByRole("link", { name: "Inbjudningar" \| "Passkeys" }).click()` | `goTo(page, "Inbjudningar")` / `goTo(page, "Passkeys")` |
| `auth.spec.ts` | 11 | `await expect(page.getByRole("link", { name: "Inbjudningar" })).toBeVisible()` | `await expect((await openMenu(page, "Konto")).getByRole("link", { name: "Inbjudningar" })).toBeVisible()` |
| `auth.spec.ts` | 49 | `toHaveCount(0)` on `bo`'s `Inbjudningar` link | unchanged: the link is not rendered for a non-admin |
| `companies.spec.ts` | 4 and its uses | `const nav = (page) => page.getByRole("link", { name: "Företag", exact: true })` then `nav(page).click()` | delete `nav`; `goTo(page, "Företag")`. Where `nav(page)` is only asserted visible, open the menu first with `openMenu(page, "Konto")` |
| `companies.spec.ts` | 62 | `Inbjudningar` click | `goTo(page, "Inbjudningar")` |

The three "Logga ut" clicks (`auth.spec.ts:17, 30, 88`) become `await (await openMenu(page, "Konto")).getByRole("button", { name: "Logga ut" }).click()`.

- [ ] **Step 8: Run everything**

Run: `cargo test --workspace`, both clippy commands, then `make web && cargo build -p doris-server && cd e2e && npx playwright test`
Expected: PASS. If "forms follow the preset's spacing" fails on `cardWidth`, the login/register wrapper from Step 6 is missing.

- [ ] **Step 9: Commit**

```bash
git add crates/web e2e/tests
git commit -m "Give the header one row with grouped menus"
```

---

### Task 3: Verifikationer and Ny verifikation

**Files:**
- Modify: `crates/web/src/pages/vouchers.rs`, `crates/web/src/pages/new_voucher.rs`
- Test: `e2e/tests/design.spec.ts`, `e2e/tests/ledger.spec.ts` (existing assertions must keep passing)

**Interfaces:**
- Consumes: `PageHeader`, `LinkButton`, `TableCard`, `Badge`, `Icon`, `IconName`, `Variant` from Task 1.

- [ ] **Step 1: Write the failing test**

Append to `e2e/tests/design.spec.ts`:

```ts
test("Verifikationer follows the design", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await goTo(page, "Verifikationer");
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { level: 1, name: "Verifikationer" })).toBeVisible();
  // The action is a button-shaped link with the primary colour, not a text link.
  const add = main.getByRole("link", { name: "Ny verifikation" });
  expect((await add.boundingBox())!.height).toBe(28);
  expect(await add.evaluate((el) => getComputedStyle(el).backgroundColor)).not.toBe("rgba(0, 0, 0, 0)");
  // The table sits in a card.
  await expect(main.locator("section").filter({ has: page.getByRole("table") })).toHaveCount(1);
  await add.click();
  await expect(main.getByRole("heading", { level: 1, name: "Ny verifikation" })).toBeVisible();
});
```

- [ ] **Step 2: Run it and see it fail**

Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test design.spec.ts -g "Verifikationer follows"`
Expected: FAIL on the link's height (a text link is not 28px high).

- [ ] **Step 3: Implement `vouchers.rs`**

Replace the top of the view (today lines 111–133, from `<div class="grid gap-6" data-wide>` through `<Table>`) with:

```rust
<div class="grid gap-6">
    <PageHeader title="Verifikationer">
        <Show when=move || years.with(|ys| is_closed(ys, &year.get()))>
            <Badge>"Stängt"</Badge>
        </Show>
        <div class="w-56">
            <Select label="Räkenskapsår" id="fiscal_year" hide_label=true value=year>
                {move || {
                    years
                        .get()
                        .into_iter()
                        .map(|y| view! { <option class=SELECT_OPTION value=y.start.clone()>{format!("{} – {}", y.start, y.end)}</option> })
                        .collect_view()
                }}
            </Select>
        </div>
        <LinkButton href="/vouchers/new" icon=IconName::Plus>"Ny verifikation"</LinkButton>
    </PageHeader>
    <ErrorAlert message=error />
    <TableCard>
        <Table>
```

and close `</TableCard>` after `</Table>`. `hide_label=true` keeps the label for `getByLabel("Räkenskapsår")`.

In `VoucherRow`:
- The number button (today line ~316) becomes:

```rust
<button
    type="button"
    class="inline-flex h-7 items-center gap-1 rounded-md pr-1.5 font-medium tabular-nums hover:bg-muted"
    aria-expanded=move || expanded.get().to_string()
    on:click=move |_| expanded.update(|e| *e = !*e)
>
    {move || view! { <Icon name=if expanded.get() { IconName::ChevronDown } else { IconName::ChevronRight } class="size-3.5 text-muted-foreground" /> }}
    {number}
</button>
```

- The status cell: `<td class=TABLE_CELL>{(!status.is_empty()).then(|| view! { <Badge>{status}</Badge> })}</td>`.
- The expanded row's `<tr class=TABLE_ROW>` gets ` bg-muted/50` appended (`class=format!("{TABLE_ROW} bg-muted/50")`).
- Replace the `<ul class="grid gap-1">` of lines with a small table, keeping the same account text and amounts so `ledger.spec.ts` still finds them:

```rust
<table class="w-full max-w-xl text-xs">
    <thead>
        <tr class="border-b text-muted-foreground">
            <th class="py-1 pr-2 text-left font-normal">"Konto"</th>
            <th class="px-2 py-1 text-right font-normal">"Debet"</th>
            <th class="py-1 pl-2 text-right font-normal">"Kredit"</th>
        </tr>
    </thead>
    <tbody>
        {lines
            .iter()
            .map(|l| {
                let name = names.with(|n| n.iter().find(|a| a.number == l.account).map(|a| a.name.clone()).unwrap_or_default());
                view! {
                    <tr class="border-b">
                        <td class="py-1.5 pr-2">{format!("{} {}", l.account, name)}</td>
                        <td class="px-2 py-1.5 text-right tabular-nums">{(l.debit > 0).then(|| amount(l.debit))}</td>
                        <td class="py-1.5 pl-2 text-right tabular-nums">{(l.credit > 0).then(|| amount(l.credit))}</td>
                    </tr>
                }
            })
            .collect_view()}
        <tr class="font-medium">
            <td class="py-1.5 pr-2">"Summa"</td>
            <td class="px-2 py-1.5 text-right tabular-nums">{amount(total)}</td>
            <td class="py-1.5 pl-2 text-right tabular-nums">{amount(total)}</td>
        </tr>
    </tbody>
</table>
```

Before changing the line list, run `grep -n "Debet\|Kredit" e2e/tests/*.spec.ts`. Any assertion on the old text `"Debet 1 000,00"`/`"Kredit 1 000,00"` is rewritten to assert the row instead, e.g. `page.getByRole("row", { name: /1930 .* 1 000,00/ })`; the amounts and account names it checks stay the same.

- [ ] **Step 4: Implement `new_voucher.rs`**

Replace `<Card title="Ny verifikation">` … `</Card>` (line 123 on) so the page has its header and the form sits in a full-width card:

```rust
<div class="grid gap-6">
    <PageHeader title="Ny verifikation" />
    <section class="rounded-lg bg-card p-4 text-xs/relaxed text-card-foreground ring-1 ring-foreground/10">
        <form class="grid gap-4" novalidate on:submit=submit>
            // …the form's children, unchanged…
        </form>
    </section>
</div>
```

Remove `data-wide` from the `<form>` and the `Card` import if unused.

- [ ] **Step 5: Run everything**

Run: `cargo test --workspace`, both clippy commands, the full Playwright suite.
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/web/src/pages/vouchers.rs crates/web/src/pages/new_voucher.rs e2e/tests
git commit -m "Move Verifikationer and Ny verifikation to the new design"
```

---

### Task 4: Saldobalans, Huvudbok, Rapporter, Kontoplan, Räkenskapsår, Ingående balanser

**Files:**
- Modify: `crates/web/src/pages/trial_balance.rs`, `account_ledger.rs`, `financial_statements.rs`, `accounts.rs`, `fiscal_years.rs`, `opening_balances.rs`
- Test: `e2e/tests/design.spec.ts`

**Interfaces:**
- Consumes: `PageHeader`, `TableCard`, `Badge`, `BadgeVariant`, `Card` (`narrow`), `LinkButton`, `Variant` from Task 1.

- [ ] **Step 1: Write the failing test**

Append to `e2e/tests/design.spec.ts` a helper and a test. Later tasks reuse the helper:

```ts
/** A view has one h1 in main, and every table in main sits in a card. */
async function expectDesign(page: import("@playwright/test").Page, heading: string | RegExp) {
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { level: 1, name: heading })).toBeVisible();
  await expect(main.getByRole("heading", { level: 1 })).toHaveCount(1);
  const loose = await main.evaluate((el) => [...el.querySelectorAll("table")].filter((t) => !t.closest("section")).length);
  expect(loose, "tables outside a card").toBe(0);
}

test("the bookkeeping views follow the design", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  for (const [link, heading] of [
    ["Saldobalans", "Saldobalans"],
    ["Rapporter", "Resultat- och balansräkning"],
    ["Kontoplan", "Kontoplan"],
    ["Räkenskapsår", "Räkenskapsår"],
  ] as const) {
    await goTo(page, link);
    await expectDesign(page, heading);
  }
  // Öppet is a badge, not bare cell text.
  const status = page.getByRole("main").getByText("Öppet", { exact: true });
  expect(await status.evaluate((el) => getComputedStyle(el).borderRadius)).not.toBe("0px");
  await page.getByRole("link", { name: "Ingående balanser" }).click();
  await expectDesign(page, /Ingående balanser/);
  await page.goto(`${app}/trial-balance/1930`);
  await expectDesign(page, /^1930/);
});
```

- [ ] **Step 2: Run it and see it fail**

Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test design.spec.ts -g "bookkeeping views"`
Expected: FAIL at Saldobalans ("tables outside a card" is not 0).

- [ ] **Step 3: Implement the six views**

The same three moves in each file: (1) the root `<div class="grid gap-6" data-wide>` loses `data-wide`; (2) the `<h1 class="text-sm font-medium">…</h1>` (and any flex row around it) becomes a `PageHeader`; (3) each `<Table>…</Table>` is wrapped in `<TableCard>…</TableCard>`.

`trial_balance.rs` (lines 105–112):

```rust
<div class="grid gap-6">
    <PageHeader title="Saldobalans">
        <Show when=closed>
            <Badge>"Stängt"</Badge>
        </Show>
        <FiscalYearSelect years=years year=year />
    </PageHeader>
    <ErrorAlert message=error />
```

`FiscalYearSelect` (in `crates/web/src/fiscal_year.rs`) renders a labelled select. In a `PageHeader` the label must be screen-reader-only: give it a `#[prop(optional)] hide_label: bool` that it passes to `Select`, and pass `hide_label=true` here and in the other headers. The section tables at line ~152 (`<h2 class="text-sm font-medium">{title}</h2>` + `<Table>`) become `<TableCard><h2 class="px-2 pt-1 text-sm font-medium">{title}</h2><Table>…</Table></TableCard>`.

`account_ledger.rs` (lines 97–104): the heading and the back link become

```rust
<div class="grid gap-6">
    <PageHeader title=Signal::derive(move || format!("{account} {}", name.get()).trim_end().to_owned())>
        <LinkButton href=format!("/trial-balance?fy={}", year.get_untracked()) variant=Variant::Outline>"Tillbaka till saldobalansen"</LinkButton>
    </PageHeader>
```

If `year` can change while the page is open, keep the existing reactive `<A href=move || …>` and give it the outline button's classes instead: `attr:class="inline-flex h-7 items-center rounded-md border border-border px-2 text-xs/relaxed font-medium hover:bg-muted dark:bg-input/30"`. The link text stays "Tillbaka till saldobalansen" (`ledger.spec.ts:237`).

`financial_statements.rs` (lines 56–63): `PageHeader title="Resultat- och balansräkning"` with the year select and, when the year is closed, `<Badge>"Räkenskapsåret är stängt"</Badge>` in its children. The two statements (line ~107) each become `<TableCard><h2 class="px-2 pt-1 text-sm font-medium">{title}</h2><Table>…</Table></TableCard>`, and their container becomes `<div class="grid gap-6 lg:grid-cols-2">`.

`accounts.rs` (lines 83–94): `PageHeader title="Kontoplan"`; `<Card title="Lägg till konto" narrow=true>`; the table in `TableCard`. Line 190: `<td class=TABLE_CELL>{if active { view! { <Badge>"Aktivt"</Badge> }.into_any() } else { view! { <Badge variant=BadgeVariant::Outline>"Inaktivt"</Badge> }.into_any() }}</td>`.

`fiscal_years.rs` (lines 57–80): `PageHeader title="Räkenskapsår"` with `<LinkButton href="/opening-balances" variant=Variant::Outline>"Ingående balanser"</LinkButton>` as its child, replacing the text link at line 80; the table in `TableCard`. Line 172: `<td class=TABLE_CELL>{if fiscal_year.closed { view! { <Badge variant=BadgeVariant::Outline>"Stängt"</Badge> }.into_any() } else { view! { <Badge>"Öppet"</Badge> }.into_any() }}</td>`.

`opening_balances.rs` (lines 106–136): the `<h1>` (it formats "Ingående balanser {start}") becomes `PageHeader title=Signal::derive(move || …same expression…)`; the table in `TableCard`.

- [ ] **Step 4: Run everything**

Run: `cargo test --workspace`, both clippy commands, the full Playwright suite.
Expected: PASS. `ledger.spec.ts` and `fiscal_year.spec.ts` find "Stängt"/"Öppet" as text and are unaffected.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src e2e/tests/design.spec.ts
git commit -m "Move the bookkeeping views to the new design"
```

---

### Task 5: Leverantörsfakturor, Ny leverantörsfaktura, Leverantörer, Kunder

**Files:**
- Modify: `crates/web/src/pages/supplier_invoices.rs`, `new_supplier_invoice.rs`, `suppliers.rs`, `customers.rs`
- Test: `crates/web/src/pages/supplier_invoices.rs` (unit), `e2e/tests/design.spec.ts`

**Interfaces:**
- Consumes: Task 1's components; `expectDesign` from Task 4.
- Produces: `fn status_badge(label: &str) -> BadgeVariant` in `supplier_invoices.rs`.

- [ ] **Step 1: Write the failing tests**

In `supplier_invoices.rs`'s test module:

```rust
#[test]
fn an_overdue_invoice_gets_the_destructive_badge() {
    assert!(status_badge("Förfallen") == BadgeVariant::Destructive);
    assert!(status_badge("Obetald") == BadgeVariant::Secondary);
    assert!(status_badge("Betald") == BadgeVariant::Outline);
    assert!(status_badge("Makulerad") == BadgeVariant::Outline);
}
```

In `e2e/tests/design.spec.ts`:

```ts
test("the purchase and customer views follow the design", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addSupplier(page, app, "Kontorshuset AB");
  await expectDesign(page, "Leverantörer");
  const active = page.getByRole("row", { name: /Kontorshuset AB/ }).getByText("Aktiv", { exact: true });
  expect(await active.evaluate((el) => getComputedStyle(el).borderRadius)).not.toBe("0px");
  await goTo(page, "Kunder");
  await expectDesign(page, "Kunder");
  await goTo(page, "Leverantörsfakturor");
  await expectDesign(page, "Leverantörsfakturor");
  const add = page.getByRole("main").getByRole("link", { name: "Ny leverantörsfaktura" });
  expect((await add.boundingBox())!.height).toBe(28);
  await add.click();
  await expectDesign(page, "Ny leverantörsfaktura");
});
```

(Import `addSupplier` from `./fixtures`.)

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p doris-web supplier_invoices` — Expected: does not compile (`status_badge` missing).
Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test design.spec.ts -g "purchase and customer"` — Expected: FAIL at Leverantörer ("tables outside a card").

- [ ] **Step 3: Implement**

`supplier_invoices.rs`:

```rust
/// How a status label is drawn: overdue stands out, settled ones recede.
fn status_badge(label: &str) -> BadgeVariant {
    match label {
        "Förfallen" => BadgeVariant::Destructive,
        "Obetald" => BadgeVariant::Secondary,
        _ => BadgeVariant::Outline,
    }
}
```

Lines 65–74: root loses `data-wide`; the heading row becomes

```rust
<PageHeader title="Leverantörsfakturor">
    <LinkButton href="/supplier-invoices/new" icon=IconName::Plus>"Ny leverantörsfaktura"</LinkButton>
</PageHeader>
```

keeping the link's existing text exactly; the table goes in `TableCard`. Where the row renders `{label}` in its status cell, write `<Badge variant=status_badge(label)>{label}</Badge>`.

`new_supplier_invoice.rs` (lines 227–228): the same shape as `new_voucher.rs` in Task 3 — `<div class="grid gap-6"><PageHeader title="Ny leverantörsfaktura" /><section class="rounded-lg bg-card p-4 text-xs/relaxed text-card-foreground ring-1 ring-foreground/10"><form class="grid gap-4" novalidate on:submit=submit>…</form></section></div>`, without `data-wide`.

`customers.rs` (lines 170–209) and `suppliers.rs` (lines 170–208), identical apart from the words:

```rust
<div class="grid gap-6">
    <PageHeader title="Kunder">
        <Button
            kind="button"
            on:click=move |_| {
                error.set(None);
                form.clear();
                open.set(Some(None));
            }
        >
            <Icon name=IconName::Plus />
            "Ny kund"
        </Button>
    </PageHeader>
    <ErrorAlert message=error />
    <Show when=move || open.get().is_some()>
        <Card title="Kunduppgifter">
```

(`"Leverantörer"`, `"Ny leverantör"`, `"Leverantörsuppgifter"` in `suppliers.rs`.) The table goes in `TableCard`. Lines 272 / 271: `<td class=TABLE_CELL>{if active { view! { <Badge>"Aktiv"</Badge> }.into_any() } else { view! { <Badge variant=BadgeVariant::Outline>"Inaktiv"</Badge> }.into_any() }}</td>`.

- [ ] **Step 4: Run everything**

Run: `cargo test --workspace`, both clippy commands, the full Playwright suite.
Expected: PASS. `addSupplier` clicks `getByRole("button", { name: "Ny leverantör" })`: the icon is `aria-hidden`, so the name is unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src e2e/tests/design.spec.ts
git commit -m "Move the purchase and customer views to the new design"
```

---

### Task 6: Lönekörningar, Lönekörning, Anställda

**Files:**
- Modify: `crates/web/src/pages/payroll_runs.rs`, `payroll_run.rs`, `employees.rs`
- Test: `crates/web/src/pages/payroll_runs.rs` (unit), `e2e/tests/design.spec.ts`

**Interfaces:**
- Consumes: Task 1's components; `expectDesign` from Task 4.
- Produces: `pub fn status_badge(label: &str) -> BadgeVariant` in `payroll_runs.rs` (used by `payroll_run.rs` too).

- [ ] **Step 1: Write the failing tests**

In `payroll_runs.rs`'s test module:

```rust
#[test]
fn a_booked_run_recedes_and_an_open_one_does_not() {
    assert!(status_badge("Öppen") == BadgeVariant::Secondary);
    assert!(status_badge("Färdigställd") == BadgeVariant::Secondary);
    assert!(status_badge("Bokförd") == BadgeVariant::Outline);
}
```

Before writing it, read `status_label` (line 16) and use every label it can return; a label that is neither Öppen nor Färdigställd nor Bokförd (for example one for a run that is due) is `Secondary` and gets its own `assert!`.

In `e2e/tests/design.spec.ts`:

```ts
test("the payroll views follow the design", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await goTo(page, "Anställda");
  await expectDesign(page, "Anställda");
  await goTo(page, "Lönekörningar");
  await expectDesign(page, "Lönekörningar");
  const add = page.getByRole("main").getByRole("link", { name: "Ny lönekörning" });
  expect((await add.boundingBox())!.height).toBe(28);
  await add.click();
  await expectDesign(page, /lönekörning/i);
});
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p doris-web payroll_runs` — Expected: does not compile.
Run the new browser test with `-g "payroll views"` — Expected: FAIL at Anställda ("tables outside a card").

- [ ] **Step 3: Implement**

`payroll_runs.rs`:

```rust
/// How a run's status is drawn: a booked run is done and recedes.
pub fn status_badge(label: &str) -> BadgeVariant {
    match label {
        "Bokförd" => BadgeVariant::Outline,
        _ => BadgeVariant::Secondary,
    }
}
```

Lines 82–88: root loses `data-wide`; heading row becomes `<PageHeader title="Lönekörningar"><LinkButton href="/payroll-runs/new" icon=IconName::Plus>"Ny lönekörning"</LinkButton></PageHeader>`; the three tables (lines 88, 138, 170) each go in a `TableCard`. Where the list row shows `{label}`, write `<Badge variant=status_badge(label)>{label}</Badge>`.

`payroll_run.rs` (lines 400–401): root loses `data-wide`; the `<h1 class="text-sm font-medium">…</h1>` becomes `PageHeader` with `title=Signal::derive(move || …the same expression the h1 renders…)`; if the run's status is shown as text next to or under the heading, move it into the `PageHeader`'s children as `<Badge variant=status_badge(label)>{label}</Badge>`. The table at line 310 goes in `TableCard`. The button row (lines 339–379) is unchanged.

`employees.rs` (lines 213–274): `PageHeader title="Anställda"`; `<Card title="Anställd">` stays full width (it holds a multi-column form); the table in `TableCard`. Line 348: `<td class=TABLE_CELL>{if employee.active { view! { <Badge>"Aktiv"</Badge> }.into_any() } else { view! { <Badge variant=BadgeVariant::Outline>"Inaktiv"</Badge> }.into_any() }}</td>`.

- [ ] **Step 4: Run everything**

Run: `cargo test --workspace`, both clippy commands, the full Playwright suite. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src e2e/tests/design.spec.ts
git commit -m "Move the payroll views to the new design"
```

---

### Task 7: Företag, Nytt företag, Företagssidan, Passkeys, Inbjudningar, Startsidan

**Files:**
- Modify: `crates/web/src/pages/companies.rs`, `new_company.rs`, `company.rs`, `passkeys.rs`, `invitations.rs`, `home.rs`
- Test: `e2e/tests/design.spec.ts`

**Interfaces:**
- Consumes: Task 1's components; `expectDesign` from Task 4.

- [ ] **Step 1: Write the failing test**

```ts
test("the account views follow the design, with narrow left-aligned forms", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await page.setViewportSize({ width: 1280, height: 800 });
  await expectDesign(page, "Översikt");
  await goTo(page, "Företag");
  await expectDesign(page, "Företag");
  await page.getByRole("main").getByRole("link", { name: "Lägg till företag" }).click();
  await expectDesign(page, "Nytt företag");
  const { card, heading } = await page.getByRole("main").evaluate((main) => ({
    card: main.querySelector("section")!.getBoundingClientRect(),
    heading: main.querySelector("h1")!.getBoundingClientRect(),
  }));
  expect(card.width).toBeLessThanOrEqual(352);
  expect(card.left).toBe(heading.left);
  await goTo(page, "Passkeys");
  await expectDesign(page, "Passkeys");
  await goTo(page, "Inbjudningar");
  await expectDesign(page, "Inbjudningar");
  await addCompany(page, app, "5560160680", "Exempel AB");
  await expectDesign(page, "Exempel AB");
});
```

- [ ] **Step 2: Run it and see it fail**

Run with `-g "account views"`. Expected: FAIL at the first `expectDesign` (the start page has no `h1` named Översikt).

- [ ] **Step 3: Implement**

Read each file's `view!` first; the rule is the same everywhere: a `grid gap-6` root, a `PageHeader` first, form cards `narrow=true`, lists in a card. Every existing text stays.

`home.rs`: root `<div class="grid gap-6">`, then `<PageHeader title="Översikt" />`, then both cards with `narrow=true`. "Inloggad som …" stays word for word (`fixtures.ts` `register` waits for it), and so do "Aktivt företag" and "Visa företaget" (`companies.spec.ts:105-108`).

`companies.rs` (line 23): today one `Card title="Företag" description="Företagen du sköter bokföringen åt."`. It becomes

```rust
<div class="grid gap-6">
    <PageHeader title="Företag" description="Företagen du sköter bokföringen åt.">
        <LinkButton href="/companies/new" icon=IconName::Plus>"Lägg till företag"</LinkButton>
    </PageHeader>
    <section class="rounded-lg bg-card p-4 text-xs/relaxed text-card-foreground ring-1 ring-foreground/10">
        // …the list (or "Inga företag än."), unchanged…
    </section>
</div>
```

and the old text link at line ~60 is removed (the `LinkButton` replaces it, same text, still inside `main` for `addCompany`).

`new_company.rs`: wrap in `<div class="grid gap-6"><PageHeader title="Nytt företag" />…</div>`. The form's `Card` gets `narrow=true`; if its title is "Nytt företag", change the card's title to "Uppgifter" so the heading is not repeated — unless a spec looks for the card title (`grep -rn "Nytt företag\|Uppgifter" e2e/tests`); then keep the title.

`company.rs` (line 78): the `<h1 class="text-sm font-medium">{c.name.clone()}</h1>` moves out to `<PageHeader title=c.name.clone() />` above the details, which sit in a `Card`-styled `section` (`rounded-lg bg-card p-4 text-xs/relaxed ring-1 ring-foreground/10`, `NARROW`). `Card title="Medlemmar"` gets `narrow=true`.

`passkeys.rs` (lines 45, 60): `<PageHeader title="Passkeys" />`, then both cards with `narrow=true`.

`invitations.rs` (lines 52, 70): `<PageHeader title="Inbjudningar" />`, then `Card title="Bjud in"` with `narrow=true`. The second card is titled "Inbjudningar" today, and `auth.spec.ts:143,147` asserts that a *heading* "Inbjudningar" is absent before the first invitation and present after it. With a page `h1` of the same name that assertion would always pass; rename the list card to "Skickade inbjudningar" and change lines 143 and 147 to that name, so the test still tests the list.

- [ ] **Step 4: Run everything**

Run: `cargo test --workspace`, both clippy commands, the full Playwright suite. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src e2e/tests
git commit -m "Move the account views and the start page to the new design"
```

---

### Task 8: Clean-up, the one-h1 rule, `AGENTS.md` and the check against the canvas

**Files:**
- Modify: `e2e/tests/design.spec.ts`, `AGENTS.md`, any view still holding `data-wide`
- Test: `e2e/tests/design.spec.ts`

- [ ] **Step 1: Write the test**

```ts
test("every signed-in view has one h1 and no page scrolls sideways", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  const paths = [
    "/", "/companies", "/companies/new", "/accounts", "/vouchers", "/vouchers/new", "/customers", "/suppliers",
    "/supplier-invoices", "/supplier-invoices/new", "/trial-balance", "/trial-balance/1930", "/financial-statements",
    "/fiscal-years", "/opening-balances", "/employees", "/payroll-runs", "/payroll-runs/new", "/settings/passkeys",
    "/admin/invitations",
  ];
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 800 });
    for (const path of paths) {
      await page.goto(`${app}${path}`);
      await expect(page.getByRole("main").getByRole("heading", { level: 1 }), `${path} at ${width}px`).toHaveCount(1);
      const wider = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
      expect(wider, `${path} scrolls sideways at ${width}px`).toBe(false);
    }
  }
});
```

- [ ] **Step 2: Run it**

Run with `-g "every signed-in view"`.
Expected: PASS if Tasks 3–7 were complete. A failure names the path and width; fix that view with the same moves as its task (a `PageHeader`, a `TableCard` whose `Table` scrolls inside its own `overflow-x-auto`, or `min-w-0` on a flex child) and re-run.

- [ ] **Step 3: Remove what is left of the old shell**

Run: `grep -rn "data-wide\|PaperclipIcon\|max-w-3xl\|max-w-sm" crates/web/src crates/web/index.html`
Expected after fixing: no `data-wide`, no `PaperclipIcon`, no `max-w-3xl`; `max-w-sm` only where a card is deliberately that width. Delete every other hit.

- [ ] **Step 4: Update `AGENTS.md`**

In "## Frontend", add after the `src/ui.rs` bullet:

```markdown
- `src/nav.rs` holds the header: one row with Doris, the company picker,
  the main menu (Översikt, Bokföring, Inköp, Kunder, Lön) and the account
  menu. Menus are native `<details name="doris-nav">`, so the browser keeps
  one open; one listener closes them on Escape, on a click outside, and on
  a click on a link. `section_of` decides which menu a path belongs to: add
  a line there for every new page.
- A view is a `grid gap-6` that starts with `PageHeader` (the page's one
  `<h1>`, actions to the right). Tables sit in `TableCard`, statuses are
  `Badge`s, "Ny …" actions are `LinkButton`s, and form cards are
  `narrow` (352px, left-aligned). Login and registration are centered.
- Icons are lucide shapes inlined in `ui.rs` (`Icon`, `IconName`), copied
  from lucide-static with closing tags written out.
```

In "## Style", replace "lucide icons (inlined SVG)" detail if needed so it reads: "… small radius, lucide icons (inlined SVG, see `IconName`). The page and the header are `max-w-6xl`."

In the e2e bullet, append: "Header links live in menus: use `goTo(page, \"Verifikationer\")` from `fixtures.ts`."

- [ ] **Step 5: Run everything, and measure the wasm**

Run: `cargo test --workspace`, both clippy commands, the full Playwright suite. Expected: PASS.
Run: `make dist && ls -l crates/web/dist/*_bg.wasm` and note the size. For the size before, run the same in a `git worktree add ../doris-main main` checkout (remove it afterwards). Put both numbers in the PR description.

- [ ] **Step 6: Check against the canvas**

Run the `verify` skill (release binary, driven through the browser). At 1280px and 390px, in light and dark (`prefers-color-scheme`), open Översikt, Verifikationer with a voucher expanded, and Nytt företag, and compare with artboards "A2" and "Verifikationer" in the canvas: header order and height, menu panel, badges, card radius and ring, button heights (28px), table cell padding. Report differences instead of silently accepting them; fix those that contradict the spec.

- [ ] **Step 7: Commit**

```bash
git add -A crates/web e2e AGENTS.md
git commit -m "Finish the new design: one h1 per view, docs and clean-up"
```
