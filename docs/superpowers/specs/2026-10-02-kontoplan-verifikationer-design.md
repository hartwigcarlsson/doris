# Doris – Steg 4: Kontoplan och verifikationer

## Kontext
Doris har användare, företag och ett aktivt företag, men ingen bokföring. Det här steget ger varje företag en kontoplan och låter medlemmarna bokföra verifikationer i en grundbok. Verifikationerna får löpnummer utan luckor per räkenskapsår, och de rättas bara med nya verifikationer (BFL 5 kap.).

### Fattade beslut
| Område | Beslut |
|---|---|
| Crate | Ny crate `crates/ledger` (`doris-ledger`) som beror på `doris-company`. |
| Strömmar | `accounts-{company_id}` för kontoplanen och `ledger-{company_id}-{fy_start}` för verifikationerna, med en ström per räkenskapsår. `fy_start` skrivs `YYYY-MM-DD`. |
| Kontoplan | Ett urval av cirka 150–250 vanliga BAS-konton följer med binären. Användaren kan lägga till konton, byta namn på dem och inaktivera eller aktivera dem. Konton tas aldrig bort. |
| Seedning | Lat. Kontoplanen seedas vid företagets första skrivning till kontoplanen eller grundboken, i samma transaktion som kommandot. |
| Numrering | Löpnumret sätts av `decide` som `last_number + 1` inom skrivtransaktionen. Klienten skickar aldrig ett nummer. Det finns en serie, och inga utkast. |
| Rättelse | Vändning med referens. En rättelse skapar en ny verifikation med debet och kredit omkastade och `corrects` satt till originalets nummer. |
| Belopp | Heltal i öre (`i64`), i domänen, i JSON och på tråden. |

## Domän (`crates/ledger/src/domain.rs`, ren och utan I/O)

### Kontoplan
- **Konto:** nummer med fyra siffror, 1000–8999. Namn med 1–100 tecken efter trim. Kontot är aktivt eller inaktivt.
- **Event:**
  - `ChartSeeded { accounts: [{number, name}] }` innehåller hela listan och inte en hänvisning till en BAS-version. Historiken förblir då läsbar och påverkas inte av att urvalet i binären ändras.
  - `AccountAdded { number, name }`
  - `AccountRenamed { number, name }`
  - `AccountDeactivated { number }`
  - `AccountReactivated { number }`
- **Regler:**
  - Ett konto som redan finns kan inte läggas till (`account_exists`).
  - Byta namn, inaktivera och aktivera kräver ett konto som finns (`account_not_found`).
  - Att inaktivera ett inaktivt konto, eller aktivera ett aktivt, är ett no-op som inte ger något event.
- **BAS-urvalet:** ligger i `crates/ledger/src/bas.rs` som en konstant lista `(u16, &str)`.

### Verifikationer
- **Event:** `VoucherRecorded { number, date, text, lines: [{account, debit, credit}], corrects: Option<u32> }`. Vem som bokförde och när står i event-metadatan, enligt 5 kap. 11 §.
- **Tillstånd per ström** (via `evolve`): `last_number`, de verifikationer som finns, vilka som är rättelser och vilka som är rättade.
- **`RecordVoucher { date, text, lines }`** måste uppfylla följande:
  - Datumet får inte ligga efter dagens datum (`voucher_date_in_future`) och inte före första räkenskapsårets start (`voucher_date_before_first_fiscal_year`). Räkenskapsåret, och därmed strömmen, bestäms med `FiscalYear::containing(date)`.
  - Texten har 1–200 tecken efter trim (`invalid_voucher_text`).
  - Verifikationen har 2–100 rader (`invalid_voucher_lines`).
  - På varje rad är exakt ett av debet och kredit större än 0, och det andra är 0. Beloppet är högst 10¹³ öre (`invalid_amount`).
  - Varje konto finns och är aktivt (`account_not_found`, `account_inactive`).
  - Summan av debet är lika med summan av kredit (`voucher_unbalanced`). Summorna räknas med kontrollerad aritmetik.
  - Resultatet blir `number = last_number + 1`.
- **`CorrectVoucher { number, date }`:**
  - Verifikationen ska finnas (`voucher_not_found`), får inte redan vara rättad (`already_corrected`) och får inte själv vara en rättelse (`cannot_correct_correction`).
  - Datumet ska ligga i originalets räkenskapsår och inte efter dagens datum (`correction_date_outside_fiscal_year`, `voucher_date_in_future`).
  - Resultatet är en ny verifikation i samma ström. Raderna har debet och kredit omkastade, texten är `Rättelse av ver {N}` och `corrects` är `Some(N)`.
  - Kontona kontrolleras inte mot aktiv-status, så en vändning går alltid att göra.

## Lagring och transaktioner (`crates/ledger/src/lib.rs`)

