# Doris – Steg 6: Ingående balanser och stängning av räkenskapsår

## Kontext
Steg 5 gav en huvudbok och en saldobalans per räkenskapsår, men utan ingående balanser. Från andra året visar konton i klass 1–2 därför bara årets rörelse, vilket gör huvudboken fel (BFL 5 kap. 1 §). Det går också att bokföra i vilket år som helst, även ett som är avslutat. Det här steget lägger till ingående balanser och låter ett räkenskapsår stängas. Ett stängt år tar inte emot fler verifikationer, och stängningen för över årets resultat till eget kapital. Det är förutsättningen för årsbokslutet (BFL 6 kap. 1 §), och det stärker varaktigheten.

Man bokför i det nya året långt innan det gamla stängs, eftersom bokslutet ofta görs månader efteråt. De ingående balanserna måste därför finnas innan föregående år är stängt.

### Fattade beslut
| Område | Beslut |
|---|---|
| IB för senare år | Räknas fram och lagras inte. IB för år N = IB för år 1 + summan av alla rader i klass 1–2 under tidigare år. |
| IB för år 1 | Matas in manuellt, för företag som byter från ett annat system. Den lagras som ett event och kan ändras tills år 1 stängs. |
| Preliminär IB | Medan föregående år är öppet ligger dess resultat inte på eget kapital, så IB summerar inte till 0. Sidan säger att IB är preliminär. |
| Resultat | Stängningen bokför automatiskt en verifikation "Årets resultat" (8999 mot 2099 eller 2019) på årets sista dag. |
| Återöppning | Tillåten, som ett spårat event med en anledning. Man öppnar bakifrån: det senaste stängda året först. Resultatverifikationen rättas automatiskt. |
| Ordning | Ett år kan bara stängas om föregående år är stängt. |
| Ström | Allt ligger i den befintliga strömmen `ledger-{company_id}-{fy_start}`, så att verifikationer och stängning ser samma tillstånd och köas av samma `BEGIN IMMEDIATE`. |
| Behörighet | Alla medlemmar i företaget. Det finns inga roller per företag än. |
| Crate | Allt hör till `doris-ledger` och `LedgerService`. |

## Domän (`crates/ledger/src/domain.rs`, ren och utan I/O)

### Event
`LedgerEvent` får tre nya varianter. Befintliga event ändras inte, så `schema_version` förblir 1.

- `OpeningBalancesSet { lines: [{account, debit, credit}] }`: IB för år 1. Eventet ersätter hela den tidigare IB:n, och en tom lista tömmer den. Det är inte en verifikation och tar inget nummer. Vem som ändrade och när står i metadatan.
- `FiscalYearClosed { result_voucher: Option<u32> }`: numret på resultatverifikationen, om en bokfördes.
- `FiscalYearReopened { reason: String }`.

### Tillstånd
`Ledger` får `closed: bool`, `result_voucher: Option<u32>` och `opening_balances: Vec<VoucherLine>`, som alla sätts av `apply`. `FiscalYearReopened` sätter `closed = false` och `result_voucher = None`.

Den rena hjälpfunktionen `result_of(&Ledger) -> Option<i64>` returnerar summan av debet − kredit över konto 3000–8999 med kontrollerad aritmetik. `None` betyder överflöd, och lagret gör det till `Error::Overflow`.

### `set_opening_balances(ledger, chart, is_first_year, lines)`
- Det måste vara första räkenskapsåret (`not_first_fiscal_year`), och året får inte vara stängt (`fiscal_year_closed`).
- Högst 500 rader (`invalid_voucher_lines`). Noll rader är tillåtet och tömmer IB.
- Varje rad har exakt ett av debet och kredit större än 0, högst 10¹³ öre (`invalid_amount`).
- Bara konto 1000–2999 får förekomma (`not_balance_sheet_account`). Kontot ska finnas i kontoplanen men får vara inaktivt (`account_not_found`). Varje konto får förekomma högst en gång (`duplicate_account`).
- Summan av debet är lika med summan av kredit (`opening_balances_unbalanced`).

### `close_fiscal_year(ledger, previous_closed, legal_form, today)`
`previous_closed` är `None` för år 1 och annars om föregående år är stängt.
- Året får inte redan vara stängt (`fiscal_year_closed`).
- Året ska vara slut, `fiscal_year.end < today` (`fiscal_year_not_ended`).
- Föregående år ska vara stängt (`previous_fiscal_year_open`).
- Låt `r = result_of(ledger)`. Om `r != 0` blir det första eventet en `VoucherRecorded` med:
  - `number = last_number + 1`, `date = fiscal_year.end`, texten `Årets resultat` och `corrects: None`
  - två rader där motkontot är 2019 för enskild firma, handelsbolag och kommanditbolag och 2099 för övriga företagsformer. Om `r < 0` (vinst, kreditsaldo) får 8999 debet `-r` och motkontot kredit `-r`. Om `r > 0` (förlust) får 8999 kredit `r` och motkontot debet `r`.
