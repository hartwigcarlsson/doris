# Doris – Steg 10: Skattetabeller

## Kontext
I steg 9 skrev användaren in preliminärskatten för hand på varje rad i lönekörningen. Det här steget räknar fram skatten automatiskt. Varje anställd får en skatteinställning: en skattetabell med kolumn, eller en fast procentsats. Månadstabellerna hämtas från Skatteverkets öppna data, första gången ett år behövs. En uträknad skatt går alltid att skriva över för hand, till exempel för en bonus som beskattas med engångsskatt.

### Fattade beslut
| Område | Beslut |
|---|---|
| Datakälla | Skatteverkets öppna dataset "Skattetabeller för månadslön" via EntryScape rowstore. Det kräver ingen inloggning. Servern hämtar ett år första gången det behövs och sparar det i SQLite. Nästa års tabeller kommer automatiskt när Skatteverket publicerar dem i december, utan ny release av Doris. |
| Utan tabell | Om Skatteverket inte går att nå, eller året inte är publicerat än, blir det felkoden `tax_table_unavailable`. Skatten kan fortfarande skrivas in för hand. Fast procent kräver ingen tabell. |
| Skatteinställning | Per anställd: **tabell** 29–42 och kolumn 1–6, eller **fast procent** 0–100 (hela procent). En anställd från steg 9 har ingen inställning, och då skrivs skatten in för hand som tidigare. |
| Överskrivning | Skatten i en rad i körningen kan lämnas tom, och räknas då fram, eller fyllas i, och är då manuell. |
| Spårbarhet | Varje låst rad sparar sin skattegrund: tabell, kolumn och år, procent, eller manuell. Tabellerna själva är referensdata och inga händelser, medan beloppet och grunden låses i händelsen. |
| Avrundning | Tabellbeloppen är hela kronor. Procentuträkningar avrundas nedåt till hela kronor. |
| HTTP | Bara servern gör HTTP-anrop, på samma sätt som Bolagsverket-uppslaget. `doris-payroll` förblir fri från nätverk. Inget anrop görs medan SQLite:s skrivlås hålls. |

## Skatteverkets data
- **Dataset:** `https://skatteverket.entryscape.net/rowstore/dataset/88320397-5c32-4c16-ae79-d36d95b17b95` (Skattetabeller för månadslön, alla år). Det frågas med `?år={år}&_limit=500&_offset={n}` och svarar med JSON: `{ resultCount, results: [...] }`.
- **En rad** ser ut så här:
  `{ "år": "2026", "tabellnr": "33", "antal dgr": "30B" | "30%", "inkomst fr.o.m.": "34801", "inkomst t.o.m.": "35000" | "", "kolumn 1" … "kolumn 6": "7134", "kolumn 7": "" }`.
- **Format 2026** enligt SKV 433, avsnitt 5:
  - **`30B`** är rader med skatteavdraget i kronor. De täcker 1–80 000 kr, i intervall om 100 kr upp till 20 000 kr och därefter 200 kr.
  - **`30%`** är rader med procent, från 80 001 kr och uppåt. Den sista raden har ett tomt `inkomst t.o.m.`, vilket betyder att den saknar övre gräns.
  - Ett år har 14 tabeller (29–42) med 569 rader var. `kolumn 7` används inte och sparas inte.
- **Kolumner:** kolumn 1 är lön till den som inte fyllt 66 vid årets ingång (jobbskatteavdrag). Kolumn 2–6 gäller pension, 66+, sjuk- och aktivitetsersättning med mera, enligt Skatteverkets förklaringar.
- **Konfiguration:** adressen kan ändras med `DORIS_TAX_TABLES_URL` (CLI `--tax-tables-url`). Standardvärdet är adressen ovan.

## Domän (`crates/payroll/src/domain.rs` och ny modul `crates/payroll/src/tax.rs`, ren och utan I/O)

### Värdeobjekt
- **`TaxSetting`:**
  - `Table { table: u8, column: u8 }`, där tabellen är 29–42 och kolumnen 1–6. Annat ger `InvalidTaxTable`.
  - `Percent { percent: u8 }`, där procenten är 0–100. Annat ger `InvalidTaxPercent`.
  - Konstruktorerna heter `TaxSetting::table(t, c)` och `TaxSetting::percent(p)`.
