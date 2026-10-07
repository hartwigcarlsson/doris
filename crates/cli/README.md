# doris-cli

The command line for Doris, for people and AI agents. It talks to the same
gRPC-Web API as the web app, with an API token. The help texts are Swedish;
the JSON field names and error codes are English.

## Install

```
cargo install --path crates/cli     # or: make dist  ->  target/dist/doris-cli
```

## Settings

| Variable | |
|---|---|
| `DORIS_TOKEN` | API token (required). Never printed or logged. |
| `DORIS_URL` | The server (required): `https://…`, or `http://` only for localhost. Only scheme, host and optional port: a path is not supported. |
| `DORIS_COMPANY` | Organisation number or id (optional). Default: the token's only company; with several, the command fails with `company_ambiguous` (exit 2) and lists them. |

## Global flags

- `--json`: exactly one JSON value on stdout, errors included.
- `--company <ORGNR|ID>`: overrides `DORIS_COMPANY`.
- `--dry-run`: checked on the server. `ver new` and `ver correct` run every
  rule in the real transaction and roll back, so nothing is saved and the
  call does not count as the token's use. On read commands it changes
  nothing and only adds `"dry_run": true` to object outputs; lists stay
  arrays.
- Every area takes `--year` (a year such as `2026`, or a start date; default
  the fiscal year containing today in Sweden), except `ver new`, which
  ignores it: the date decides the year.

Amounts in JSON are kronor strings (`"1250.00"`); text mode shows `1 250,00`.

## Exit codes and errors

| Exit | Meaning |
|---|---|
| 0 | Done |
| 1 | The server refused (a rule, missing scope, not found) |
| 2 | Usage: fix the arguments (also `company_ambiguous`) |
| 3 | Token, URL or connection: `missing_token`, `missing_url`, `insecure_url`, `not_signed_in`, `connection_failed` |

With `--json` a failure is `{"error":{"code":"voucher_unbalanced","message":"…"}}`
on stdout; without it the Swedish message goes to stderr. Codes are the
server's stable codes plus the client's own: `usage`, `missing_token`,
`missing_url`, `insecure_url`, `connection_failed`, `internal`.

## Commands

Each example shows text mode, then the `--json` form.

### `auth status`
```
$ doris-cli auth status
Anna Andersson <anna@example.se>
{"name":"Anna Andersson","email":"anna@example.se"}
```

### `company list`, `company view`
```
556016-0680  Exempel AB
[{"id":"…","org_nr":"556016-0680","name":"Exempel AB"}]
{"id","org_nr","name","legal_form","accounting_method","fiscal_year_start","fiscal_year_end"}
```

### `year list`
```
2026-01-01 – 2026-12-31  Öppet
[{"start":"2026-01-01","end":"2026-12-31","closed":false}]
```

### `account list`
```
1930  Företagskonto
[{"number":1930,"name":"Företagskonto","active":true}]
```

### `ver list [--year]`
Newest first.
```
   1  2026-02-02     1 000,00  Inköp
[{"number":1,"date":"2026-02-02","text":"Inköp","total":"1000.00","corrects":null,
  "corrected_by":null,"attachments":0,"recorded_at":"…","recorded_by":"Anna Andersson"}]
```

### `ver view NUMBER [--year]`
```
{"fiscal_year_start","number","date","text",
 "lines":[{"account":6110,"debit":"800.00","credit":"0.00"}],
 "corrects","corrected_by",
 "attachments":[{"file_name","sha256","size","content_type"}],
 "recorded_at","recorded_by"}
```

### `ver new`
```
doris-cli ver new --date 2026-02-02 --text Inköp --debit 6110=800 --debit 2641=200 --credit 1930=1000 [--attach kvitto.pdf]
Verifikation 1 i räkenskapsåret 2026 bokförd (2026-02-02, 1 000,00 kr, 0 underlag).
{"dry_run":false,"fiscal_year_start":"2026-01-01","number":1,"date","text","lines":[…],"attachments":[…]}
```
`--dry-run` answers "Skulle bokföras som verifikation N …" and saves
nothing. `--input FILE|-` takes the whole voucher as JSON
(`{"date","text","lines":[{"account","debit","credit"}],"attachments":["path"]}`;
amounts as strings or numbers). Attachment paths in `--input` are relative to
the current directory. Underlag are PDF, JPEG or PNG, at most 10 MiB each and
20 MiB together. `--year` is ignored.

### `ver correct NUMBER --date DATE [--year]`
```
Verifikation 1 rättad med verifikation 2.
{"dry_run":false,"fiscal_year_start":"2026-01-01","number":2,"corrects":1}
```

### `report trial-balance [--year]`
```
Konto  Namn                                    IB         Debet        Kredit            UB
1930   Företagskonto                         0,00      1 250,00          0,00      1 250,00
{"fiscal_year_start":"2026-01-01","rows":[{"account":1930,"name":"Företagskonto",
  "opening":"0.00","debit":"1250.00","credit":"0.00","closing":"1250.00"}]}
```
`closing` is `opening + debit − credit`.

### `report ledger ACCOUNT [--year]`
```
Ingående balans 0,00
2026-02-02     1      1 250,00          0,00      1 250,00  Försäljning
{"fiscal_year_start","account":1930,"opening":"0.00",
 "entries":[{"date","number","text","debit","credit","balance"}]}
```

### `report statements [--year]`
Resultat- och balansräkning, with the year before as comparison.
```
{"fiscal_year_start","previous_fiscal_year_start":null,
 "income_statement":[{"label":"Nettoomsättning","kind":"item","amount":"1000.00","previous":null}],
 "balance_sheet":[…],"difference":"0.00"}
```
`kind` is `heading` (`amount` null), `item` or `subtotal`. `previous` and
`previous_fiscal_year_start` are `null` when there is no earlier year.
