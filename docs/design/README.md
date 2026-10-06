# Doris design rules

Every view in the web app (`crates/web`) follows one design. It was settled
in October 2026, and a new or changed view is not done until it follows the
rules here. A spec for a step with UI says how it does.

## Where the design comes from
- **Preset.** shadcn preset `b1Gdz9bFY`
  (https://ui.shadcn.com/create?preset=b1Gdz9bFY): style mira, base colour
  stone, theme amber, font Inter (self-hosted), small radius, lucide icons.
  Its tokens are in `crates/web/style/input.css`.
- **Sketches.** The canvas https://claude.ai/artifact/FpdLiRfFggm3ZVSuM6nyUL:
  the page "Runda 2 · Meny" (artboard A2: the shell and the overview) and
  the page "Verifikationer" (a list view with its expanded row).
- **Decisions.** The three specs in `docs/superpowers/specs/`:
  `2026-10-05-ny-design-skal-och-vyer-design.md` (the shell and all views),
  `2026-10-05-ny-design-oversikten-design.md` (the overview) and
  `2026-10-05-ny-design-verifikationssidan-design.md` (search, filters and
  history on Verifikationer).

## The shell
- The header (`crates/web/src/nav.rs`) is one row: Doris, the company
  picker, the main menu and the account menu.
- A page for signed-in users gets a place in the menus: a `NavItem` with a
  lucide icon in the group it belongs to (Bokföring, Inköp, Försäljning,
  Lön, or the account menu), and a line in `SECTIONS` so the group is marked
  while the page is open.
- A new group in the menu is a design decision: ask first.
- Sub-pages (`/x/new`, `/x/:id`) are reached from their list, not from the
  menu.
- The page and the header are `max-w-6xl`.

## A view
- A view is a `grid gap-6` that starts with `PageHeader`: the page's only
  `<h1>`, an optional line under it, and the page's actions to the right.
- "Ny …" is a primary `LinkButton` with the plus icon. Other actions in the
  header are outline buttons.
- Cards have `<h2>`.
- Login and registration are the exception: a centered card whose title is
  the `<h1>` (`page_title`).

## Components
Use what `crates/web/src/ui.rs` has before writing markup.

| Need | Component |
|---|---|
| A table | `TableCard` around `Table`; its `toolbar` holds search and filters |
| A one-column form | `Card` with `narrow` (352px, left-aligned) |
| A wide form, a line editor or a list | `Panel`, or a full-width `Card` |
| A status | `Badge`; the text carries the meaning, never the colour alone |
| An action | `Button`, or `LinkButton` when it navigates |
| An icon | `Icon` with an `IconName` |

A component that is missing is added to `ui.rs`, with its classes copied
from the preset's generated output (`npx shadcn init -t vite -b radix -p
b1Gdz9bFY` in a scratch directory), not improvised in a page. A new icon is
a lucide shape copied from lucide-static, with the closing tags written out.

## Colour, size and text
- Colours, radii and type sizes come from the preset's tokens through
  Tailwind classes. No hex or oklch values in a view.
- A new token in `input.css` needs a reason written next to it, as
  `--chart-1` and `--chart-2` have.
- Where the preset has a weakness (for example a faint input border), keep
  its value and say so, rather than change it.
- UI text is Swedish. Error texts come from the server's codes
  (`crates/web/src/errors.rs`).
- Amounts go through `format::amount`, or `overview::whole_kronor` for a
  headline figure. Dates are `YYYY-MM-DD`.

## Light and dark, wide and narrow
- Every view works in light and in dark mode. The tokens do that as long as
  nothing is hard-coded.
- Every view works at 390px without the page scrolling sideways: rows wrap,
  and a wide table scrolls inside its card.

## Tests keep it so
- `crates/web/src/app.rs` has tests that fail when a route is missing from
  the menu grouping, or from the `paths` list of "every signed-in view has
  one h1 and no page scrolls sideways" in `e2e/tests/design.spec.ts`. Add a
  new page to both.
- A view with a table, a form or a status gets its own assertions in
  `design.spec.ts` (the table sits in a card, the action is a button, the
  form is narrow), as the existing groups of views have.
- Header links live in menus: tests use `goTo` and `openMenu` from
  `e2e/tests/fixtures.ts`.

## Before a UI step is done
- Run it (`/verify`) and look at the view at 1280 and 390px, in light and
  in dark, next to the canvas.
- A design question that the canvas and the specs do not answer goes to the
  user, with a sketch on the canvas rather than a guess in the code.
