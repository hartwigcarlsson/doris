# Plan 17: New Design, Part 3 (Search, Filters and History on Verifikationer) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Verifikationer gets a search field, two filters, newest-first order with "Visa fler", who booked a voucher and when, and account links to the huvudbok.

**Architecture:**
- `ListVouchers` also returns `recorded_at` and the recorder's display name, read from the `vouchers` projection joined with `users`. No event, migration or new RPC.
- Search, filters and paging are pure functions in a new `crates/web/src/voucher_search.rs`, over the year's vouchers the page already loads.
- `crates/web/src/pages/vouchers.rs` wires them in; `TableCard` gains the toolbar row Plan 15 postponed.

**Tech Stack:** Rust, sqlx 0.9 (SQLite), tonic 0.14, Leptos 0.8 CSR, Playwright 1.63.

**Spec:** `docs/superpowers/specs/2026-10-05-ny-design-verifikationssidan-design.md`. Visual reference: the page "Verifikationer" of https://claude.ai/artifact/FpdLiRfFggm3ZVSuM6nyUL.

## Global Constraints
- No events, commands, projections or migrations. The only proto change is two fields on `Voucher` (numbers 8 and 9).
- Only the display name leaves the server, never the email. Neither is logged.
- `list_vouchers` keeps its membership check first; a voucher is listed even when its recorder is not in `users` (empty name).
- Search and filters never call the server.
- The page size is 50; the order is highest number first.
- Amounts are öre (`i64`). Code, identifiers, comments and commits are English; UI text is Swedish.
- TDD: a failing test first. Each task ends green on `cargo test --workspace` (which runs `crates/ledger/tests/stress.rs`), `cargo clippy --workspace -- -D warnings`, `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`, and the Playwright suite (`make web && cargo build -p doris-server && cd e2e && npx playwright test`).
- After an `await`, a page reads its own signals with `try_get…` (AGENTS.md).
- Commits end with:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01JHpfH7CxMW86ZHVLURbK65
  ```

## Review Focus
1. **A voucher recorded by a user who is not in `users`** (or with an empty `recorded_by`): it must still be listed, with an empty name. Pinned in Task 1.
2. **A search word with regex or odd characters** (`(`, `*`, `å`, a lone `,`): no panic, plain substring matching. Pinned in Task 2.
3. **An amount typed the Swedish way** ("1 250,00" is two words: "1" and "250,00"): the rule must not silently match everything or nothing in a surprising way. Pinned in Task 2 with the documented outcome.
4. **A timestamp near midnight or New Year in another time zone**: the local date and time must roll over correctly. Pinned in Task 3.
5. **Changing the search while more than 50 rows are shown, then expanding a row**: the limit resets and the expanded row's content belongs to the row shown. Pinned in Task 4 (e2e "Visa fler" and search).

---

## File Structure
| File | Responsibility |
|---|---|
| `proto/doris/ledger/v1/ledger.proto` (modify) | `Voucher.recorded_at`, `Voucher.recorded_by_name`. |
| `crates/ledger/src/domain.rs` (modify) | `Recorded`, `Voucher.recorded`. |
| `crates/ledger/src/queries.rs` (modify) | `list_vouchers` reads when and who. |
| `crates/server/src/ledger.rs` (modify) | `voucher_message` fills the two fields. |
| `crates/web/src/voucher_search.rs` (create) | `Filter`, `matches`, `shown`. |
| `crates/web/src/format.rs` (modify) | `local_time`. |
| `crates/web/src/ui.rs` (modify) | `TableCard` toolbar. |
| `crates/web/src/pages/vouchers.rs` (modify) | The page. |
| `e2e/tests/voucher_search.spec.ts` (create), `e2e/tests/fixtures.ts` (modify) | Browser tests; `bookMany`. |
| `AGENTS.md` (modify) | API and Frontend sections. |

---

### Task 1: `ListVouchers` says who recorded a voucher and when

**Files:**
- Modify: `proto/doris/ledger/v1/ledger.proto`, `crates/ledger/src/domain.rs`, `crates/ledger/src/queries.rs`, `crates/server/src/ledger.rs`
- Test: `crates/ledger/tests/store.rs`, `crates/server/tests/ledger.rs`

**Interfaces:**
- Produces:
  - `pub struct Recorded { pub at: String, pub by: String }` (`Debug, Clone, PartialEq, Eq`) in `doris_ledger::domain`; `Voucher.recorded: Option<Recorded>`
  - proto `Voucher.recorded_at: String` (RFC 3339, UTC, as `events.recorded_at` stores it), `Voucher.recorded_by_name: String`

- [ ] **Step 1: Write the failing ledger tests**

Append to `crates/ledger/tests/store.rs`:

```rust
/// A row in identity's `users` projection, as registration writes it.
async fn user(pool: &SqlitePool, id: Uuid, email: &str, name: &str) {
    sqlx::query(
        "INSERT INTO users (user_id, email, display_name, role, registered_at)
         VALUES (?, ?, ?, 'member', '2025-01-01T00:00:00.000Z')",
    )
    .bind(id.to_string())
    .bind(email)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn a_listed_voucher_says_who_recorded_it_and_when() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    user(&pool, anna, "anna@example.se", "Anna Lind").await;
    user(&pool, bo, "bo@example.se", "Bo Ek").await;
    let id = company(&pool, anna).await;
    doris_company::add_member(&pool, id, anna, "bo@example.se").await.unwrap();
    let today = d(TODAY);
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), today).await.unwrap();
    record_voucher(&pool, id, bo, sale("2025-03-02", 200), today).await.unwrap();

    let listed = list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap();

    let recorded: Vec<_> = listed.iter().map(|v| v.recorded.clone().unwrap()).collect();
    assert_eq!(recorded[0].by, "Anna Lind");
    assert_eq!(recorded[1].by, "Bo Ek");
    // The event's own timestamp, as the projection stores it.
    let stored: Vec<String> = table(&pool, "SELECT recorded_at FROM vouchers ORDER BY number").await;
    assert_eq!(vec![recorded[0].at.clone(), recorded[1].at.clone()], stored);
    assert!(recorded[0].at.ends_with('Z') && recorded[0].at.contains('T'));
}