- **`TaxBasis`**, som låses på varje rad:
  - `Table { year: i16, table: u8, column: u8 }`,
  - `Percent { percent: u8 }`,
  - `Manual`.

  Den har `#[serde(default)]`, där standardvärdet är `Manual`.
- **`TaxTable { year: i16, rows: Vec<TaxTableRow> }`**, där `TaxTableRow { table: u8, kind: Amount | Percent, from: i64, to: Option<i64>, columns: [i64; 6] }`. Inkomstgränserna anges i hela kronor. Värdena är kronor för `Amount` och procent för `Percent`.

### Kontroll av en hämtad tabell
`TaxTable::validate(year, rows) -> Result<TaxTable, TaxTableError>` kräver följande för vart och ett av tabellnumren 29–42:
- `Amount`-raderna, sorterade på `from`, börjar på 1, slutar på 80 000 och har inga glapp eller överlapp (`from` = föregående `to` + 1).
- `Percent`-raderna börjar på 80 001 och har inga glapp. Exakt den sista har `to = None`.
- Inga andra tabellnummer förekommer.

Annars blir det `TaxTableError` med en beskrivning för loggen. Sådana fel innehåller aldrig personuppgifter, eftersom tabellerna inte har några.

### Uträkning
`preliminary_tax(setting: &TaxSetting, table: Option<&TaxTable>, gross: i64) -> Result<(i64, TaxBasis), TaxError>` räknar i öre:
- **Inkomsten** är `gross / 100`, i hela kronor med öre bortkastade.
- **`Table`:**
  - En inkomst på högst 80 000 kr ger `Amount`-raden där `from ≤ inkomst ≤ to`. Skatten är `columns[kolumn - 1] × 100` öre.
  - En inkomst på 0 kr ger skatten 0.
  - En inkomst över 80 000 kr ger `Percent`-raden som omfattar inkomsten. Skatten är `floor(inkomst × procent / 100) × 100` öre.
  - Saknas tabellen blir det `TaxError::TableMissing(year)`.
- **`Percent`:** `floor(inkomst × procent / 100) × 100` öre. Ingen tabell behövs.
- **Taket på skatten:** skatten blir aldrig större än `gross`. Det kan bara bli aktuellt vid 100 %, och då gäller `min(skatt, gross)`.

### Händelser
- **Ny händelse** i strömmen `payroll-{company_id}`: `EmployeeTaxChanged { employee_id, tax: TaxSetting }`. Befintliga händelser ändras inte. En ändring utan effekt ger inga händelser. En inaktiv anställd ger `EmployeeInactive`.
- **`AddEmployee`** får `tax: Option<TaxSetting>`. Är den satt läggs `EmployeeTaxChanged` till direkt efter `EmployeeAdded`.
- **`DraftLine.tax`** blir `Option<i64>`, där `None` betyder att skatten räknas fram. Gamla händelser har alltid ett tal och läses därför som `Some`, alltså manuell skatt. Ingen uppkastning behövs.
- **`PayrollRunLine`** får `tax_basis: TaxBasis`. Gamla `PayrollRunFinalized` saknar fältet och läses som `Manual` via `#[serde(default)]`. `schema_version` förblir 1, eftersom ingen befintlig händelse får ny betydelse.

### Lönekörningen
`compute_lines(payroll, run_id, draft, table: Option<&TaxTable>)` får tabellen för utbetalningsdagens år. Per rad gäller:
- **`tax: Some(x)`:** det befintliga villkoret 0 ≤ x ≤ brutto gäller (`InvalidTax`), och grunden blir `Manual`.
- **`tax: None` med en inställning:** `preliminary_tax` räknar fram skatten. `TableMissing(år)` går vidare som `DomainError::TaxTableMissing(year)`.
- **`tax: None` utan inställning:** `TaxRequired`.

`validate_draft` kräver inte längre en skatt på varje rad. Kontrollen av den manuella skatten ligger kvar.

`book_payroll_run` räknar inte om skatten. De låsta raderna gäller, som i steg 9.

## Lagring

