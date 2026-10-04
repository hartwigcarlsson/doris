# Doris – Steg 11: Arbetsgivardeklaration på individnivå (AGI)

## Kontext
Steg 9 och 10 räknar fram och bokför löner. Varje månad som lön betalas ut måste arbetsgivaren också lämna en arbetsgivardeklaration på individnivå (AGI) till Skatteverket. Den har två delar:
- en huvuduppgift med summa arbetsgivaravgifter och summa skatteavdrag,
- en individuppgift per betalningsmottagare med ersättning och avdragen skatt.

I det här steget tar Doris fram AGI per månad ur de bokförda lönekörningarna och skapar en XML-fil som användaren laddar upp i Skatteverkets e-tjänst. Användaren markerar sedan månaden som inlämnad. Doris kommer ihåg vad som lämnades, visar när en månad har ändrats och skapar då en rättelsefil med de ändrade individuppgifterna, en ny huvuduppgift och borttag.

### Fattade beslut
| Område | Beslut |
|---|---|
| Underlag | Bara *bokförda* lönekörningar, alltså de vars bokföring inte är backad, med utbetalningsdag i månaden (ÅÅÅÅMM). |
| Individuppgift | En per anställd och månad. Flera körningar i samma månad slås ihop. |
| Belopp | Hela kronor, avrundat nedåt (öre kastas), på summan per anställd och månad. |
| Inlämning | Doris skapar filen, och användaren laddar upp den hos Skatteverket och markerar sedan månaden som inlämnad. Det sparas som en händelse med vem och när, och med en ögonblicksbild av vad som lämnades. Doris skickar inget direkt till Skatteverket. |
| Rättelser | Status räknas fram genom att jämföra med den senaste inlämningen. En ändrad månad får en fil med ändrade och nya individuppgifter, borttag för anställda som inte längre har lön den månaden och en ny huvuduppgift. |
| Specifikationsnummer | Ett per anställd: hennes plats i personalregistret (anställningsordning, från 1). Samma i alla månader, före och efter inlämning. |
| Arbetsgivaravgifter | Summan (fältkod 487) räknas som Skatteverket gör: per avgiftssats, på summerat underlag, avrundat nedåt till hela kronor. Skillnaden mot verifikationernas 2731, som är avrundade per rad till öre, visas men bokförs inte. Skatteverket godtar mindre beräkningsdifferenser (kontroll B_006). |
| Kontaktperson | Sparas per företag med namn, telefon och e-post. Filen kräver dem både för avsändarens tekniska kontakt och för arbetsgivarens kontaktperson, och samma person används för båda. |
| Format | Skatteverkets XML-schema för arbetsgivardeklaration 1.1, enligt teknisk beskrivning 1.1.18.2. Filen skrivs som text och ingen XML-modul läggs till. |

Källor: Skatteverket, "Teknisk beskrivning (1.1.18.2) för arbetsgivardeklaration", med bilagorna Fältlista 1.1.18.1, Summering och beräkningsregler, Kontroller 1.1.18.1, Exempelfiler och `arbetsgivardeklaration_1.1.xsd` / `arbetsgivardeklaration_component_1.1.xsd`.

## Skatteverkets format (det som används)
- **Rot:** `<Skatteverket omrade="Arbetsgivardeklaration" xmlns="http://xmls.skatteverket.se/se/skatteverket/da/instans/schema/1.1" xmlns:agd="http://xmls.skatteverket.se/se/skatteverket/da/komponent/schema/1.1" …>`.
- **`agd:Avsandare`:**
  - `Programnamn` ("Doris")
  - `Organisationsnummer` (12 tecken, arbetsgivarens ID)
  - `TekniskKontaktperson` (`Namn`, `Telefon`, `Epostadress`)
  - `Skapad` (xs:dateTime, svensk lokal tid utan zon, som i exemplen)
- **`agd:Blankettgemensamt/agd:Arbetsgivare`:** `AgRegistreradId` och en `Kontaktperson` (`Namn`, `Telefon`, `Epostadress`).
- **En `agd:Blankett` per uppgift.** Varje blankett har:
  - `Arendeinformation`, med `Arendeagare` (arbetsgivarens ID) och `Period` (ÅÅÅÅMM),
  - `Blankettinnehall`, som innehåller antingen en `HU` eller en `IU`.