#[tokio::test]
async fn a_voucher_whose_recorder_is_unknown_is_still_listed() {
    let pool = db().await;
    // No row in `users` for anna: the store tests' usual setup.
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), d(TODAY)).await.unwrap();

    let listed = list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap();

    assert_eq!(listed.len(), 1);
    let recorded = listed[0].recorded.clone().unwrap();
    assert_eq!(recorded.by, "");
    assert!(!recorded.at.is_empty());
}
```

Check `doris_company::add_member`'s real signature first (`grep -n "pub async fn add_member" crates/company/src/lib.rs`) and how other tests in this file make a second member (`grep -n "bo" crates/ledger/tests/store.rs | head`); use the same call. If `users` has more NOT NULL columns in a later migration (`grep -n "ALTER TABLE users" migrations/*.sql`), add them to the helper.

The rebuild test `the_voucher_projections_rebuild_from_the_events` already compares `list_vouchers` before and after a rebuild; with `recorded` in `Voucher` it now covers "the rebuilt projection gives the same `recorded`" without a change.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p doris-ledger --test store a_listed_voucher a_voucher_whose_recorder`
Expected: does not compile (`Voucher` has no field `recorded`).

- [ ] **Step 3: Implement the ledger side**

`crates/ledger/src/domain.rs`, next to `Voucher`:

```rust
/// When a voucher was recorded and by whom: behandlingshistorik (BFL 5 kap.
/// 11 §). Read from the projection, so the domain's own state has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// RFC 3339, UTC: the event's `recorded_at`.
    pub at: String,
    /// The recorder's display name; empty if the user is unknown.
    pub by: String,
}
```

and in `Voucher`:

```rust
    /// Set by `list_vouchers`; `None` in the domain's state.
    pub recorded: Option<Recorded>,
```

Add `recorded: None` at every other place a `Voucher` is built (`grep -rn "Voucher {" crates/ --include=*.rs | grep -v "struct\|RecordVoucher\|pb::\|lpb::\|CorrectVoucher"`): `domain.rs` (`evolve`), `queries.rs` (the test at the bottom), and any test helper.

`crates/ledger/src/queries.rs`, `list_vouchers`:

```rust
    type Head = (u32, String, String, Option<u32>, Option<u32>, String, String);
    // …
    let heads: Vec<Head> = sqlx::query_as(
        // LEFT JOIN: a voucher is listed whether or not its recorder is
        // still (or ever was) in `users`.
        "SELECT v.number, v.date, v.text, v.corrects, v.corrected_by, v.recorded_at,
                COALESCE(u.display_name, '')
         FROM vouchers v LEFT JOIN users u ON u.user_id = v.recorded_by
         WHERE v.company_id = ? AND v.fiscal_year_start = ? ORDER BY v.number",
    )
```

and in the `.map`:

```rust
        .map(|(number, date, text, corrects, corrected_by, at, by)| Voucher {
            number,
            date: date.parse().expect("projected dates are valid"),
            text,
            lines: Vec::new(),
            corrects,
            corrected_by,
            attachments: Vec::new(),
            recorded: Some(Recorded { at, by }),
        })
```

Import `Recorded` in `queries.rs`.

- [ ] **Step 4: Run the ledger tests**

Run: `cargo test -p doris-ledger`
Expected: PASS, including `stress.rs` and the rebuild test.

- [ ] **Step 5: Write the failing server test**

In `crates/server/tests/ledger.rs`, the existing test around line 177 compares `vouchers[0]` with a whole `pb::Voucher { … }`. Change that literal to end with `recorded_at: vouchers[0].recorded_at.clone(), recorded_by_name: "Anna".into(),` — using the display name that test registers `anna` with (read the top of the test; if the helper registers her as something else, use that). Then add, right after the comparison:

```rust
    // RFC 3339 in UTC, and nothing of the email.
    assert!(vouchers[0].recorded_at.ends_with('Z'), "{}", vouchers[0].recorded_at);
    assert!(!format!("{:?}", vouchers[0]).contains('@'));
```

Do the same for any other whole-`pb::Voucher` comparison in `crates/server/tests` (`grep -rn "pb::Voucher {" crates/server/tests`).

- [ ] **Step 6: Run it and see it fail**

Run: `cargo test -p doris-server --test ledger`
Expected: does not compile (`pb::Voucher` has no field `recorded_at`).

- [ ] **Step 7: Implement the proto and the mapping**

`proto/doris/ledger/v1/ledger.proto`, in `message Voucher` after `attachments = 7;`:

```proto
  // When the voucher was recorded (RFC 3339, UTC) and by whom (the user's
  // display name; empty if unknown). Set by ListVouchers; empty elsewhere.
  string recorded_at = 8;
  string recorded_by_name = 9;
```

`crates/server/src/ledger.rs`, `voucher_message`:

```rust
fn voucher_message(v: Voucher) -> pb::Voucher {
    let (recorded_at, recorded_by_name) = v.recorded.map(|r| (r.at, r.by)).unwrap_or_default();
    pb::Voucher {
        number: v.number,
        date: v.date.to_string(),
        text: v.text,
        lines: v.lines.iter().map(line_message).collect(),
        corrects: v.corrects.unwrap_or(0),
        corrected_by: v.corrected_by.unwrap_or(0),
        attachments: v.attachments.iter().map(attachment_message).collect(),
        recorded_at,
        recorded_by_name,
    }
}
```

Every `lpb::Voucher { … }` literal in `crates/web` (tests in `overview.rs` use `..Default::default()`; check with `cargo test -p doris-web`) keeps compiling through `..Default::default()`; fix any that lists all fields.

- [ ] **Step 8: Run everything**

Run: `cargo test --workspace` and both clippy commands.
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add proto crates
git commit -m "Say who recorded a voucher and when in ListVouchers"
```

---

### Task 2: The search and filter rules

**Files:**
- Create: `crates/web/src/voucher_search.rs`
- Modify: `crates/web/src/main.rs` (`mod voucher_search;` after `mod ui;`)
- Test: `crates/web/src/voucher_search.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug, Default, PartialEq)] pub struct Filter { pub query: String, pub missing_attachment: bool, pub corrections: bool }`
  - `pub fn matches(voucher: &lpb::Voucher, accounts: &[lpb::Account], filter: &Filter) -> bool`
  - `pub fn shown(total: usize, limit: usize) -> usize`
  - `pub const PAGE: usize = 50;`

- [ ] **Step 1: Write the failing tests**

Create `crates/web/src/voucher_search.rs`:

```rust
//! Which vouchers the list shows for what is typed and ticked. Pure: the
//! page already holds the year's vouchers and the chart of accounts.

use crate::api::lpb;

#[cfg(test)]
mod tests {
    use super::*;

    fn voucher(number: u32, text: &str, lines: &[(u32, i64, i64)]) -> lpb::Voucher {
        lpb::Voucher {
            number,
            text: text.into(),
            lines: lines
                .iter()
                .map(|&(account, debit, credit)| lpb::VoucherLine { account, debit, credit })
                .collect(),
            ..Default::default()
        }
    }

    fn accounts() -> Vec<lpb::Account> {
        [(1930, "Företagskonto"), (3001, "Försäljning inom Sverige"), (5010, "Lokalhyra")]
            .into_iter()
            .map(|(number, name)| lpb::Account { number, name: name.into(), active: true })
            .collect()
    }

    fn sale() -> lpb::Voucher {
        voucher(21, "Försäljning Nordvik Bygg", &[(1930, 1_250_00, 0), (3001, 0, 1_250_00)])
    }

    fn found(voucher: &lpb::Voucher, query: &str) -> bool {
        matches(voucher, &accounts(), &Filter { query: query.into(), ..Default::default() })
    }

    #[test]
    fn an_empty_search_matches_everything() {
        assert!(found(&sale(), ""));
        assert!(found(&sale(), "   "));
    }

    #[test]
    fn text_matches_anywhere_and_ignores_case() {
        assert!(found(&sale(), "nordvik"));
        assert!(found(&sale(), "SÄLJ"));
        assert!(!found(&sale(), "hyra"));
    }

    #[test]
    fn every_word_must_match_something() {
        assert!(found(&sale(), "nordvik 1930"));
        assert!(found(&sale(), "bygg försäljning"));
        assert!(!found(&sale(), "nordvik hyra"));
    }

    #[test]
    fn a_number_matches_the_voucher_number_exactly() {
        assert!(found(&sale(), "21"));
        assert!(!found(&voucher(210, "x", &[]), "21"));
        assert!(!found(&voucher(121, "x", &[]), "21"));
    }

    #[test]
    fn a_number_matches_accounts_that_start_with_it() {
        assert!(found(&sale(), "1930"));
        assert!(found(&sale(), "19"));
        assert!(found(&sale(), "3"));
        assert!(!found(&sale(), "930"));
        assert!(!found(&sale(), "5010"));
    }

    #[test]
    fn an_account_name_matches() {
        assert!(found(&sale(), "företagskonto"));
        assert!(found(&sale(), "inom sverige"));
        // 5010 Lokalhyra is in the chart but not on this voucher.
        assert!(!found(&sale(), "lokalhyra"));
        // An account missing from the chart has no name to match, and no panic.
        assert!(!found(&voucher(1, "x", &[(9999, 1_00, 0)]), "företagskonto"));
    }

    #[test]
    fn an_amount_matches_whole_amounts_only() {
        assert!(found(&sale(), "1250"));
        assert!(found(&sale(), "1250,00"));
        assert!(found(&sale(), "1250.00"));
        assert!(found(&sale(), "1250,0"));
        assert!(!found(&sale(), "125"));
        assert!(!found(&sale(), "1250,01"));
        assert!(!found(&sale(), "250"));
        let small = voucher(7, "Bankavgift", &[(6570, 12_50, 0), (1930, 0, 12_50)]);
        assert!(found(&small, "12,50"));
        assert!(found(&small, "12,5"));
        assert!(!found(&small, "12"));
    }

    #[test]
    fn an_amount_typed_with_a_space_is_two_words() {
        // "1 250,00" asks for "1" and "250,00": neither is this voucher's
        // amount, and "1" is not its number. Documented, not clever.
        assert!(!found(&sale(), "1 250,00"));
        // The same words do match a voucher that has both.
        let both = voucher(1, "x", &[(1930, 250_00, 0), (3001, 0, 250_00)]);
        assert!(found(&both, "1 250,00"));
    }

    #[test]
    fn odd_characters_are_just_characters() {
        for query in ["(", "*", ".*", ",", ".", "å", "\\", "1,2,3", "--", "💰"] {
            // No panic, and nothing in the sale contains these.
            assert!(!found(&sale(), query), "{query}");
        }
        assert!(found(&voucher(1, "Hyra (mars)", &[]), "(mars)"));
    }

    #[test]
    fn missing_attachment_keeps_vouchers_without_underlag() {
        let filter = Filter { missing_attachment: true, ..Default::default() };
        let mut with = sale();
        with.attachments.push(lpb::Attachment::default());
        assert!(matches(&sale(), &accounts(), &filter));
        assert!(!matches(&with, &accounts(), &filter));
    }

    #[test]
    fn corrections_keeps_both_the_corrected_and_the_correction() {
        let filter = Filter { corrections: true, ..Default::default() };
        let (mut corrected, mut correction) = (sale(), sale());
        corrected.corrected_by = 22;
        correction.corrects = 21;
        assert!(matches(&corrected, &accounts(), &filter));
        assert!(matches(&correction, &accounts(), &filter));
        assert!(!matches(&sale(), &accounts(), &filter));
    }

    #[test]
    fn filters_and_search_all_have_to_hold() {
        let mut correction = sale();
        correction.corrects = 20;
        let filter = |query: &str| Filter { query: query.into(), missing_attachment: true, corrections: true };
        assert!(matches(&correction, &accounts(), &filter("nordvik")));
        assert!(!matches(&correction, &accounts(), &filter("hyra")));
        assert!(!matches(&sale(), &accounts(), &filter("nordvik")));
    }

    #[test]
    fn a_page_shows_the_limit_or_what_there_is() {
        assert_eq!(shown(0, PAGE), 0);
        assert_eq!(shown(49, PAGE), 49);
        assert_eq!(shown(50, PAGE), 50);
        assert_eq!(shown(51, PAGE), 50);
        assert_eq!(shown(51, 2 * PAGE), 51);
    }
}
```

Add `mod voucher_search;` to `crates/web/src/main.rs`. Check `lpb::VoucherLine`, `lpb::Account` and `lpb::Attachment` have exactly the fields used (`proto/doris/ledger/v1/ledger.proto`); use `..Default::default()` where a message has more.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p doris-web voucher_search::`
Expected: does not compile (`Filter`, `matches`, `shown`, `PAGE` missing).

- [ ] **Step 3: Implement**

```rust
use crate::format::parse_amount;

/// Rows shown before "Visa fler", and how many more each click adds.
pub const PAGE: usize = 50;

/// What is typed in the search field and ticked next to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub query: String,
    pub missing_attachment: bool,
    pub corrections: bool,
}

/// Whether `word` (lower case, no spaces) is found in the voucher: as its
/// number (exactly), an account on it (by prefix), a whole amount on it,
/// or a part of its text or of an account's name.
fn word_matches(word: &str, voucher: &lpb::Voucher, accounts: &[lpb::Account]) -> bool {
    if word.bytes().all(|b| b.is_ascii_digit()) {
        if word.parse() == Ok(voucher.number) {
            return true;
        }
        if voucher.lines.iter().any(|l| l.account.to_string().starts_with(word)) {
            return true;
        }
    }
    // "1250", "1250,00" and "1250.0" are amounts; the öre must agree too.
    if let Some(ore) = parse_amount(word) {
        let total: i64 = voucher.lines.iter().map(|l| l.debit).sum();
        if ore == total || voucher.lines.iter().any(|l| ore == l.debit || ore == l.credit) {
            return true;
        }
    }
    if voucher.text.to_lowercase().contains(word) {
        return true;
    }
    voucher.lines.iter().any(|line| {
        accounts
            .iter()
            .find(|a| a.number == line.account)
            .is_some_and(|a| a.name.to_lowercase().contains(word))
    })
}

/// Whether the list shows `voucher` under `filter`. Every word of the
/// search and every ticked box has to hold.
pub fn matches(voucher: &lpb::Voucher, accounts: &[lpb::Account], filter: &Filter) -> bool {
    if filter.missing_attachment && !voucher.attachments.is_empty() {
        return false;
    }
    if filter.corrections && voucher.corrects == 0 && voucher.corrected_by == 0 {
        return false;
    }
    filter
        .query
        .to_lowercase()
        .split_whitespace()
        .all(|word| word_matches(word, voucher, accounts))
}

/// How many of `total` matching rows are shown under `limit`.
pub fn shown(total: usize, limit: usize) -> usize {
    total.min(limit)
}
```

`parse_amount` accepts "12" as 1 200 öre: the test `!found(&small, "12")` holds because 12,00 is not on that voucher and 12 is not its number or an account prefix. An amount of 0 on a line (`debit: 0`) must not make "0" match every voucher: add the guard `ore != 0 &&` before the comparison, and the test

```rust
    #[test]
    fn zero_is_not_an_amount_to_search_for() {
        assert!(!found(&sale(), "0"));
        assert!(!found(&sale(), "0,00"));
    }
```

(write the test first, with the others).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-web voucher_search::` and both clippy commands. The module is unused until Task 4: put `#![allow(dead_code)]` at its top with `// Used by Verifikationer from the next commits on.`; Task 4 removes it.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src
git commit -m "Decide which vouchers a search and its filters show"
```

---

### Task 3: A recorded time in the reader's time zone

**Files:**
- Modify: `crates/web/src/format.rs`
- Test: `crates/web/src/format.rs`

**Interfaces:**
- Consumes: `day_number`, `plus_days` in `format.rs`.
- Produces: `pub fn local_time(utc: &str, offset_minutes: i32) -> String` — `"2026-10-02T12:12:45.123Z"` with offset 120 → `"2026-10-02 14:12"`; `""` if `utc` is not such a timestamp. `offset_minutes` is minutes east of UTC.

- [ ] **Step 1: Write the failing tests**

In `format.rs`'s test module:

```rust
    #[test]
    fn a_utc_time_is_shown_in_the_local_zone_without_seconds() {
        assert_eq!(local_time("2026-10-02T12:12:45.123Z", 120), "2026-10-02 14:12");
        assert_eq!(local_time("2026-10-02T12:12:45Z", 0), "2026-10-02 12:12");
        assert_eq!(local_time("2026-01-15T08:05:00.000Z", 60), "2026-01-15 09:05");
    }

    #[test]
    fn local_time_rolls_over_midnight_and_new_year() {
        assert_eq!(local_time("2026-10-02T23:30:00.000Z", 120), "2026-10-03 01:30");
        assert_eq!(local_time("2026-12-31T23:30:00.000Z", 60), "2027-01-01 00:30");
        assert_eq!(local_time("2027-01-01T00:30:00.000Z", -300), "2026-12-31 19:30");
        assert_eq!(local_time("2028-02-28T23:59:00.000Z", 60), "2028-02-29 00:59");
        assert_eq!(local_time("2026-03-01T00:00:00.000Z", -1), "2026-02-28 23:59");
    }

    #[test]
    fn a_time_that_is_not_one_is_empty() {
        for raw in ["", "2026-10-02", "nonsense", "2026-10-02T25:00:00Z", "2026-10-02T12:60:00Z", "2026-13-02T12:00:00Z", "2026-10-02 12:12"] {
            assert_eq!(local_time(raw, 120), "", "{raw}");
        }
    }
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p doris-web format::`
Expected: does not compile (`local_time` missing).

- [ ] **Step 3: Implement**

```rust
/// A UTC timestamp (`YYYY-MM-DDTHH:MM…Z`) as local `YYYY-MM-DD HH:MM`,
/// `offset_minutes` east of UTC. "" if it isn't one.
pub fn local_time(utc: &str, offset_minutes: i32) -> String {
    let parsed = (|| {
        let (date, time) = utc.split_once('T')?;
        if !time.ends_with('Z') || time.as_bytes().get(2) != Some(&b':') {
            return None;
        }
        let hour: i64 = time.get(..2)?.parse().ok()?;
        let minute: i64 = time.get(3..5)?.parse().ok()?;
        if hour > 23 || minute > 59 {
            return None;
        }
        // Minutes since midnight in the local zone, and the days that shifts.
        let minutes = hour * 60 + minute + i64::from(offset_minutes);
        let date = plus_days(date, minutes.div_euclid(1440))?;
        let minutes = minutes.rem_euclid(1440);
        Some(format!("{date} {:02}:{:02}", minutes / 60, minutes % 60))
    })();
    parsed.unwrap_or_default()
}
```

`plus_days` refuses a month over 12, which is what rejects "2026-13-02…".

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-web format::` and both clippy commands (allow dead code on the function with `#[allow(dead_code)] // used by Verifikationer in Task 5`, removed there).
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/format.rs
git commit -m "Format a recorded time in the reader's time zone"
```

---

### Task 4: The page: newest first, search, filters and "Visa fler"

**Files:**
- Modify: `crates/web/src/pages/vouchers.rs`, `crates/web/src/ui.rs`, `crates/web/src/voucher_search.rs` (drop the allow)
- Create: `e2e/tests/voucher_search.spec.ts`
- Modify: `e2e/tests/fixtures.ts`

**Interfaces:**
- Consumes: Task 2 (`Filter`, `matches`, `shown`, `PAGE`).
- Produces:
  - `TableCard` gains `#[prop(optional, into)] toolbar: Option<ViewFn>`: a row above the table.
  - e2e: `bookMany(page: Page, app: string, companyName: string, count: number, date: string): Promise<void>` in `fixtures.ts` — records `count` vouchers over gRPC-Web with the page's session.

- [ ] **Step 1: Write the e2e helper and the failing browser tests**

`fixtures.ts`: `bookMany` needs the company's id and a hand-encoded `RecordVoucherRequest`. The id is in the company page's URL (`/companies/{id}`), which `addCompany` lands on; change `addCompany` to return it:

```ts
  await expect(page.getByRole("heading", { name })).toBeVisible();
  return new URL(page.url()).pathname.split("/").pop()!;
```

(its return type becomes `Promise<string>`; callers that ignore it are unaffected), and add:

```ts
// Protobuf by hand, for the one request the tests send many of.
const varint = (n: number): number[] => (n < 0x80 ? [n] : [(n & 0x7f) | 0x80, ...varint(Math.floor(n / 128))]);
const bytes = (field: number, body: number[]) => [(field << 3) | 2, ...varint(body.length), ...body];
const text = (field: number, s: string) => bytes(field, [...Buffer.from(s, "utf8")]);
const uint = (field: number, n: number) => [field << 3, ...varint(n)];

/** Records `count` vouchers (1930 against 3001, 1 kr more each) over
 * gRPC-Web with the page's session: far quicker than the form. */
