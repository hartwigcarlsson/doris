# Doris – Steg 5: Huvudbok och saldobalans

## Kontext
Steg 4 gav varje företag en kontoplan och en grundbok, alltså verifikationerna i registreringsordning. BFL 5 kap. 1 § kräver också att bokföringen kan visas i systematisk ordning, det vill säga per konto. Det här steget lägger till en huvudbok per konto och en saldobalans per räkenskapsår. Båda är bara läsningar: det tillkommer inga event, inga kommandon och inga nya projektioner.

### Fattade beslut
| Område | Beslut |
|---|---|
| Källa | Frågor mot projektionerna `vouchers`, `voucher_lines` och `accounts` från steg 4. Ingen ny migration. |
| Period | Ett räkenskapsår åt gången, valt med `fiscal_year_start` precis som i `ListVouchers`. Det finns inga datumintervall inom året. |
| Rättelser | Både rättade verifikationer och rättelser räknas med. De tar ut varandra, vilket är det bokföringsmässigt korrekta. |
| Ingående balanser | Ingår inte. Saldot är bara årets rörelse. Det stämmer för första räkenskapsåret, och för senare år ger konton i klass 1–2 fel saldo tills ingående balanser finns. Sidan säger det i klartext. |
| Saldo | `debet − kredit` i öre (`i64`). Ett positivt saldo är debetsaldo. |
| Crate | Allt hör till `doris-ledger` (`queries.rs`) och `LedgerService`. |

## Läsningar (`crates/ledger/src/queries.rs`)
Båda funktionerna kontrollerar medlemskap först med `doris_company::get_company`, precis som `list_vouchers`. Den som inte är medlem får `company_not_found`.

### `trial_balance(pool, company_id, user_id, fiscal_year_start) -> Vec<TrialBalanceRow>`
- `TrialBalanceRow { account: u32, name: String, debit: i64, credit: i64 }`. Saldot räknas som `debit - credit` där det behövs och lagras inte.
- En enda fråga: `voucher_lines` grupperade på `account` med `SUM(debit)` och `SUM(credit)`, och `JOIN accounts` för namnet. Den sorteras på kontonummer.
- Bara konton med minst en rad under året kommer med. Ett konto vars rader tar ut varandra (till exempel 0 efter en rättelse) kommer också med, med saldo 0.
- Kontoplanen är alltid seedad om det finns verifikationer, eftersom seedningen sker vid första skrivningen. Därför räcker en `JOIN`.
- SQLite avbryter `SUM` med ett fel vid heltalsöverflöd i stället för att räkna fel. Det felet blir `internal`. Med högst 10¹³ öre per rad händer det inte i praktiken.

### `account_ledger(pool, company_id, user_id, fiscal_year_start, account) -> Vec<LedgerEntry>`
- `LedgerEntry { date, number, text, debit, credit, balance }`, där `balance` är det löpande saldot efter raden.
- Kontonumret valideras med `AccountNumber::parse` (`invalid_account_number`). Ett giltigt nummer utan rader, även ett som inte finns i kontoplanen, ger en tom lista.
- En fråga: `voucher_lines JOIN vouchers` för kontot och räkenskapsåret, sorterad på `date`, `number` och `line_no`. Om samma konto förekommer på två rader i en verifikation blir det två poster.
- Det löpande saldot räknas i en ren funktion, `running_balance(lines) -> Vec<LedgerEntry>`, med kontrollerad addition. Överflöd ger `internal`, av samma skäl som ovan.

`// ponytail:`-kommentar: hela året i ett svar, precis som grundboken. Paginering kommer när ett år blir för stort.

## API (`proto/doris/ledger/v1/ledger.proto`, `LedgerService`)
| RPC | In | Ut |
|---|---|---|
| `GetTrialBalance` | `company_id`, `fiscal_year_start` | `rows`: `account`, `name`, `debit`, `credit` |
| `GetAccountLedger` | `company_id`, `fiscal_year_start`, `account` | `entries`: `date`, `number`, `text`, `debit`, `credit`, `balance` |