### Skrivflöde (alla kommandon)
1. `doris_eventstore::begin` (`BEGIN IMMEDIATE`).
2. Ladda företaget i samma transaktion. `doris-company` exporterar för det en funktion som laddar ett företag på en `&mut SqliteConnection`. Om användaren inte är medlem blir svaret `company_not_found`.
3. Ladda `accounts-…`. Om strömmen är tom läggs `ChartSeeded` till först.
4. Vid verifikationer: ladda `ledger-…` för räkenskapsåret och kör `evolve` och sedan `decide`.
5. `append` med `expected_version`. Projektionerna uppdateras i samma transaktion.
6. Commit. Vid fel rullas allt tillbaka, och inget nummer har förbrukats.

Varje kommando finns i två former: `record_voucher_in(&mut tx, …)` gör steg 2–5, och `record_voucher(pool, …)` är `begin` + `…_in` + `commit`. Samma uppdelning gäller `correct_voucher` och kontoplanens kommandon.

### Projektioner (`migrations/0006_ledger.sql`)
- `accounts(company_id, number, name, active)`, med primärnyckeln `(company_id, number)`.
- `vouchers(company_id, fiscal_year_start, number, date, text, corrects, corrected_by, recorded_at, recorded_by)`, med primärnyckeln `(company_id, fiscal_year_start, number)`.
- `voucher_lines(company_id, fiscal_year_start, number, line_no, account, debit, credit)`, med primärnyckeln `(company_id, fiscal_year_start, number, line_no)`.
- **Trigger** på `INSERT` i `vouchers`: anropet avbryts om `NEW.number != COALESCE(MAX(number), 0) + 1` för samma `company_id` och `fiscal_year_start`. Tillsammans med primärnyckeln gör det att databasen själv stoppar luckor och dubbletter.
- `rebuild_projections` läser `read_all` i `global_position`-ordning och bygger om alla tre tabellerna.

### Läsningar (`queries.rs`)
- `list_accounts`: tom projektion för företaget betyder att kontoplanen inte är seedad, och då returneras BAS-urvalet från binären.
- `list_vouchers(company_id, fy_start)` returnerar verifikationerna med rader, sorterade på nummer.
- `list_fiscal_years(company_id)` går från första räkenskapsåret med `FiscalYear::next` till året som innehåller dagens datum, och returnerar det nyaste först.

## API (`proto/doris/ledger/v1/ledger.proto`, `LedgerService`)
Varje anrop kräver en session och har ett `company_id`. Den som inte är medlem får `NOT_FOUND "company_not_found"`. Datum skrivs `YYYY-MM-DD` och belopp är `int64` i öre.

| RPC | In | Ut |
|---|---|---|
| `ListAccounts` | — | konton (nummer, namn, aktiv) |
| `AddAccount` | `number`, `name` | — |
| `RenameAccount` | `number`, `name` | — |
| `SetAccountActive` | `number`, `active` | — |
| `ListFiscalYears` | — | räkenskapsår (`start`, `end`), nyaste först |
| `RecordVoucher` | `date`, `text`, `lines` | `fiscal_year_start`, `number` |
| `CorrectVoucher` | `fiscal_year_start`, `number`, `date` | `number` |
| `ListVouchers` | `fiscal_year_start` | verifikationer med rader, `corrects`, `corrected_by` |

`ListVouchers` sidindelas inte. Det markeras med en `ponytail:`-kommentar att paginering behövs när ett år blir för stort.

Domänfelen mappas i `crates/server/src/ledger.rs` till stabila snake_case-koder:
- `INVALID_ARGUMENT` för `invalid_account_number`, `invalid_account_name`, `invalid_voucher_text`, `invalid_voucher_lines`, `invalid_amount`, `voucher_unbalanced`, `voucher_date_in_future`, `voucher_date_before_first_fiscal_year` och `correction_date_outside_fiscal_year`.
- `ALREADY_EXISTS` för `account_exists`.
- `NOT_FOUND` för `account_not_found` och `voucher_not_found`.
- `FAILED_PRECONDITION` för `account_inactive`, `already_corrected` och `cannot_correct_correction`.

Varje kod får en svensk text i `crates/web/src/errors.rs`.

## Frontend
- **Navigering:** i sidhuvudet läggs länkarna "Verifikationer" (`/vouchers`) och "Kontoplan" (`/accounts`) till. De gäller det aktiva företaget från `Companies`-kontexten.
- **Byte i andra flikar:** `active_company.rs` lyssnar på `storage`-händelsen, så att en öppen flik byter aktivt företag direkt när valet ändras i en annan flik. Det var utanför omfattningen i steg 3 men behövs nu, så att en gammal flik inte bokför i fel företag.
- **`/accounts`, Kontoplan:**
  - En tabell med kolumnerna Konto, Namn och Status.
  - Per rad finns "Byt namn" (redigering i raden) och "Inaktivera"/"Aktivera".
  - Ett formulär "Lägg till konto" med fälten nummer och namn.
  - Kryssrutan "Visa inaktiva" är förvalt av.