- Det sista eventet är `FiscalYearClosed { result_voucher }`.
- Kontonas aktiv-status kontrolleras inte, precis som vid rättelser.
- Om användaren redan har bokfört resultatet själv är `r = 0`, och då bokförs ingen verifikation.

### `reopen_fiscal_year(ledger, next_closed, reason)`
- Året ska vara stängt (`fiscal_year_open`).
- Nästa år får inte vara stängt (`later_fiscal_year_closed`).
- Anledningen har 1–200 tecken efter trim (`invalid_reason`).
- Det första eventet är `FiscalYearReopened { reason }`. Om `result_voucher` är `Some(n)` och verifikation `n` inte redan är rättad, följer en rättelse av `n`. Den är daterad `fiscal_year.end` och byggs med samma logik som `correct_voucher`.
- Ordningen gör att det aldrig finns en `VoucherRecorded` mellan ett `FiscalYearClosed` och nästa `FiscalYearReopened` i en ström.

### Ändringar i befintlig logik
- `record_voucher` och `correct_voucher` avvisar ett stängt år med `fiscal_year_closed`, före alla andra kontroller av innehållet.
- `running_balance` får ett startvärde, `running_balance(opening, lines)`.
- Den befintliga destruktureringen av `LedgerEvent::VoucherRecorded` (i `apply`, `commit_voucher` och projektionerna) blir en `match`.

## Lagring och transaktioner (`crates/ledger/src/lib.rs`)

### Skrivflöde
Alla kommandon finns som `…_in(&mut conn, …)` och `…(pool, …)`, precis som `record_voucher`.

- **`set_opening_balances(company_id, actor, lines)`:**
  1. `begin`.
  2. Kontroll av medlemskap.
  3. `seeded_chart`.
  4. Ladda år 1:s ledger, kör `decide` och sedan `append`.
- **`close_fiscal_year(company_id, actor, fiscal_year_start, today)`:**
  1. `begin`, kontroll av medlemskap och kontroll av årsstarten (se nedan).
  2. Ladda årets ledger och, om det inte är år 1, föregående års ledger.
  3. `decide`.
  4. `append`, med resultatverifikationen och `FiscalYearClosed` i ett anrop.
- **`reopen_fiscal_year(company_id, actor, fiscal_year_start, reason, today)`:** samma flöde, men nästa års ledger laddas i stället för föregående. Ett nästa år som ligger efter `today` eller saknar event räknas som öppet.
- **Årsstarten:** kontrollen i `correct_voucher_in` (att året inte startar efter `today` och att `containing(start).start == start`) flyttas till en gemensam hjälpfunktion, `fiscal_year_at(company, start, today)`. Den ger `voucher_not_found` när den används för rättelser och den nya koden `fiscal_year_not_found` vid stängning och återöppning.
- **Samtidighet:** allt sker i en `BEGIN IMMEDIATE`-transaktion. Att stänga år N samtidigt som någon bokför i N eller öppnar N−1 köas därför och ger ett korrekt resultat.

### Projektioner (`migrations/0007_fiscal_year_closing.sql`)
- `opening_balances(company_id, account, debit, credit)` med primärnyckeln `(company_id, account)`. Vid `OpeningBalancesSet` töms raderna för företaget och skrivs på nytt.
- `closed_fiscal_years(company_id, fiscal_year_start, closed_at, closed_by)` med primärnyckeln `(company_id, fiscal_year_start)`. `FiscalYearClosed` lägger till en rad och `FiscalYearReopened` tar bort den. Projektionen får ändras, det är bara `events` som är append-only.
- Resultatverifikationen och rättelsen går genom den vanliga voucher-projektionen. Triggern `vouchers_numbered_without_gaps` gäller alltså även dem.
- `rebuild_projections` tömmer även de två nya tabellerna innan händelserna spelas upp igen.

### Läsningar (`queries.rs`)
Alla kontrollerar medlemskap först, precis som i dag.

- **IB för år N per konto:**
  - Om N är år 1 är det raderna i `opening_balances`.
  - Om N är senare är det `opening_balances` plus `voucher_lines` med `account BETWEEN 1000 AND 2999` och `fiscal_year_start < N`.

  Det blir en fråga med `UNION ALL` och `GROUP BY account`. Saldot är debet − kredit. Konton vars IB blir 0 tas inte med.
