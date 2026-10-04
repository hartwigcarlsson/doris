# Doris – Steg 8: Resultat- och balansräkning

## Kontext
Steg 5 gav saldobalansen per konto och steg 6 gav ingående balanser och stängda räkenskapsår. Det här steget ställer upp samma siffror som en resultaträkning och en balansräkning med rubriker enligt årsredovisningslagens (ÅRL) förkortade uppställning, så som K2 använder den. Rapporten är det företagaren tittar på för att se hur det går. Den är också grunden för årsredovisningen i ett senare steg. Allt här är läsningar: det tillkommer inga event, inga kommandon, inga projektioner och ingen migration.

### Fattade beslut
| Område | Beslut |
|---|---|
| Uppställning | ÅRL bilaga 2 (kostnadsslagsindelad resultaträkning) och bilaga 1 (balansräkning), förkortad som i K2. |
| Koppling konto → post | BAS SRU-kolumn för INK2R, vars poster motsvarar K2:s. En statisk tabell i `crates/ledger/src/statements.rs`, som är den enda platsen för kopplingen. |
| Eget kapital | Styrs av `legal_form`. AB, ekonomisk förening och övriga former får bundet och fritt eget kapital. Enskild firma, HB och KB får "Eget kapital" och "Årets resultat". |
| Jämförelse | En kolumn för föregående räkenskapsår om det finns i Doris. Första året har ingen. |
| Beräkning | På servern i en ren funktion, `build`. Klienten ritar bara upp raderna. |
| Rubriktexter | Skickas från servern, som BAS-kontonamnen i `bas.rs`. De är facktermer ur ÅRL. |
| Tomma poster | En post som är 0 både i år och föregående år döljs (ÅRL tillåter det). Rubriker och delsummor visas alltid. |
| Belopp | Öre (`i64`). Kontrollerad addition, och ett överflöd ger `internal`. |

## Uppställning

### Resultaträkning
Varje post är −Σ(debet − kredit) för sina konton, för året, så att intäkter blir positiva och kostnader negativa.

| Post | Konton (grov indelning) |
|---|---|
| Nettoomsättning | 3000–3799 |
| Förändring av lager av produkter i arbete, färdiga varor och pågående arbete för annans räkning | delar av 4900–4999 enligt SRU |
| Aktiverat arbete för egen räkning | 3800–3899 |
| Övriga rörelseintäkter | 3900–3999 |
| Råvaror och förnödenheter | delar av 4000–4999 enligt SRU |
| Handelsvaror | delar av 4000–4999 enligt SRU |
| Övriga externa kostnader | 5000–6999 |
| Personalkostnader | 7000–7699 |
| Av- och nedskrivningar av materiella och immateriella anläggningstillgångar | 7700–7899 enligt SRU |
| Övriga rörelsekostnader | 7900–7999 |
| **Rörelseresultat** | delsumma |
| Finansiella poster (resultat från andelar, övriga ränteintäkter, räntekostnader m.fl. enligt SRU) | 8000–8799 |
| **Resultat efter finansiella poster** | delsumma |
| Bokslutsdispositioner | 8800–8899 |
| **Resultat före skatt** | delsumma |
| Skatt på årets resultat | 8900–8979 enligt SRU |
| Övriga skatter | 8980–8989 enligt SRU |
| **Årets resultat** | delsumma |

Konto 8990–8999 räknas inte i resultaträkningen. Där hamnar bokslutsverifikationens 8999, som bara flyttar resultatet till eget kapital.

### Balansräkning
Varje post är UB, alltså IB plus årets rörelse, från `trial_balance`. Tillgångar visas med debet som plus. Eget kapital och skulder visas med kredit som plus.

**Tillgångar**
- **Anläggningstillgångar:**
  - Immateriella anläggningstillgångar (10xx).
  - Materiella anläggningstillgångar: Byggnader och mark (11xx), samt Maskiner och inventarier (12xx).
  - Finansiella anläggningstillgångar (13xx).
- **Omsättningstillgångar:**
  - Varulager m.m. (14xx).
  - Kortfristiga fordringar: Kundfordringar (15xx), Övriga fordringar (16xx), samt Förutbetalda kostnader och upplupna intäkter (17xx).
  - Kortfristiga placeringar (18xx).
  - Kassa och bank (19xx).
- **Summa tillgångar.**