- **HU (huvuduppgift):**
  - `ArbetsgivareHUGROUP/AgRegistreradId` (fältkod 201)
  - `RedovisningsPeriod` (006)
  - `SummaArbAvgSlf` (487)
  - `SummaSkatteavdr` (497)
- **IU (individuppgift):**
  - `ArbetsgivareIUGROUP/AgRegistreradId` (201)
  - `BetalningsmottagareIUGROUP/BetalningsmottagareIDChoice/BetalningsmottagarId` (215, 12 tecken)
  - `RedovisningsPeriod` (006)
  - `Specifikationsnummer` (570, 1–9 999 999 999)
  - `KontantErsattningUlagAG` (011)
  - `AvdrPrelSkatt` (001)

  Belopp skrivs som heltal 0–9 999 999 999.
- **IU med borttag:** samma identifierare (201, 215, 006, 570) samt `Borttag` (205) med värdet `1`, och inga andra fält (kontroll S_003).
- **Arbetsgivarens ID (12 tecken):**
  - Organisationsnummer får prefixet `16`, till exempel `165560160680`.
  - En enskild firma använder ägarens personnummer (det 10-siffriga `OrgNr`) med sekel: `19` om ÅÅ är större än årets två sista siffror, annars `20`.
  - `// ponytail:` Seklet för en enskild firma antar en ägare under 100 år. Ett fält för fullständigt personnummer blir aktuellt om det inte räcker.
- **Avgiftsregler (fliken Ålderskategorier/Procentsatser, 2026)**, där `Y` är periodens år:
  - **Alder_1:** full avgift 31,42 %.
  - **Alder_2:** födda 1938–(Y−68), 10,21 %.
  - **Alder_3:** födda 1937 eller tidigare, 0 %.
  - **Alder_5:** född (Y−23)–(Y−19) med period 202604–202709. 20,81 % på ersättningen upp till 25 000 kr per individuppgift (IK7001/IK9001), och full avgift på resten.

  Reglerna är desamma som `doris_payroll::domain::employer_fee`.

## Domän (`crates/payroll/src/agi.rs`, ren och utan I/O)

### Värdeobjekt
- **`Period(i32)`** i formen ÅÅÅÅMM, med `Period::parse(&str)` ("202610"; månad 01–12; annars `InvalidPeriod`) och `Period::of(date)`.
- **`AgiContact { name, phone, email }`**, där `AgiContact::parse(name, phone, email)` följer schemat:
  - namn: 1–50 tecken,
  - telefon: 1–20 tecken och inte bara blanksteg,
  - e-post: 5–254 tecken som matchar Skatteverkets EPOST-mönster `[a-zA-Z0-9_]+([-+.'][a-zA-Z0-9_]+)*@[a-zA-Z0-9_]+([-.][a-zA-Z0-9_]+)*\.[a-zA-Z0-9_]+([-.][a-zA-Z0-9_]+)*`. Mönstret skrivs som en liten handskriven kontroll, utan regex-beroende.

  Strängarna trimmas. Annars blir det `InvalidAgiContact`. Fel tecken, som `<` och `>`, avvisas av schemamönstret TEXT50/TEXT20 och avvisas därför också här.
- **`AgiLine { employee_id, specification_number: u64, gross: i64, tax: i64 }`**, med belopp i hela kronor.

### Händelser (ström `payroll-{company_id}`, `schema_version` 1)
```rust
/// Who Skatteverket may contact about the company's AGI.
AgiContactChanged { contact: AgiContact },
/// What was submitted for a period, as the user confirms after uploading
/// the file. The latest one per period is what Skatteverket has.
AgiMonthSubmitted {
    period: Period,
    lines: Vec<AgiLine>,   // the individuppgifter now in force
    fee_sum: i64,          // FK487
    tax_sum: i64,          // FK497
},
```

### Tillstånd
`Payroll` får fälten `agi_contact: Option<AgiContact>` och `agi_submissions: BTreeMap<Period, (Vec<AgiLine>, i64, i64)>`. Det senaste per period gäller. Varje anställds specifikationsnummer är hennes plats i `Payroll.employees` (1-baserad). Listan växer bara och sorteras aldrig om, så numret ändras aldrig.

