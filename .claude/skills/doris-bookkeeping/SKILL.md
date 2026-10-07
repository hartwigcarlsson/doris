---
name: doris-bookkeeping
description: Use when doing a Swedish company's bookkeeping in Doris through doris-cli — booking a verifikation, a receipt or sale with moms, correcting or "deleting" a voucher, reading saldobalans/huvudbok — or when a doris-cli command exits non-zero.
---

# Bookkeeping with doris-cli

## Overview
A booked verifikation can never be changed or removed (Bokföringslagen); a
mistake costs a rättelse and a new voucher, visible forever. So every write
is rehearsed with `--dry-run`, every answer is read as JSON, and every
failure is classified by its exit code before anything is retried.

Commands, flags and JSON shapes: `crates/cli/README.md` and `doris-cli --help`.
Use only commands `--help` lists; there is no other way in (e.g. no command adds
underlag to a booked voucher).

## Commands
```
doris-cli --json auth status | company list | year list | account list
doris-cli --json ver list [--year 2026] | ver view N [--year 2026]
doris-cli --json [--dry-run] ver new --date D --text T --debit K=B … --credit K=B … [--attach FIL]
doris-cli --json [--dry-run] ver correct N --date D [--year 2026]
doris-cli --json report trial-balance | ledger KONTO | statements [--year 2026]
```
Global flags (`--json`, `--dry-run`, `--company`) go anywhere; `--year` goes after
the action (`ver list --year 2026`) and is the year the räkenskapsår starts (or
its start date). Field names in the JSON answers: see the README.

## Booking a voucher
1. `doris-cli --json account list` and `year list`: the accounts exist and are active, the year is open.
2. Rehearse: `doris-cli --json --dry-run ver new --date … --text … --debit KONTO=BELOPP … --credit KONTO=BELOPP … [--attach FIL]`.
   Continue only if exit 0 **and** `"dry_run": true`.
3. Book: the identical command without `--dry-run`. Note `number` and `fiscal_year_start`.
4. The underlag (receipt, invoice) goes on the voucher itself: `--attach` on
   `ver new`, in the rehearsal and the booking alike. It cannot be added later.

## Reading a failure
| Exit | Meaning | Do |
|---|---|---|
| 1 | The books refused (`error.code`). Nothing was saved — voucher and underlag are one transaction — except `dry_run_unsupported` | Fix the content from the command you sent: re-add debits vs credits, VAT line, account, date. Rehearse again. |
| 1 `dry_run_unsupported` | The server is too old: the voucher **was booked for real** as `error.number` | Don't book again. `ver view` it; if it was what you meant, keep it; if not, `ver correct` it. Tell the owner. |
| 2 | Usage: your arguments | Fix the flags. |
| 3 | Token, URL or connection | After a write, the voucher **may have been booked**. Run `ver list` and look for it before any retry. Never retry a write blindly. |

## "Delete" or "change" a voucher
There is no delete. `ver view N` (check `corrected_by` is null), then
`ver correct N --date <today>` (rehearsed first). The rättelse is dated the day it
is made — take it from `date +%F`, don't guess — and no later than the year's end. Then book
the right voucher as a new one. Tell the owner both numbers stay in the books.

## Moms (25 %) and common BAS accounts
Gross × 0.8 = net, gross × 0.2 = VAT (12 %: ÷ 1.12; 6 %: ÷ 1.06).

| Purchase / sale | Debit | Credit |
|---|---|---|
| Office supplies, paid from bank/card | 6110 net, 2641 ingående moms | 1930 gross |
| Sale 25 %, paid to bank | 1930 gross | 3001 net, 2611 utgående moms |
| Rent (no VAT unless stated) | 5010 | 1930 |

Before sending, the --debit amounts must add up to the --credit amounts. The
receipt "office supplies 1 000 kr incl. 25 % moms, paid by card" in full:
```
doris-cli --json --dry-run ver new --date 2026-02-02 --text "Kontorsmaterial Clas Ohlson" \
  --debit 6110=800 --debit 2641=200 --credit 1930=1000 --attach kvitto.pdf
```
Ingående moms (2641) is a debit, like the cost; utgående moms (2611) is a
credit, like the sale. Check the company's chart (`account list`) before using
any account.

## Never
- Print `DORIS_TOKEN`, not even a prefix; it never goes in output, logs or messages.
- Book without `--dry-run` first, or skip checking `"dry_run": true` in the rehearsal.
- Treat exit 3 as "nothing happened".
- Ask the owner about `voucher_unbalanced`: you built the voucher, so recompute it yourself. Ask only when a fact is missing (which account, which VAT rate, a closed year).