export async function bookMany(page: Page, app: string, companyId: string, count: number, date: string) {
  for (let i = 1; i <= count; i++) {
    const ore = i * 100;
    const message = [
      ...text(1, companyId),
      ...text(2, date),
      ...text(3, `Serie ${i}`),
      ...bytes(4, [...uint(1, 1930), ...uint(2, ore)]),
      ...bytes(4, [...uint(1, 3001), ...uint(3, ore)]),
    ];
    const frame = Buffer.from([0, ...[24, 16, 8, 0].map((s) => (message.length >>> s) & 0xff), ...message]);
    const response = await page.request.post(`${app}/doris.ledger.v1.LedgerService/RecordVoucher`, {
      headers: { "content-type": "application/grpc-web+proto", "x-grpc-web": "1" },
      data: frame,
    });
    const status = response.headers()["grpc-status"] ?? (await response.body()).toString("latin1").match(/grpc-status: ?(\d+)/)?.[1];
    if (status !== undefined && status !== "0") throw new Error(`RecordVoucher ${i} failed: grpc-status ${status}`);
  }
}
```

`page.request` shares the page's cookies, so the session goes with it. On success tonic-web puts `grpc-status` in a trailer frame in the body, not in a header, so `status` may be `undefined` in the header and `0` in the body: both are success. If the first call fails with `not_signed_in`, the cookie is `Secure` and Playwright's request context did not send it over http: send it by hand (`const cookie = (await page.context().cookies()).find((c) => c.name === "doris_session")`, header `cookie: doris_session=${cookie.value}`).

Create `e2e/tests/voucher_search.spec.ts`:

```ts
import type { Page } from "@playwright/test";
import { addCompany, bookMany, expect, goTo, register, test } from "./fixtures";