- **`trial_balance`:** `TrialBalanceRow` får fältet `opening: i64`. Konton som har IB men inga rader under året kommer med, med debet och kredit 0. IB och årets rader läses i en och samma lästransaktion.
- **`account_ledger`:** returnerar `AccountLedger { opening: i64, entries: Vec<LedgerEntry> }`. Det löpande saldot startar från `opening`, som är 0 för klass 3–8.
- **`list_fiscal_years`:** returnerar `FiscalYearStatus { fiscal_year, closed }`, där `closed` läses från `closed_fiscal_years`.
- **`opening_balances(company_id)`:** IB-raderna för år 1 sorterade på konto, som formuläret behöver.

## API (`proto/doris/ledger/v1/ledger.proto`, `LedgerService`)
| RPC | In | Ut |
|---|---|---|
| `GetOpeningBalances` | `company_id` | `lines` (`VoucherLine`) för år 1 |
| `SetOpeningBalances` | `company_id`, `lines` | — |
| `CloseFiscalYear` | `company_id`, `fiscal_year_start` | `result_voucher` (0 betyder ingen) |
| `ReopenFiscalYear` | `company_id`, `fiscal_year_start`, `reason` | — |

Befintliga meddelanden utökas: `FiscalYear` får `bool closed = 3`, `TrialBalanceRow` får `int64 opening = 5` och `GetAccountLedgerResponse` får `int64 opening = 2`.

Nya domänfel mappas i `crates/server/src/ledger.rs` till följande statusar:
- `INVALID_ARGUMENT` för `not_balance_sheet_account`, `duplicate_account`, `opening_balances_unbalanced` och `invalid_reason`.
- `NOT_FOUND` för `fiscal_year_not_found`.
- `FAILED_PRECONDITION` för `fiscal_year_closed`, `fiscal_year_open`, `fiscal_year_not_ended`, `previous_fiscal_year_open`, `later_fiscal_year_closed` och `not_first_fiscal_year`.

Varje kod får en svensk text i `crates/web/src/errors.rs`.

## Frontend
- **Navigering:** i sidhuvudet läggs länken "Räkenskapsår" (`/fiscal-years`) till efter "Saldobalans".
- **`/fiscal-years`, Räkenskapsår:**
  - En tabell med kolumnerna Räkenskapsår (`2026-01-01 – 2026-12-31`), Status ("Öppet" eller "Stängt") och en åtgärd.
  - "Stäng år" visas bara för det äldsta öppna året som har tagit slut. Ett klick visar en bekräftelse i raden, "Årets resultat bokförs som en verifikation och året låses för bokföring.", med knappen "Bekräfta stängning". Efteråt visas "Räkenskapsåret stängt. Resultatet bokfördes som ver N.", eller "Räkenskapsåret stängt." om ingen verifikation behövdes.
  - "Öppna igen" visas bara för det senaste stängda året. Ett klick visar fältet "Anledning" och knappen "Bekräfta" i raden.
  - Servern kontrollerar allt. Knapparnas synlighet är bara hjälp, och felen kommer från serverns koder.
  - Under tabellen finns länken "Ingående balanser" till `/opening-balances`.
- **`/opening-balances`, Ingående balanser:**
  - Rubriken anger år 1, till exempel "Ingående balanser 2025-01-01".
  - Formuläret har samma rader som Ny verifikation (Konto, Debet, Kredit, "Lägg till rad", "Ta bort") och en levande summering. `datalist` innehåller konton i klass 1–2, även inaktiva.
  - Knappen "Spara", och efteråt meddelandet "Ingående balanser sparade".
  - När sidan öppnas fylls formuläret från `GetOpeningBalances`. Om år 1 är stängt visas raderna som en skrivskyddad tabell.
  - Radkomponenten och dess tolkning lyfts ut ur `new_voucher.rs` till en delad modul, så att båda sidorna använder samma kod.
- **`/trial-balance`, Saldobalans:**
  - Kolumnerna blir Konto, Namn, Ingående, Debet, Kredit och Utgående, där Utgående = Ingående + Debet − Kredit.
  - Notisen om att ingående balanser saknas tas bort. I stället visas "Föregående räkenskapsår är inte stängt, så de ingående balanserna är preliminära.", men bara när föregående år finns och är öppet.
  - Ett stängt år får märket "Stängt" bredvid valet av räkenskapsår.
  - "Beräknat resultat" räknas på klass 3–8 utom 8999, så att årets resultat syns även efter stängningen.