### Migration `migrations/0011_tax_tables.sql`
```sql
-- Skatteverket's monthly tax tables (referensdata, not events): fetched
-- once per year, replaceable. What a run used is locked in its event.
CREATE TABLE tax_tables (
    year      INTEGER NOT NULL,
    table_no  INTEGER NOT NULL,
    kind      TEXT    NOT NULL CHECK (kind IN ('amount', 'percent')),
    income_from INTEGER NOT NULL,
    income_to   INTEGER,          -- NULL: no upper limit
    col1 INTEGER NOT NULL, col2 INTEGER NOT NULL, col3 INTEGER NOT NULL,
    col4 INTEGER NOT NULL, col5 INTEGER NOT NULL, col6 INTEGER NOT NULL,
    PRIMARY KEY (year, table_no, kind, income_from)
);

ALTER TABLE employees ADD COLUMN tax_table  INTEGER;  -- with tax_column
ALTER TABLE employees ADD COLUMN tax_column INTEGER;
ALTER TABLE employees ADD COLUMN tax_percent INTEGER;
-- payroll_run_lines.tax becomes nullable (NULL: computed) and gains
-- tax_basis (JSON TaxBasis, NULL while open). SQLite can't drop NOT NULL,
-- so the projection table is recreated and its rows copied.
```
- `employees` får sina skattekolumner från `EmployeeTaxChanged`, och en ny inställning nollställer den andra varianten.
- `payroll_run_lines.tax_basis` sätts av `PayrollRunFinalized` och nollställs av `PayrollRunReopened`.
- Ombyggnaden från `read_all` täcker de nya kolumnerna. `tax_tables` byggs inte om, eftersom den inte kommer från händelser.

### `doris-payroll` (lib)
- `tax_table(pool, year) -> Result<Option<TaxTable>>` läser en sparad tabell.
- `store_tax_table(pool, TaxTable)` gör `DELETE` för året och `INSERT` av alla rader i en egen `BEGIN IMMEDIATE`. Ett år är alltså aldrig halvsparat.
- `preview_payroll_run` och `finalize_payroll_run` läser tabellen för utbetalningsdagens år. Det sker i transaktionen för `finalize`, så det är samma ögonblicksbild som besluten bygger på. Om tabellen behövs och saknas blir det `Error::Domain(TaxTableMissing(year))`, och ingenting skrivs.
- `set_employee_tax(pool, company_id, actor, employee_id, TaxSetting)`.
- `NewEmployee` får `tax: Option<TaxSetting>`.

## Server

### Hämtning (`crates/server/src/skatteverket.rs`)
- `TaxTables::new(url)` använder en `reqwest::Client`, på samma sätt som `Bolagsverket`, med en timeout på 30 sekunder per sida.
- `fetch(year) -> Result<TaxTable, FetchError>` hämtar sidor med `_limit=500` tills alla `resultCount` rader är lästa. Raderna tolkas: siffror som text, `"30B"` blir `Amount`, `"30%"` blir `Percent` och ett tomt `t.o.m.` blir `None`. Därefter körs `TaxTable::validate`.
  - Ett nätverksfel, en annan HTTP-status än 200, trasig JSON, 0 rader eller en underkänd kontroll ger `FetchError`. Det loggas med orsaken och mappas till `tax_table_unavailable`.
- Om två anrop behöver samma år samtidigt hämtar båda. Den som sparar sist vinner, och tabellerna är identiska. `// ponytail:` per-år-lås om hämtningarna blir dyra.

### `PayrollService`
- `PreviewPayrollRun` och `FinalizePayrollRun` är de enda anropen som räknar skatt, och de fungerar så här vid `TaxTableMissing(year)`:
  1. Hämta året med `TaxTables::fetch(year)`.
  2. Spara det med `store_tax_table`.
  3. Försök anropet en gång till.
  4. Går hämtningen inte, blir det `tax_table_unavailable`.
- **`SetEmployeeTax(SetEmployeeTaxRequest { company_id, employee_id, tax })`** är ett nytt anrop.
- **Nya felkoder:**
  - `invalid_tax_table`, `invalid_tax_percent` och `tax_required` mappas till `InvalidArgument`.
  - `tax_table_unavailable` mappas till `Unavailable`.