const year = new Date().getFullYear();
const rows = (page: Page) => page.getByRole("main").locator("tbody > tr").filter({ has: page.getByRole("button", { name: /visa konteringen/ }) });
const numbers = async (page: Page) => (await rows(page).getByRole("button", { name: /visa konteringen/ }).allInnerTexts()).map((t) => Number(t.trim()));

async function book(page: Page, app: string, text: string, debit: string, credit: string, kronor: string, file?: boolean) {
  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Datum").fill(`${year}-01-15`);
  await page.getByLabel("Text").fill(text);
  await page.getByLabel("Konto, rad 1").fill(debit);
  await page.getByLabel("Debet, rad 1").fill(kronor);
  await page.getByLabel("Konto, rad 2").fill(credit);
  await page.getByLabel("Kredit, rad 2").fill(kronor);
  if (file) await page.getByLabel("Underlag").setInputFiles([{ name: "kvitto.pdf", mimeType: "application/pdf", buffer: Buffer.from("%PDF-1.4\n%%EOF\n") }]);
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toBeVisible();
}

test("vouchers are listed newest first and found by text, account and amount", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, "Försäljning Nordvik", "1930", "3001", "1250", true);
  await book(page, app, "Hyra januari", "5010", "1930", "8000", true);
  await book(page, app, "Bankavgift", "6570", "1930", "12,50");
  await goTo(page, "Verifikationer");
  expect(await numbers(page)).toEqual([3, 2, 1]);
  await expect(page.getByRole("status")).toHaveText("Visar 3 av 3");

  const search = page.getByLabel("Sök bland verifikationer");
  await search.fill("nordvik");
  expect(await numbers(page)).toEqual([1]);
  await expect(page.getByRole("status")).toHaveText("Visar 1 av 1");
  await search.fill("5010");
  expect(await numbers(page)).toEqual([2]);
  await search.fill("12,50");
  expect(await numbers(page)).toEqual([3]);
  await search.fill("1930 hyra");
  expect(await numbers(page)).toEqual([2]);
  await search.fill("finns inte");
  await expect(page.getByRole("main")).toContainText("Inga verifikationer matchar.");
  await search.fill("");
  expect(await numbers(page)).toEqual([3, 2, 1]);
});