**Eget kapital och skulder**
- **Eget kapital, för AB, ekonomisk förening och övriga former:**
  - Bundet eget kapital (2080–2089).
  - Fritt eget kapital: Balanserat resultat (2090–2099 och 8990–8999), samt Årets resultat (beräknat resultat, se nedan).
  - Konton 2000–2079 ligger under Bundet eget kapital om SRU säger det, annars under Balanserat resultat.
- **Eget kapital, för enskild firma, HB och KB:**
  - Eget kapital (20xx och 8990–8999).
  - Årets resultat (beräknat resultat).
- Obeskattade reserver (21xx).
- Avsättningar (22xx).
- Långfristiga skulder (23xx).
- Kortfristiga skulder: Leverantörsskulder (244x), Skatteskulder (251x), Övriga kortfristiga skulder, samt Upplupna kostnader och förutbetalda intäkter (29xx).
- **Summa eget kapital och skulder.**

De exakta intervallen tas från BAS SRU-kolumn när tabellen skrivs, och intervallen ovan är grovindelningen. Ett test kräver att varje kontonummer från 1000 till 8999 hamnar i exakt en post.

### Årets resultat för öppna och stängda år
Det är samma regel för båda. Koden behöver inte veta om året är stängt.
- **Resultaträkningen:** −Σ saldon på 3000–8989.
- **Balansräkningen:** −Σ saldon på 3000–8989, alltså samma belopp som i resultaträkningen. ÅRL kräver att de stämmer.
  - Resultatkontot (2099, eller 2019 för EF/HB/KB) och 8990–8999 ligger med tidigare års resultat: Balanserat resultat, eller Eget kapital för EF/HB/KB.
  - *Stängt år:* bokslutsverifikationen bokar 8999 mot resultatkontot. Båda ligger i samma post och tar ut varandra, så ett stängt år ser ut som ett öppet.
  - Ett tidigare års resultat som ligger kvar på 2099 (eller 2019) och inte har förts vidare visas därför som Balanserat resultat (eller Eget kapital), inte som årets.

### Balanskontroll
`difference = Summa tillgångar − Summa eget kapital och skulder`. IB för år 1 måste balansera och varje verifikation balanserar. Den enda orsaken till en differens är därför att ett tidigare år inte är stängt, så att dess resultat aldrig har flyttats till eget kapital. Differensen visas med den förklaringen.

## Domän (`crates/ledger/src/statements.rs`, ren kod)
```rust
pub enum LineKind { Heading, Item, Subtotal }
pub struct StatementLine {
    pub label: &'static str,
    pub kind: LineKind,
    pub amount: i64,           // öre, with the sign the statement shows
    pub previous: Option<i64>, // None when there is no previous year
}
pub struct FinancialStatements {
    pub income: Vec<StatementLine>,
    pub balance: Vec<StatementLine>,
    pub difference: i64,
    pub previous_difference: Option<i64>,
}
pub fn build(
    current: &[TrialBalanceRow],
    previous: Option<&[TrialBalanceRow]>,
    legal_form: LegalForm,
) -> Result<FinancialStatements>
```
- Kopplingen är en `const` med `(från, till, post)`.
- Uppställningen är en statisk lista med rubriker, poster och delsummor per rapport, med en variant för eget kapital per juridisk form.

## Läsning (`crates/ledger/src/queries.rs`)
### `financial_statements(pool, company_id, user_id, fiscal_year_start) -> FinancialStatements`
1. `doris_company::get_company` kontrollerar medlemskap och ger `legal_form` och `first_fiscal_year`. Den som inte är medlem får `company_not_found`.
2. `fiscal_year_start` måste vara början på ett av företagets räkenskapsår (`start >= first.start` och `first.containing(start).start == start`), annars `fiscal_year_not_found`.
3. Föregående år finns om `start > first.start`. Det är året vars `next()` börjar på `start`.
4. `trial_balance` körs för valt år och, om det finns, för föregående år. Resultaten skickas till `build`.

`// ponytail:` två frågor och därmed två snapshots. En verifikation som bokförs mellan dem kan synas i bara den ena kolumnen. Det kan rättas med en gemensam läs-transaktion om det någon gång spelar roll.