- Ett ogiltigt datum ger `invalid_date` och ett ogiltigt konto `invalid_account_number`. Båda koderna finns redan och har svenska texter, så `errors.rs` behöver inga nya rader.
- Mappningen följer `crates/server/src/ledger.rs`: `caller` för session och medlemskap, `date` för `fiscal_year_start` och `domain_status` för domänfelen.

## Frontend
- **Navigering:** sidhuvudet får länken "Saldobalans" (`/trial-balance`) efter "Verifikationer".
- **`/trial-balance`, Saldobalans:**
  - En Select för räkenskapsår, fylld från `ListFiscalYears` precis som på `/vouchers`. Förvalt är året i `?fy=` om det finns, annars det nyaste.
  - Två tabeller, "Balansräkning" (konton i klass 1–2) och "Resultaträkning" (klass 3–8). Kolumnerna är Konto, Namn, Debet, Kredit och Saldo.
  - Varje tabell slutar med en summarad. Under resultaträkningen visas "Beräknat resultat", som är summan av resultaträkningens saldon med omvänt tecken (vinst är positiv).
  - Längst ned visas "Summa saldo". Den är 0 om bokföringen balanserar och visas som ett fel om den inte är det.
  - För andra räkenskapsåret och senare visas en notis: "Ingående balanser saknas än, så saldon för balansräkningens konton visar bara årets rörelser."
  - Ett tomt år visar "Inga verifikationer under räkenskapsåret."
  - Kontonumret är en länk till `/trial-balance/{konto}?fy={start}`.
- **`/trial-balance/:account`, Huvudbok för ett konto:**
  - Rubriken är "{nummer} {namn}". Namnet hämtas från `ListAccounts`.
  - Kolumnerna är Datum, Ver, Text, Debet, Kredit och Saldo, med en summarad längst ned.
  - Räkenskapsåret kommer från `?fy=`, med samma Select som på saldobalansen. Länken "Tillbaka till saldobalansen" behåller `?fy=`.
- **Byte av aktivt företag:** båda sidorna läser in på nytt när det aktiva företaget byts, precis som `/vouchers`. Om kontot inte finns hos det nya företaget visas en tom huvudbok.
- **Ren logik med enhetstester i `doris-web`:** uppdelning i balans- och resultaträkning och summering per avsnitt, i en funktion som tar raderna och returnerar avsnitten med summor.
- **Komponenter:** `Table`, `Select` och `amount` från `format.rs` finns redan. Inga nya beroenden tillkommer, och wasm-budgeten (900 KB) gäller.

## Tester
- **Ren funktion** (`running_balance`): saldot ackumuleras över debet- och kreditrader, och överflöd ger ett fel i stället för panik.
- **Lagring** (`crates/ledger/tests/store.rs`):
  - Givet några bokförda verifikationer, varav en rättad, ger `trial_balance` rätt summor per konto och summan av saldona är 0.
  - Verifikationer i ett annat räkenskapsår och hos ett annat företag räknas inte med.
  - `account_ledger` ger rätt ordning (datum före nummer, om en verifikation med högre nummer har ett tidigare datum) och rätt löpande saldo.
  - Den som inte är medlem får `company_not_found`.
- **Server** (`crates/server/tests/ledger.rs`, gRPC-Web): båda RPC:erna ger förväntat svar, `invalid_date` och `invalid_account_number` kommer fram, och en annan användare får `company_not_found`.
- **Playwright** (`e2e/tests/ledger.spec.ts`): bokför 1930 mot 3001, öppna Saldobalans och se båda kontona med saldo och "Beräknat resultat". Klicka på 1930 och se verifikation 1 i huvudboken.

## Utanför omfattningen
Ingående balanser, årsbokslut och låsta räkenskapsår (nästa steg). Dessutom datumintervall och perioder inom året, länkar från huvudboken till en enskild verifikation i grundboken, en huvudbok med alla konton på en sida, utskrift och export (PDF, SIE), balans- och resultaträkning med BAS-rubriker samt paginering.