### Beräkning
- **`agi_lines(payroll, period) -> Vec<AgiLine>`:**
  1. För varje bokförd körning med `Period::of(pay_date) == period` summeras de låsta radernas `gross` och `tax` per anställd.
  2. Summorna görs om till hela kronor (`/ 100`).
  3. Specifikationsnummer sätts enligt ovan: platsen i registret.

  Anställda vars summor båda är 0 tas inte med. En rad skapas även när bara den ena summan är större än 0.
- **`agi_fee_sum(payroll, period, &lines) -> i64`:**
  1. Varje rad delas upp per avgiftssats, med den anställdas födelseår och periodens första dag (åldersreglerna gäller per år). För Alder_5 i nedsättningsperioden går `min(gross, 25 000)` till 20,81 % och resten till 31,42 %.
  2. Underlaget summeras per sats.
  3. Summan är `floor(Σ underlag × sats)`, räknat i hundradels procent med heltal.
- **Kontrollvärde:** `agi_tax_sum` är `Σ tax`. `booked_fees(payroll, period)` är summan av de låsta radernas `fee` i öre, alltså det som bokförts på 2731, och visas för jämförelse.
- **`agi_month(payroll, period) -> AgiMonth`:**
  - `AgiMonth { period, lines, fee_sum, tax_sum, booked_fees, changes: Vec<(employee_id, Change)>, status }`.
  - `Change` är `New`, `Changed`, `Removed` eller `Unchanged`, jämfört med den senaste inlämningen. `Removed` används för den som finns i inlämningen men inte i `lines`.
  - `status` är:
    - `NotSubmitted` om det inte finns någon inlämning,
    - `Submitted` om allt är `Unchanged` och summorna är lika,
    - `Changed` annars.
- **`submit_agi_month(payroll, period) -> Result<PayrollEvent>`:**
  - Inga rader och ingen tidigare inlämning ger `AgiPeriodEmpty`.
  - Status `Submitted` ger `AgiUnchanged`.
  - Saknad kontaktperson ger `AgiContactMissing`.
  - Annars blir det `AgiMonthSubmitted { period, lines, fee_sum, tax_sum }`. En månad där alla har tagits bort kan alltså lämnas in med tomma rader och summor 0, och då blir det borttag för alla.
- **`set_agi_contact(payroll, contact)`:** samma kontakt ger inga händelser, annars `AgiContactChanged`.

### Filen (`agi_xml`)
`agi_xml(month: &AgiMonth, employer_id: &str, contact: &AgiContact, personal_ids: &HashMap<Uuid, String>, created: DateTime) -> Result<String, DomainError>`. `AgiContactMissing` hanteras av den som anropar, och raderna kommer från `month`.
- `Avsandare` och `Blankettgemensamt` skrivs enligt formatet ovan.
- Därefter en HU-blankett med summorna för perioden.
- Därefter en IU-blankett för varje rad vars `Change` är `New` eller `Changed`. För en månad som inte lämnats in tidigare är det alla rader.
- Därefter en IU med borttag för varje `Removed`, med det tidigare specifikationsnumret och personnumret.
- `Unchanged` skrivs inte.

Texten escapas för `&`, `<`, `>`, `"` och `'`. Filen är UTF-8 med XML-deklaration och har filnamnet `AGI_{arbetsgivar-id}_{ÅÅÅÅMM}.xml`. En fil kan skapas för alla statusar. För `Submitted` innehåller den bara HU, som då är oförändrad, och knappen döljs i gränssnittet.

## Lagring
### Migration `migrations/0014_agi.sql`
```sql
-- Projections of AgiContactChanged and AgiMonthSubmitted. Rebuildable.
CREATE TABLE agi_contacts (
    company_id TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    phone      TEXT NOT NULL,
    email      TEXT NOT NULL
);

-- Every submission, in order; the latest per period is in force.
CREATE TABLE agi_submissions (
    company_id   TEXT    NOT NULL,
    period       INTEGER NOT NULL,
    submitted_at TEXT    NOT NULL,
    submitted_by TEXT    NOT NULL,
    lines        TEXT    NOT NULL, -- JSON Vec<AgiLine>
    fee_sum      INTEGER NOT NULL,
    tax_sum      INTEGER NOT NULL
);
```
Projektionerna läggs till i `rebuild_projections`. Läsningarna bygger `AgiMonth` från domänens tillstånd, det vill säga strömmen plus `reversed`, precis som `book_payroll_run` gör. Projektionerna används för listan och visar historiken med vem och när.