## API (`proto/doris/ledger/v1/ledger.proto`, `LedgerService`)
```proto
rpc GetFinancialStatements(GetFinancialStatementsRequest) returns (GetFinancialStatementsResponse);

message GetFinancialStatementsRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
}
enum StatementLineKind {
  STATEMENT_LINE_KIND_UNSPECIFIED = 0;
  STATEMENT_LINE_KIND_HEADING = 1;
  STATEMENT_LINE_KIND_ITEM = 2;
  STATEMENT_LINE_KIND_SUBTOTAL = 3;
}
message StatementLine {
  string label = 1;
  StatementLineKind kind = 2;
  int64 amount = 3;
  optional int64 previous = 4;
}
message GetFinancialStatementsResponse {
  repeated StatementLine income_statement = 1;
  repeated StatementLine balance_sheet = 2;
  string previous_fiscal_year_start = 3; // empty when there is none
  int64 difference = 4;
  optional int64 previous_difference = 5;
}
```
- Felkoderna `invalid_date`, `fiscal_year_not_found` och `company_not_found` finns redan. `errors.rs` får därför inga nya rader.
- Mappningen i `crates/server/src/ledger.rs` följer `get_trial_balance`, med `caller`, `date` och `domain_status`.

## Frontend
- **Navigering:** sidhuvudet får länken "Rapporter" (`/financial-statements`) efter "Saldobalans".
- **`/financial-statements`, Resultat- och balansräkning:**
  - En Select för räkenskapsår med `?fy=`, som på `/trial-balance`. Den fylls från `ListFiscalYears`, och nyaste året är förvalt. Vid ett stängt år står "Räkenskapsåret är stängt" bredvid.
  - Två `Card` under varandra: "Resultaträkning" och "Balansräkning".
  - Kolumnerna är Post, valt år och föregående år. Rubriken för varje beloppskolumn är periodens datum ("2026-01-01 – 2026-12-31"), så att ett förkortat eller förlängt första år syns. Jämförelsekolumnen visas bara när `previous_fiscal_year_start` inte är tom.
  - Hur en rad ser ut beror på `kind`:
    - *Heading* är fet text utan belopp.
    - *Item* är indragen.
    - *Subtotal* är fet med en kantlinje ovanför.
  - Belopp formateras med `amount` från `format.rs`.
  - Om `difference` eller `previous_difference` inte är 0 visas en varning under balansräkningen: "Balansräkningen balanserar inte (differens {belopp}). Ett tidigare räkenskapsår är inte stängt, så dess resultat finns inte i eget kapital."
- **Byte av aktivt företag** läser in sidan på nytt, som `/trial-balance`.
- **Komponenter:** `Card`, `Table`, `Select` och `amount` finns redan. Det blir inga nya beroenden och ingen ny logik i klienten, och wasm-budgeten gäller.

## Tester
- **Ren funktion** (`statements.rs`):
  - Varje kontonummer från 1000 till 8999 hamnar i exakt en post.
  - Försäljning och kostnader ger rätt rörelseresultat och årets resultat, med rätt tecken.
  - Årets resultat i balansräkningen blir lika för ett öppet år och för samma år stängt med 8999 mot 2099, och differensen är 0 i båda fallen.
  - AB får bundet och fritt eget kapital. Enskild firma får "Eget kapital" och "Årets resultat".
  - Ett tidigare års resultat som ligger kvar på 2099 eller 2019 ingår inte i balansräkningens Årets resultat, som alltid är lika med resultaträkningens.
  - En post som är 0 i båda åren döljs. En post som är 0 i år men har belopp föregående år visas.
  - Utan föregående år har alla rader `previous: None`.
  - Ett underlag där ett tidigare år inte är stängt ger rätt `difference`.
  - Överflöd ger ett fel i stället för panik.
- **Lagring** (`crates/ledger/tests/store.rs`):
  - År 2 får jämförelsetal från år 1.
  - En `fiscal_year_start` mitt i ett år ger `fiscal_year_not_found`.
  - Den som inte är medlem får `company_not_found`.
- **Server** (`crates/server/tests/ledger.rs`, gRPC-Web): svaret har rätt form, och `invalid_date` och `fiscal_year_not_found` når fram.
- **Playwright** (`e2e/tests/ledger.spec.ts`): bokför 1930 mot 3001 och öppna Rapporter. Nettoomsättning och Årets resultat ska visa beloppet, och Kassa och bank ska visa det i balansräkningen.

## Dokumentation
`AGENTS.md` får en rad om `GetFinancialStatements` och om att `statements.rs` är den enda platsen för kopplingen mellan konto och ÅRL-post.

## Utanför omfattningen
- Årsredovisning, noter, förvaltningsberättelse och K3
- Funktionsindelad resultaträkning
- Jämförelsetal för första året från inmatad IB
- Kontonivå under varje post (saldobalansen visar den)
- Perioder inom året
- PDF och utskrift
- SIE- och SRU-export
- Automatisk omföring av 2099 till 2098