test("the filters keep vouchers without underlag, and corrections", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, "Med underlag", "1930", "3001", "100", true);
  await book(page, app, "Utan underlag", "1930", "3001", "200");
  await goTo(page, "Verifikationer");
  await rows(page).filter({ hasText: "Med underlag" }).getByRole("button", { name: "Rätta" }).click();
  await page.getByRole("button", { name: "Bekräfta rättelse" }).click();
  await expect(rows(page)).toHaveCount(3);

  await page.getByLabel("Saknar underlag").check();
  expect(await numbers(page)).toEqual([3, 2]);
  await page.getByLabel("Rättelser").check();
  expect(await numbers(page)).toEqual([3]);
  await page.getByLabel("Saknar underlag").uncheck();
  expect(await numbers(page)).toEqual([3, 1]);
});

test("the list shows fifty at a time", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  const id = await addCompany(page, app, "5560160680", "Exempel AB");
  await bookMany(page, app, id, 51, `${year}-01-15`);
  await goTo(page, "Verifikationer");
  await expect(page.getByRole("status")).toHaveText("Visar 50 av 51");
  await expect(rows(page)).toHaveCount(50);
  expect((await numbers(page))[0]).toBe(51);
  // A search starts from the top again, and an expanded row is the row shown.
  await page.getByRole("button", { name: "Visa fler" }).click();
  await expect(rows(page)).toHaveCount(51);
  await expect(page.getByRole("button", { name: "Visa fler" })).toHaveCount(0);
  await page.getByLabel("Sök bland verifikationer").fill("serie 7");
  expect(await numbers(page)).toEqual([7]);
  await rows(page).getByRole("button", { name: /visa konteringen/ }).click();
  await expect(page.getByRole("main").getByRole("row", { name: /^Summa 7,00 7,00$/ })).toBeVisible();
  await page.getByLabel("Sök bland verifikationer").fill("");
  await expect(page.getByRole("status")).toHaveText("Visar 50 av 51");
});
```

"serie 7" is two words: "serie" (text) and "7" (number 7 exactly, or amount 7,00): voucher 7 only — vouchers 17, 27, 37, 47 contain "7" in neither a matching way (their numbers are not 7, their amounts are 17,00 …, and no account starts with 7). Voucher 51 − nothing. Check the corrected-voucher flow's labels against `e2e/tests/ledger.spec.ts` ("Rätta", "Bekräfta rättelse") before running.

The second test assumes a correction carries no underlag and is dated today by default; if "Bekräfta rättelse" needs a date inside the year, it is prefilled (`ledger.spec.ts` shows how that flow is driven).

- [ ] **Step 2: Run them and see them fail**

Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test voucher_search.spec.ts`
Expected: FAIL — the order is `[1, 2, 3]`, and there is no search field.

