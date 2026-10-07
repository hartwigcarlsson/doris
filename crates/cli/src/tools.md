# Bookkeeping in Doris

A booked verifikation can never be changed or removed (Bokföringslagen). A
mistake costs a rättelse and a new voucher, visible forever. Every answer
is JSON; amounts are kronor strings ("1250.00").

## Booking a voucher
1. `list_accounts` and `list_fiscal_years`: the accounts exist and are
   active, the year is open.
2. Rehearse: `record_voucher` with `dry_run: true`. Continue only if the
   answer has `"dry_run": true` and no error.
3. Book: the identical call without `dry_run`. Note `number` and
   `fiscal_year_start`.
4. Underlag (receipts, invoices) cannot be sent here. Tell the owner which
   voucher numbers need one; they add it in Doris' web app.

## When a call fails
- `isError` with `error.code`: the books refused and nothing was saved.
  Fix the content and rehearse again. For `voucher_unbalanced`, recompute
  the lines yourself; ask the owner only when a fact is missing (which
  account, which VAT rate, a closed year).
- No answer at all after `record_voucher` or `correct_voucher`: the voucher
  may have been booked. Look for it with `list_vouchers` before any retry.
  Never retry a write blindly.

## "Delete" or "change" a voucher
There is no delete. `get_voucher` (check `corrected_by` is null), then
`correct_voucher` with today's date, rehearsed first, no later than the
year's end. Then book the right voucher as a new one. Tell the owner both
numbers stay in the books.

## Moms (25 %) and common BAS accounts
Gross × 0.8 = net, gross × 0.2 = VAT (12 %: ÷ 1.12; 6 %: ÷ 1.06).

| Purchase / sale | Debit | Credit |
|---|---|---|
| Office supplies, paid from bank/card | 6110 net, 2641 ingående moms | 1930 gross |
| Sale 25 %, paid to bank | 1930 gross | 3001 net, 2611 utgående moms |
| Rent (no VAT unless stated) | 5010 | 1930 |

Debits must add up to credits. Ingående moms (2641) is a debit, like the
cost; utgående moms (2611) is a credit, like the sale. Check the company's
chart (`list_accounts`) before using any account.