### `doris-payroll` (lib)
- `agi_contact(pool, company_id, actor) -> Result<Option<AgiContact>>`
- `set_agi_contact(pool, company_id, actor, name, phone, email)`
- `agi_months(pool, company_id, actor) -> Result<Vec<AgiMonthSummary>>`, för alla perioder med bokförda körningar eller en inlämning, nyast först
- `agi_month(pool, company_id, actor, period) -> Result<AgiMonth>`
- `agi_file(pool, company_id, actor, period, created) -> Result<(String /*name*/, String /*xml*/, String /*fingerprint*/)>`, som läser företagets organisationsnummer och juridiska form från `doris_company`
- `submit_agi_month(pool, company_id, actor, period, fingerprint)`, som kör beslutet och lägger till händelsen i en `BEGIN IMMEDIATE`

## API (`payroll.proto`, `PayrollService`)
```proto
rpc GetAgiContact(GetAgiContactRequest) returns (AgiContact);   // empty fields when unset
rpc SetAgiContact(SetAgiContactRequest) returns (SetAgiContactResponse);
rpc ListAgiMonths(ListAgiMonthsRequest) returns (ListAgiMonthsResponse);
rpc GetAgiMonth(AgiMonthRef) returns (AgiMonth);
rpc ExportAgiFile(AgiMonthRef) returns (AgiFile);
rpc MarkAgiSubmitted(MarkAgiSubmittedRequest) returns (MarkAgiSubmittedResponse);

message AgiContact { string name = 1; string phone = 2; string email = 3; }
message GetAgiContactRequest { string company_id = 1; }
message SetAgiContactRequest { string company_id = 1; AgiContact contact = 2; }
message SetAgiContactResponse {}
message AgiMonthRef { string company_id = 1; string period = 2; } // "ÅÅÅÅMM"
enum AgiStatus {
  AGI_STATUS_UNSPECIFIED = 0;
  AGI_STATUS_NOT_SUBMITTED = 1;
  AGI_STATUS_SUBMITTED = 2;
  AGI_STATUS_CHANGED = 3;
}
message AgiMonthSummary {
  string period = 1;
  int64 gross = 2;     // kronor
  int64 tax_sum = 3;   // FK497
  int64 fee_sum = 4;   // FK487
  AgiStatus status = 5;
  string submitted_at = 6; // latest, if any
}
message ListAgiMonthsRequest { string company_id = 1; }
message ListAgiMonthsResponse { repeated AgiMonthSummary months = 1; }
enum AgiChange {
  AGI_CHANGE_UNSPECIFIED = 0;
  AGI_CHANGE_NEW = 1;
  AGI_CHANGE_CHANGED = 2;
  AGI_CHANGE_REMOVED = 3;
  AGI_CHANGE_UNCHANGED = 4;
}
message AgiLine {
  string employee_id = 1;
  string employee_name = 2;
  string personal_identity_number = 3; // ÅÅÅÅMMDD-NNNN
  uint64 specification_number = 4;
  int64 gross = 5;                     // kronor (FK011)
  int64 tax = 6;                       // kronor (FK001)
  AgiChange change = 7;
}
message AgiMonth {
  AgiMonthSummary summary = 1;
  repeated AgiLine lines = 2;          // incl. removed ones
  int64 booked_fees = 3;               // öre on 2731 for comparison
  string fingerprint = 4;
}
message AgiFile { string file_name = 1; string xml = 2; string fingerprint = 3; }
message MarkAgiSubmittedRequest { string company_id = 1; string period = 2; string fingerprint = 3; }
message MarkAgiSubmittedResponse {}
```
Nya felkoder, mappade i `crates/server/src/payroll.rs`:
- `invalid_period` och `invalid_agi_contact` mappas till InvalidArgument.
- `agi_contact_missing`, `agi_period_empty`, `agi_unchanged` och `agi_file_outdated` mappas till FailedPrecondition.