- [ ] **Step 3: Implement**

`crates/web/src/ui.rs`:

```rust
/// A card around a `Table`, with an optional row above it for a search
/// field and filters.
#[component]
pub fn TableCard(#[prop(optional, into)] toolbar: Option<ViewFn>, children: Children) -> impl IntoView {
    view! {
        <section class="overflow-hidden rounded-lg bg-card px-2 py-2 text-xs/relaxed text-card-foreground ring-1 ring-foreground/10">
            {toolbar.map(|toolbar| view! { <div class="flex flex-wrap items-center gap-4 px-2 pt-2 pb-1">{toolbar.run()}</div> })}
            {children()}
        </section>
    }
}
```

`crates/web/src/pages/vouchers.rs`, in `Vouchers`:

```rust
    let filter = RwSignal::new(Filter::default());
    let query = RwSignal::new(String::new());
    let missing = RwSignal::new(false);
    let corrections = RwSignal::new(false);
    let limit = RwSignal::new(PAGE);
    Effect::new(move |_| {
        filter.set(Filter {
            query: query.get(),
            missing_attachment: missing.get(),
            corrections: corrections.get(),
        });
        // A new question starts from the top.
        limit.set(PAGE);
    });
```

Reset the search with the year and the company: in the existing effect that clears `vouchers` when `year` changes, also `limit.set(PAGE)`. (The search text itself stays when the year changes, so the same search can be tried in another year; it is cleared with the page.)