- `router` och `main` får `TaxTables`, som byggs från `--tax-tables-url` / `DORIS_TAX_TABLES_URL`.

### Proto (`payroll.proto`)
```proto
message TaxSetting {
  oneof kind {
    TableTax table = 1;
    uint32 percent = 2;
  }
}
message TableTax { uint32 table = 1; uint32 column = 2; }

message TaxBasis {
  oneof kind {
    TableBasis table = 1;
    uint32 percent = 2;
    bool manual = 3;
  }
}
message TableBasis { uint32 year = 1; uint32 table = 2; uint32 column = 3; }

// Employee:            TaxSetting tax = 7;          (unset: no setting)
// AddEmployeeRequest:  TaxSetting tax = 6;          (optional)
// PayrollRunLineInput: optional int64 tax = 3;      (unset: computed)
// PayrollRunLine:      optional int64 tax = 4;      (unset: open line, computed)
//                      TaxBasis tax_basis = 9;      (once finalized, and in a preview)

rpc SetEmployeeTax(SetEmployeeTaxRequest) returns (SetEmployeeTaxResponse);
message SetEmployeeTaxRequest { string company_id = 1; string employee_id = 2; TaxSetting tax = 3; }
message SetEmployeeTaxResponse {}
```
`PayrollRunLineInput.tax` får `optional`, och det är inte bakåtkompatibelt på tråden: en äldre klient som skickar 0 utelämnar fältet. Frontend och server levereras alltid tillsammans i samma binär, så det spelar ingen roll.

## Frontend

### `/employees`, Anställda
- Formuläret får fältet "Skatt" med valen "Skattetabell" (förvalt), "Fast procent" och "Ingen (skatten skrivs in för hand)". En inställning kan bytas men inte tas bort, eftersom det saknas en händelse för det. Därför visas "Ingen" bara för en anställd som saknar inställning.
  - **Skattetabell:** "Tabell" som en lista 29–42 och "Kolumn" som en lista 1–6.
  - **Fast procent:** fältet "Procent".
- Kolumnlistans texter:
  - "1 – Lön (under 66 år)", förvald.
  - "2 – Pension (66 år eller äldre)".
  - "3 – Lön (66 år eller äldre)".
  - "4 – Sjuk- och aktivitetsersättning".
  - "5 – Annan pensionsgrundande ersättning".
  - "6 – Pension (under 66 år)".
- När en anställd sparas anropas `AddEmployee` med `tax`. Vid redigering anropas `UpdateEmployee` och sedan `SetEmployeeTax`, om skatten har ändrats.
- Tabellen får kolumnen "Skatt", som visar "Tabell 33, kol 1", "30 %" eller "–".

### `/payroll-runs/new` och `/payroll-runs/:id`
- Skattefältet för en anställd med inställning har platshållaren "Tabell 33, kol 1" eller "30 %". Lämnas det tomt räknas skatten fram.
- Kontrollen "Ange skatt för {namn}." från steg 9 gäller nu bara anställda utan inställning.
- `RunLines`, alltså förhandsgranskningen och den låsta vyn, får kolumnen "Skattegrund", som visar "T33 k1", "30 %" eller "Manuell".

### `src/errors.rs`
- `invalid_tax_table`: "Välj tabell 29–42 och kolumn 1–6."
- `invalid_tax_percent`: "Procentsatsen måste vara 0–100."
- `tax_required`: "Ange skatt eller en skatteinställning för den anställda."
- `tax_table_unavailable`: "Skattetabellen kunde inte hämtas från Skatteverket. Försök igen senare eller skriv in skatten för hand."

## Tester
Varje beteende utvecklas med TDD: rött, grönt, refaktorering och commit.

### Domän (`crates/payroll/tests/tax.rs`, `domain.rs`)
- **`TaxSetting`:**
  - tabell 28 och 43 avvisas, 29 och 42 godtas,
  - kolumn 0 och 7 avvisas,
  - procent 101 avvisas, 0 och 100 godtas.