`fingerprint` är hex SHA-256 över det som `AgiMonthSubmitted` skulle spara (raderna, fee_sum och tax_sum). `MarkAgiSubmitted` tar fingeravtrycket från den senast nedladdade filen, annars från månaden som visas; stämmer det inte (eller är tomt) blir det `agi_file_outdated`, så bara det användaren har sett markeras.

Personnummer loggas aldrig. Filen innehåller personnummer, och det är avsikten, men den skickas bara till den inloggade medlemmen.

## Frontend
### `/agi`, Arbetsgivardeklaration
Menylänken "Arbetsgivardeklaration" ligger i bokföringsraden, efter "Anställda".
- **Kort "Kontaktperson"**, för den som Skatteverket kan kontakta om arbetsgivardeklarationen:
  - Fälten Namn, Telefon och E-post samt knappen "Spara kontaktperson".
  - Om inget är sparat förifylls Namn och E-post med den inloggade användarens uppgifter.
- **Tabell:** Period (till exempel "oktober 2026"), Ersättning, Skatteavdrag, Arbetsgivaravgifter och Status. Status visas som "Ej deklarerad", "Deklarerad" eller "Ändrad".
- **En rad kan expanderas.** Den visar då:
  - Individuppgifterna: Anställd, Personnummer, Spec.nr, Ersättning, Skatt och Ändring ("Ny", "Ändrad", "Borttag" eller "–").
  - Texten "Avgifter enligt bokföringen: {kr}". Skiljer det sig från huvuduppgiften läggs " (skillnad {kr} från avrundning)" till.
  - Knappen "Ladda ner fil", när status inte är Deklarerad. Den anropar `ExportAgiFile` och sparar filen via en `Blob` och en `<a download>`.
  - Knappen "Markera som inlämnad", med bekräftelsetexten "Markera som inlämnad när filen är uppladdad hos Skatteverket." och knappen "Bekräfta". Den visas också bara när status inte är Deklarerad.
- **Utan kontaktperson** är båda knapparna avstängda, med förklaringen "Spara en kontaktperson först."

### `src/errors.rs`
- `invalid_period`: "Ogiltig period."
- `invalid_agi_contact`: "Ange namn (högst 50 tecken), telefon (högst 20 tecken) och en giltig e-postadress."
- `agi_contact_missing`: "Spara en kontaktperson först."
- `agi_period_empty`: "Det finns inga bokförda löner den månaden."
- `agi_unchanged`: "Månaden är redan inlämnad och har inte ändrats."
- `agi_file_outdated`: "Filen är inaktuell, ladda ner den igen."

## Tester
Varje beteende utvecklas med TDD: rött, grönt, refaktorering och commit.

### Domän (`crates/payroll/tests/agi.rs`)
- **`Period::parse`:** "202610" godtas, medan "202613", "2026-10", "26010" och "" avvisas. `Period::of` ger rätt period för ett datum.
- **`AgiContact::parse`:** gränserna 50 och 51 tecken för namn och 20 och 21 för telefon, en telefon med bara blanksteg, och e-post enligt mönstret, med både giltiga och ogiltiga exempel.
- **`agi_lines`:**
  - En bokförd körning ger en rad per anställd.
  - En körning som är öppen, färdigställd eller backad räknas inte.
  - Två körningar i samma månad slås ihop.
  - En körning i en annan månad räknas inte.
  - Öre avrundas nedåt (35 000,99 blir 35 000).
- **`agi_fee_sum`:**
  - Åsa (född 1980) och Bo (född 1950), oktober 2026, med 35 000 och 20 000 kr. Avgiften blir floor(35 000 × 31,42 % + 20 000 × 10,21 %) = 10 997 + 2 042 = 13 039.
  - En ung anställd (född 2005) med 30 000 kr i oktober 2026 ger floor(25 000 × 20,81 % + 5 000 × 31,42 %) = 6 773.
  - Samma unga anställda i mars 2026 ger 30 000 × 31,42 % = 9 426.
  - Två körningar för den unga i samma månad, med 20 000 kr var, ger ett underlag på 40 000 i en individuppgift. Det blir floor(25 000 × 20,81 % + 15 000 × 31,42 %) = 5 202,5 + 4 713, alltså 9 915.