The rows shown: newest first, those that match, cut at the limit.

```rust
    // (company, fiscal year, vouchers) as loaded; the list shows the
    // matching ones, highest number first.
    let matching = Memo::new(move |_| {
        let (company_id, fiscal_year, mut list) = vouchers.get();
        let filter = filter.get();
        names.with(|accounts| list.retain(|v| matches(v, accounts, &filter)));
        list.sort_by_key(|v| std::cmp::Reverse(v.number));
        (company_id, fiscal_year, list)
    });
```

`Memo` needs `PartialEq` on the tuple: `lpb::Voucher` and `lpb::FiscalYear` derive it (prost). If the memo's clone of the list is a concern, keep it: the list is at most a year.

In the `<For>`, iterate `matching` cut at `limit`:

```rust
                        each=move || {
                            let (company_id, fiscal_year, list) = matching.get();
                            list.into_iter()
                                .take(limit.get())
                                .map(|v| (company_id.clone(), fiscal_year.clone(), v))
                                .collect::<Vec<_>>()
                        }
```

The `<For>` key already includes the voucher number, so a row that stays keeps its expanded state and a row that leaves loses it.

The toolbar, the empty states and the footer:

```rust
            <TableCard toolbar=move || view! {
                <div class="relative max-w-xs flex-[1_1_15rem]">
                    <Icon name=IconName::Search class="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                    <input
                        type="search"
                        aria-label="Sök bland verifikationer"
                        placeholder="Sök nummer, text, konto eller belopp"
                        class=format!("{INPUT} pl-7")
                        bind:value=query
                    />
                </div>
                <Checkbox label="Saknar underlag" id="missing_attachment" checked=missing />
                <Checkbox label="Rättelser" id="corrections" checked=corrections />
            }>
                <Table>…the existing table…</Table>
                {move || {
                    let total = matching.with(|(_, _, list)| list.len());
                    let loaded = vouchers.with(|(_, _, list)| list.len());
                    let visible = shown(total, limit.get());
                    view! {
                        <div class="flex flex-wrap items-center justify-between gap-4 px-2 pt-2 pb-1">
                            {if loaded == 0 {
                                view! { <p class="text-muted-foreground">"Inga verifikationer under räkenskapsåret."</p> }.into_any()
                            } else if total == 0 {
                                view! { <p class="text-muted-foreground">"Inga verifikationer matchar."</p> }.into_any()
                            } else {
                                view! { <p role="status" class="text-muted-foreground">{format!("Visar {visible} av {total}")}</p> }.into_any()
                            }}
                            {(visible < total).then(|| view! {
                                <Button variant=Variant::Outline kind="button" on:click=move |_| limit.update(|l| *l += PAGE)>"Visa fler"</Button>
                            })}
                        </div>
                    }
                }}
            </TableCard>
```

`INPUT` is private in `ui.rs`: make it `pub const INPUT`. Add `IconName::Search` (lucide-static 1.52.0 `search.svg`: fetch it with `curl -fsSL https://unpkg.com/lucide-static@1.52.0/icons/search.svg`, closing tags written out; add it to `ALL` and bump its length).

"Inga verifikationer under räkenskapsåret." must not flash while the list loads: the page sets `vouchers` to an empty list at the start of a load. If there is no "loaded" flag today, add `let loaded = RwSignal::new(false)`, set it `false` where the load starts and `true` when the answer is in, and show the empty-year text only when `loaded.get()`. Read the existing load effect first.

Remove `#![allow(dead_code)]` from `voucher_search.rs`.

- [ ] **Step 4: Run the new tests, then everything**

Run: `npx playwright test voucher_search.spec.ts`, then `cargo test --workspace`, both clippy commands and the whole Playwright suite.
Expected: PASS. Other specs that read voucher rows by position (`grep -n "nth(\|first()" e2e/tests/ledger.spec.ts e2e/tests/attachments.spec.ts e2e/tests/payroll.spec.ts e2e/tests/supplier_invoices.spec.ts e2e/tests/customer_invoices.spec.ts`) may assume oldest first: fix each by naming the row (`getByRole("row", { name: /^2 / })`) rather than by position, and say in the commit which ones changed.