- **`/vouchers`, Verifikationer (grundboken):**
  - Räkenskapsåret väljs i en Select som fylls från `ListFiscalYears`. Förvalt är det nyaste året.
  - En tabell med kolumnerna Nr, Datum, Text, Belopp (summan av debet) och Status ("Rättad av ver N" eller "Rättelse av ver N"). Ett klick på en rad visar konteringsraderna.
  - "Rätta" visar ett datumfält direkt i raden, förvalt idag eller räkenskapsårets sista dag om året är slut, och knappen "Bekräfta rättelse".
- **`/vouchers/new`, Ny verifikation:**
  - Fälten Datum (`<input type="date">`, förvalt idag) och Text.
  - Raderna har Konto, Debet och Kredit. Kontofältet är en `<input>` med `<datalist>` över aktiva konton i formen "1930 Företagskonto". Knapparna "Lägg till rad" och "Ta bort".
  - En levande summering, "Debet · Kredit · Differens", visas bara som hjälp.
  - Formuläret har `novalidate`, och felen kommer från serverns koder.
  - När verifikationen är bokförd visas "Verifikation N bokförd" och formuläret töms.
- **Belopp:** `format.rs` får två funktioner.
  - `parse_amount(&str) -> Option<i64>` returnerar öre. Den accepterar mellanslag och hårt mellanslag som tusentalsavgränsare och både komma och punkt som decimaltecken, och avvisar fler än två decimaler.
  - `amount(i64) -> String` ger till exempel `1 234,50`.
- **Komponenter:** `ui.rs` får `Table` med klasser från shadcn-presetet. Inga nya beroenden tillkommer, och wasm-budgeten (800 KB) gäller.

## Tester
- **Domän** (`crates/ledger/tests/domain.rs`, given/when/then):
  - Numrering: en första verifikation får nummer 1, och givet 1–3 blir nästa 4.
  - Varje felkod ovan har ett eget fall.
  - Rättelse: raderna vänds och `corrects` sätts. Vändning fungerar även när kontot har blivit inaktivt.
  - Kontoplanens kommandon och regeln om no-op.
- **Lagring** (`crates/ledger/tests/store.rs`):
  - Lat seedning: en läsning sparar inget, och första skrivningen sparar `ChartSeeded` och kommandot i samma transaktion.
  - Den som inte är medlem får `company_not_found`.
  - Ombyggnad från `read_all` ger identiska tabeller.
  - Triggern avvisar en `INSERT` med ett hoppat nummer.
- **Stresstest** (`crates/ledger/tests/stress.rs`, i vanliga `cargo test`):
  - Testet körs mot en riktig SQLite-fil (`tempfile`, WAL, poolens 8 anslutningar) och ett enda företag.
  - 32 tokio-tasks gör 50 operationer var. Datumen sprids över tre räkenskapsår.
  - Operationerna väljs deterministiskt efter index, utan `rand`:
    - de flesta är giltiga bokningar
    - var 7:e är ogiltig (obalans eller inaktivt konto) och ska ge fel
    - var 5:e anropar `record_voucher_in` och släpper sedan transaktionen utan commit
    - en del är rättelser, där flera tasks rättar samma verifikation samtidigt.
  - Efteråt kontrolleras att följande gäller, både före och efter `rebuild_projections`:
    1. Per räkenskapsår är numren i `VoucherRecorded` från `read_all` exakt `1..=n`.
    2. `n` är lika med antalet lyckade commits som testet räknat.
    3. Projektionen stämmer med eventen, och varje verifikation balanserar.
    4. Varje verifikation som rättats samtidigt har exakt en rättelse, och de andra försöken fick `already_corrected`.
  - `SQLITE_BUSY` räknas som testfel.
- **Server** (`crates/server/tests/ledger.rs`, gRPC-Web): kontoplanens RPC:er, bokföring, lista och rättelse, att felkoderna kommer fram, och att en annan användare får `company_not_found`.
- **Playwright** (`e2e/tests/ledger.spec.ts`):
  - Bokföra 1930 mot 3001 och se verifikation 1 i grundboken.
  - Rätta den och se "Rättad av ver 2" på ver 1 och "Rättelse av ver 1" på ver 2.
  - Lägga till ett konto och använda det.
  - En obalanserad verifikation ger det svenska felet.

## Utanför omfattningen
Låsta räkenskapsår och bokslut, flera verifikationsserier, utkast, underlag och bilagor, huvudbok och saldobalans, moms, SIE, kontroll av "senast påföljande arbetsdag", omlagt räkenskapsår och paginering av grundboken.