- **Status och ändringar:**
  - Ingen inlämning ger `NotSubmitted`, och alla rader är `New`.
  - Efter `AgiMonthSubmitted` blir det `Submitted`, och alla rader är `Unchanged`.
  - Om en körning backas blir det `Changed`. Den anställda blir `Removed` om hon inte har någon annan körning, annars `Changed`.
  - En ny körning ger `Changed` och `New` för en ny anställd.
- **Specifikationsnummer:**
  - De är platsen i registret, från 1.
  - De flyttar sig inte när en tidigare månad markeras inlämnad.
  - En ny anställd tar aldrig en borttagens nummer.
  - En borttagen behåller sitt nummer.
- **`submit_agi_month`:** felen `AgiPeriodEmpty`, `AgiUnchanged` och `AgiContactMissing`. En månad där alla har tagits bort kan lämnas in.
- **Arbetsgivarens ID:**
  - Ett aktiebolag med `5560160680` ger `165560160680`.
  - En enskild firma med `8001011231` ger `198001011231`, och med `0506151232` ger den `200506151232`, när året är 2026.
- **`agi_xml`:**
  - En jämförelse mot den förväntade filen `crates/payroll/tests/fixtures/agi_202610.xml`, uppbyggd som Skatteverkets exempelfil 01 för vanliga löntagare men med Doris data.
  - En fil för en ändrad månad innehåller HU, ändrade IU och IU med borttag (bara 201, 215, 006, 570 och 205), men inga oförändrade.
  - Escaping av `Åsa & Bo <AB>` i kontaktens namn.

### Lagring och server
- Inlämningen sparas med vem och när, och projektionerna byggs om från `read_all`.
- **gRPC-Web-flödet** (`crates/server/tests/payroll.rs`):
  1. En anställd och en bokförd körning.
  2. `ExportAgiFile` utan kontaktperson ger `agi_contact_missing`.
  3. `SetAgiContact` och sedan `ExportAgiFile`, som ger XML med `<agd:SummaSkatteavdr faltkod="497">` och personnumret.
  4. `MarkAgiSubmitted`, och `ListAgiMonths` visar Submitted.
  5. `MarkAgiSubmitted` en gång till ger `agi_unchanged`.
  6. Backa körningen ger status Changed, och filen innehåller `Borttag`.
- **Felkoder:** ogiltig period och ogiltig kontaktperson.

### E2E (`e2e/tests/agi.spec.ts`)
- Bokför en körning och spara en kontaktperson. Månaden visas som "Ej deklarerad".
- "Ladda ner fil" ger en nedladdning med namnet `AGI_…_ÅÅÅÅMM.xml` som innehåller personnumret.
- "Markera som inlämnad" ger "Deklarerad".
- Backa körningen ger "Ändrad", och raden visar "Borttag".

### Avslutning
- Verifiera med xmllint: en skapad fil valideras mot Skatteverkets XSD (`xmllint --noout --schema arbetsgivardeklaration_1.1.xsd fil.xml`, med komponentschemat bredvid). Det görs en gång i planens slutkontroll och dokumenteras, men blir inget enhetstest. Schemana ligger inte i repot.
- `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
- AGENTS.md uppdateras med AGI:
  - underlaget är bokförda körningar per utbetalningsmånad,
  - inlämningen markeras för hand och sparas som händelse,
  - specifikationsnumren är stabila,
  - borttag skapas automatiskt,
  - de nya felkoderna.

## Utanför omfattningen
- Att skicka filen direkt till Skatteverket via API, vilket kräver e-legitimation och behörighet
- Förmåner (fältkod 012, 013 med flera)
- Frånvarouppgifter
- SINK och A-SINK
- Utsända anställda och socialförsäkringskonventioner
- Växa-stöd
- Bokföring av skattekontot (2710 och 2731 mot 1630)
- Deklarationer äldre än sex år
- Arbetsplatsadress (fältkod 245 och 246), som bara krävs i särskilda fall