- **`/trial-balance/:account`, Huvudbok:**
  - Första raden är "Ingående balans", med saldot i Saldo-kolumnen. Raden visas bara om kontot har en IB.
  - Det löpande saldot fortsätter från IB, och summaraden visar utgående saldo.
- **`/vouchers`, Verifikationer:**
  - I ett stängt år göms "Rätta", och märket "Stängt" visas.
  - "Ny verifikation" finns kvar, eftersom datumet avgör året. Servern avvisar ett stängt år med `fiscal_year_closed`.
- **Inga nya beroenden.** Wasm-budgeten (500 KB komprimerad med gzip) gäller.

## Tester
- **Domän** (`crates/ledger/tests/domain.rs`, given/when/then):
  - **IB:**
    - En balanserad IB i år 1 ger `OpeningBalancesSet`, och en tom lista godtas.
    - Varje felkod har ett eget fall: `not_first_fiscal_year`, `not_balance_sheet_account`, `duplicate_account`, `opening_balances_unbalanced`, `invalid_amount`, `account_not_found` och `fiscal_year_closed`.
    - Ett inaktivt konto godtas.
  - **Stängning:**
    - Vinst ger 8999 debet mot 2099 kredit, och förlust ger det omvända.
    - En enskild firma får 2019.
    - Resultatverifikationen har nästa nummer och är daterad sista dagen i året.
    - Om klass 3–8 redan summerar till 0 skapas ingen verifikation och `result_voucher` blir `None`.
    - Felen `fiscal_year_not_ended`, `previous_fiscal_year_open` och `fiscal_year_closed`.
    - År 1 kräver inget föregående år.
  - **Låsning:** `record_voucher` och `correct_voucher` i ett stängt år ger `fiscal_year_closed`.
  - **Återöppning:**
    - Eventen kommer i ordningen `FiscalYearReopened` och sedan rättelsen.
    - Om ingen resultatverifikation fanns blir det ingen rättelse.
    - Felen `fiscal_year_open`, `later_fiscal_year_closed` och `invalid_reason`.
    - Stäng, öppna igen och stäng: ett nytt resultat bokförs och numreringen saknar luckor.
  - **`running_balance`:** saldot fortsätter från startvärdet.
- **Lagring** (`crates/ledger/tests/store.rs`):
  - IB för år 2 är IB för år 1 plus rörelserna i klass 1–2 under år 1.
  - Före stängningen av år 1 skiljer sig summan av IB för år 2 från 0 med exakt år 1:s resultat. Efter stängningen är summan 0.
  - `trial_balance` tar med konton som har IB men ingen rörelse.
  - `account_ledger` startar på IB.
  - `list_fiscal_years` ger rätt `closed` efter stängning och efter återöppning.
  - Ombyggnad från `read_all` ger identiska tabeller, med IB, en stängning och en återöppning.
  - Den som inte är medlem får `company_not_found` vid de nya kommandona och läsningarna.
  - Ett `fiscal_year_start` som inte är en årsstart ger `fiscal_year_not_found`.
- **Stresstest** (`crates/ledger/tests/stress.rs`):
  - Det äldsta av de tre åren stängs och öppnas igen av några tasks, medan andra bokför och rättar i samma år.
  - Efteråt gäller de befintliga kontrollerna. Dessutom finns ingen `VoucherRecorded` mellan ett `FiscalYearClosed` och nästa `FiscalYearReopened` i strömmen.
- **Server** (`crates/server/tests/ledger.rs`, gRPC-Web):
  - De fyra nya RPC:erna ger förväntade svar.
  - Koderna `fiscal_year_closed`, `previous_fiscal_year_open` och `opening_balances_unbalanced` kommer fram.
  - En annan användare får `company_not_found`.
  - `opening` kommer med i saldobalansen och huvudboken.
- **Playwright** (`e2e/tests/fiscal_year.spec.ts`):
  1. Skapa ett företag vars första räkenskapsår redan har tagit slut (föregående kalenderår).
  2. Mata in IB: 1930 debet mot 2081 kredit.
  3. Bokför en försäljning i det året.
  4. Stäng året och se "Resultatet bokfördes som ver N".
  5. Försök bokföra i året och se det svenska felet.
  6. Öppna innevarande års saldobalans och se rätt ingående värden på 1930 och 2099.
  7. Öppna det gamla året igen med en anledning och se rättelsen i grundboken.

## Utanför omfattningen
- Årsredovisning (K2/K3)
- Balans- och resultaträkning med BAS-rubriker
- Automatisk omföring av 2099 till 2098 efter stämman (användaren bokför den själv)
- IB för år 1 med fler än 500 rader
- Roller och behörigheter per företag
- SIE-export
- Omlagt räkenskapsår