- **`TaxTable::validate`** godtar en minimal korrekt tabell, där varje tabellnummer 29–42 har ett fåtal beloppsrader 1–80 000 och en öppen procentrad. Den avvisar:
  - ett saknat tabellnummer,
  - ett glapp eller överlapp i beloppen,
  - beloppsrader som slutar på 79 999,
  - en procentrad som inte är öppen,
  - två öppna procentrader,
  - ett okänt tabellnummer.
- **`preliminary_tax` med tabell:**
  - Uppslagning vid intervallgränser, med Skatteverkets verkliga värden för 2026 tabell 33 kolumn 1 som fixtur. Inkomsten 35 000 kr ger 7 134 kr.
  - Gränserna 2 000 och 2 001, 20 000 och 20 001, samt 80 000 och 80 001 kr.
  - 0 kr ger 0.
  - Öre kastas: 35 000,99 kr räknas som 35 000.
  - Den översta öppna procentraden.
  - Kolumn 3 skiljer sig från kolumn 1.
- **`preliminary_tax` med procent:**
  - 30 % av 12 345,67 kr ger floor(12 345 × 0,3) = 3 703 kr.
  - 0 % ger 0.
  - 100 % ger hela lönen i kronor, men aldrig mer än `gross`.
- **`compute_lines`:**
  - Ingen skatt med tabell ger skatten och `TaxBasis::Table` med år.
  - En ifylld skatt ger `Manual`.
  - Ingen inställning ger `TaxRequired`.
  - Saknad tabell ger `TaxTableMissing(år)`.
  - Procent fungerar utan tabell.
- **Uppkastning:** en `PayrollRunFinalized` i JSON utan `tax_basis` läses som `Manual`, och en `DraftLine` med `"tax": 800000` läses som `Some`.
- **Given/when/then för `EmployeeTaxChanged`:** inställningen sätts, samma inställning ger inga händelser, och en inaktiv anställd avvisas.

### Lagring (`crates/payroll/tests/store.rs`)
- `store_tax_table` följt av `tax_table` ger samma tabell tillbaka. Att spara samma år igen ersätter tabellen och dubblerar inget.
- `finalize_payroll_run` med en tabellanställd och utan sparad tabell ger `TaxTableMissing`, och ingenting skrivs. När tabellen är sparad låses skatt och grund.
- Ombyggnaden från `read_all` ger samma `employees`, med skattekolumner, och samma `payroll_run_lines`, med `tax_basis`.

### Server (`crates/server/tests/payroll.rs`)
En låtsas-Skatteverket körs som en axum-tjänst på en ledig port i testet och serverar en minimal giltig tabell för 2026, uppdelad på flera sidor. `TestServer` får en konstruktor med tabelladressen.
- Den första förhandsgranskningen hämtar tabellen, med anropen räknade i låtsastjänsten, och räknar fram skatten. Nästa förhandsgranskning gör inga nya anrop.
- När låtsastjänsten svarar 500 eller saknar året blir det `tax_table_unavailable`. En rad med manuell skatt fungerar ändå.
- `SetEmployeeTax` med tabell 43 ger `invalid_tax_table`.

### E2E (`e2e/`)
E2E-servern har ingen internetåtkomst, och tabellvägen täcks av servertesterna.
- Lägg till en anställd med fast procent 30 % och skapa en körning med tomt skattefält. Förhandsgranskningen visar skatten och "30 %". Skriv sedan över skatten och förhandsgranska igen, så visas "Manuell".
- En anställd utan inställning ger "Ange skatt för {namn}.".
- Redigera en anställd till "Skattetabell", "Tabell 33", "Kolumn 1". Tabellen över anställda visar då "Tabell 33, kol 1".

### Avslutning
- `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
- `make dist` håller sig inom `WASM_BUDGET`.
- AGENTS.md uppdateras med:
  - att `tax_tables` är referensdata från Skatteverket, inte händelser,
  - `DORIS_TAX_TABLES_URL`,
  - att servern gör den utgående hämtningen,
  - de nya felkoderna,
  - att skattegrunden låses i körningen.

## Utanför omfattningen
- Engångsskatt för bonus och andra engångsbelopp (överskrivning räcker så länge)
- Jämkning med fast belopp
- Skatt på förmåner
- Tabeller för vecko- och tvåveckolön
- Förslag på tabell utifrån den anställdas kommun
- Kontroll av A-skattsedel
- AGI