- [ ] **Step 5: Commit**

```bash
git add crates/web e2e/tests
git commit -m "Search, filter and page the voucher list, newest first"
```

---

### Task 5: Behandlingshistorik and account links in the expanded row

**Files:**
- Modify: `crates/web/src/pages/vouchers.rs`, `crates/web/src/format.rs` (drop the allow)
- Modify: `e2e/tests/voucher_search.spec.ts`

**Interfaces:**
- Consumes: Task 1 (`recorded_at`, `recorded_by_name`), Task 3 (`local_time`).

- [ ] **Step 1: Write the failing browser test**

Append to `e2e/tests/voucher_search.spec.ts`:

```ts
test("an expanded voucher says who booked it and when, and links its accounts", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna Lind" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, "Försäljning", "1930", "3001", "1250");
  await goTo(page, "Verifikationer");
  await rows(page).getByRole("button", { name: /visa konteringen/ }).click();
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { name: "Behandlingshistorik" })).toBeVisible();
  // Today, in the browser's zone, to the minute.
  const now = new Date();
  const today = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
  await expect(main.getByText(new RegExp(`^Bokförd ${today} \\d\\d:\\d\\d av Anna Lind$`))).toBeVisible();
  await main.getByRole("link", { name: /^1930 / }).click();
  await expect(page).toHaveURL(new RegExp(`/trial-balance/1930\\?fy=${year}-01-01$`));
  await expect(page.getByRole("heading", { level: 1, name: /^1930 / })).toBeVisible();
});
```

A run that starts just before midnight and expands the row just after would see yesterday's date: accept either day by building the pattern from `today` and the day before only if this proves flaky; do not weaken it up front.

- [ ] **Step 2: Run it and see it fail**

Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test voucher_search.spec.ts -g "who booked"`
Expected: FAIL (no "Behandlingshistorik" heading).

- [ ] **Step 3: Implement**

In `VoucherRow`, before the `view!`:

```rust
    // Minutes east of UTC; JS gives minutes west.
    let offset = -(js_sys::Date::new_0().get_timezone_offset() as i32);
    let recorded = local_time(&voucher.recorded_at, offset);
    let history = match (recorded.is_empty(), voucher.recorded_by_name.is_empty()) {
        (true, _) => None,
        (false, true) => Some(format!("Bokförd {recorded}")),
        (false, false) => Some(format!("Bokförd {recorded} av {}", voucher.recorded_by_name)),
    };
    let ledger_year = fiscal_year.get_value().map(|y| y.start).unwrap_or_default();
```

(`fiscal_year` is the `StoredValue` the row already has; take `ledger_year` before it is moved.)

The account cell of the kontering table becomes a link:

```rust
<td class="py-1.5 pr-2">
    <A href=format!("/trial-balance/{}?fy={ledger_year}", l.account) attr:class="underline-offset-4 hover:underline">
        {format!("{} {}", l.account, name)}
    </A>
</td>
```

and under the underlag block, inside the same right-hand column:

```rust
{history.clone().map(|line| view! {
    <div class="grid gap-1">
        <h2 class="text-xs/relaxed font-medium">"Behandlingshistorik"</h2>
        <p>{line}</p>
    </div>
})}
```

Read the expanded row's current markup first: Plan 15 put the kontering table and the underlag in a `flex flex-wrap` row; the history goes in the underlag's column (wrap the two in one `<div class="grid gap-3">` if they are not already).

The e2e test in `design.spec.ts` "an expanded voucher shows its kontering…" reads the account cell's text (`/^1930 /`): a link inside the cell keeps the text. Remove the `#[allow(dead_code)]` on `local_time`.

- [ ] **Step 4: Run everything**

Run: the new test, then `cargo test --workspace`, both clippy commands and the whole Playwright suite.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web e2e/tests
git commit -m "Show who booked a voucher and when, and link its accounts"
```

---

### Task 6: Docs and the check against the canvas

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Update `AGENTS.md`**

In "## API", in the `LedgerService` bullets, add:

```markdown
- `ListVouchers` also says when each voucher was recorded and by whom
  (`recorded_at`, `recorded_by_name`: the display name, read from `users`;
  empty if the user is unknown). Never the email.
```

In "## Frontend", after the overview bullet:

```markdown
- Verifikationer lists the year's vouchers newest first, fifty at a time.
  The search field and the "Saknar underlag"/"Rättelser" boxes filter in
  the browser (`src/voucher_search.rs`): every word must match the number
  (exactly), an account on the voucher (by prefix), a whole amount, or part
  of the text or of an account's name.
```

- [ ] **Step 2: Run everything and measure**

Run: `cargo test --workspace`, both clippy commands, the whole Playwright suite; `make dist && ls -l crates/web/dist/*_bg.wasm` (before this plan: measure on the base branch first if the number is not known).

- [ ] **Step 3: Check against the canvas**

Run the `verify` skill. With two users (the second invited and added as a member, so that a voucher booked by each exists), a dozen vouchers of which one is corrected and two lack underlag: open Verifikationer at 1280 and 390px, light and dark, and compare with the artboard "Verifikationer": the toolbar row, the order, the badges, the expanded row (kontering, underlag, history with the other member's name), the footer. Search, tick both boxes, "Visa fler" is not reachable with a dozen rows (covered by the e2e test). Also check with `sqlite3` that no email appears in what `ListVouchers` returns (the server test asserts it; this is the runtime look). Report differences; fix those that contradict the spec, each with a failing test first.

- [ ] **Step 4: Commit**

```bash
git add AGENTS.md
git commit -m "Document search and history on Verifikationer"
```
