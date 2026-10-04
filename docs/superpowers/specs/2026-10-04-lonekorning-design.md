# Doris – Steg 8: Anställda och lönekörning

## Kontext
Doris har i dag kontoplan, verifikationer, huvudbok, bokslut och underlag, men inget stöd för löner. Det här steget inför lönehantering i sin minsta form: ett register över anställda och en månatlig lönekörning som räknar fram arbetsgivaravgift och nettolön och bokförs som en verifikation. Preliminärskatten skrivs in för hand av användaren, som hämtar beloppet från Skatteverkets skattetabell.

Lönedatan sparas per anställd och körning, eftersom de följande stegen bygger på den:
1. **Nästa steg:** skattetabeller. Varje anställd får tabell och kolumn, och skatten räknas fram från Skatteverkets tabell i stället för att skrivas in.
2. Därefter AGI, alltså arbetsgivardeklaration på individnivå, samt lönebesked, semester och förmåner.

### Fattade beslut
| Område | Beslut |
|---|---|
| Placering | Ny crate `doris-payroll` med egen event-ström. Den anropar `doris_ledger::record_voucher_in` i samma transaktion, så ledgern vet inget om löner. |
| Åtkomst | Alla medlemmar i företaget, som för bokföringen. En särskild löneroll ligger utanför omfattningen. |
| Lönetyp | Månadslön. Bruttolönen förifylls från registret och kan ändras i körningen. |
| Skatt | Skrivs in i kronor och öre per anställd och körning. Den får inte vara negativ eller större än bruttolönen. |
| Arbetsgivaravgift | Räknas fram i koden från födelseåret och utbetalningsdagen (se nedan). |
| Två steg | En körning **färdigställs** först. Det går när som helst, även före utbetalningsdagen, och låser belopp, avgifter och konton. Den **bokförs** sedan med knappen "Bokför", tidigast på utbetalningsdagen. Bokföringen görs av en person och sker aldrig automatiskt, eftersom Doris inte har några bakgrundsjobb och behandlingshistoriken ska visa vem som bokförde. |
| Bokföring | En verifikation per körning, daterad på utbetalningsdagen, mot 1930. Den bygger på de belopp och konton som låstes när körningen färdigställdes. Ledgerns regler gäller oförändrade: inga stängda år och inga inaktiva konton. |
| Flera körningar | Tillåtna i samma månad, till exempel för en extra utbetalning. |
| Makulering | En färdigställd körning som inte har bokförts makuleras med "Makulera" (`PayrollRunCancelled`). En bokförd körning makuleras genom att verifikationen rättas i grundboken som vanligt. Det finns ingen egen funktion för det. |
| Personnummer | Personuppgift. Loggas aldrig och skickas aldrig till någon extern tjänst. Kan inte ändras på en anställd. |

## Domän (`crates/payroll/src/domain.rs`, ren och utan I/O)

### Värdeobjekt
- `PersonalIdentityNumber::parse(raw)` godtar `ÅÅÅÅMMDDNNNN` och `ÅÅÅÅMMDD-NNNN`, efter att blanksteg har trimmats. Tiosiffriga former avvisas, eftersom seklet då är okänt. Datumdelen måste vara ett giltigt datum, där ett samordningsnummer har dagen + 60 (61–91). Luhn-kontrollen räknas på de sista tio siffrorna. Fel ger `InvalidPersonalIdentityNumber`. Numret lagras som tolv siffror. `birth_year()` ger de fyra första siffrorna och `formatted()` ger `ÅÅÅÅMMDD-NNNN`. Luhn-funktionen kopieras från `doris_company::domain` eller görs publik där. Det avgörs i planen, inte med ett nytt beroende.
- `EmployeeName::parse(raw)` trimmar namnet, som måste ha 1–100 tecken. Annars blir det `InvalidEmployeeName`.
- `SalaryAccount` är `7010`, `7210` eller `7220`, med `7210` som standard. Annat ger `InvalidSalaryAccount`.
- Belopp anges i öre (`i64`), som i ledgern. Månadslönen måste vara > 0, annars `InvalidSalary`.

### Event (ström `payroll-{company_id}`, `schema_version` 1)
```rust
#[serde(tag = "type")]
pub enum PayrollEvent {
    EmployeeAdded {
        employee_id: Uuid,
        name: EmployeeName,
        personal_identity_number: PersonalIdentityNumber,
        monthly_salary: i64,
        salary_account: SalaryAccount,
    },
    EmployeeUpdated {
        employee_id: Uuid,
        name: EmployeeName,
        monthly_salary: i64,
        salary_account: SalaryAccount,
    },
    EmployeeDeactivated { employee_id: Uuid },
    /// Amounts, fees and accounts locked; may precede the pay date.
    PayrollRunFinalized {
        payroll_run_id: Uuid,
        pay_date: Date,
        text: String,
        lines: Vec<PayrollRunLine>,
    },
    /// Booked as a voucher, on or after the pay date.
    PayrollRunBooked {
        payroll_run_id: Uuid,
        fiscal_year_start: Date,
        voucher_number: u32,
    },
    /// Withdrawn before it was booked.
    PayrollRunCancelled { payroll_run_id: Uuid },
}

pub struct PayrollRunLine {
    pub employee_id: Uuid,
    /// Copied from the employee when finalized, so a later change of
    /// account doesn't move the booking.
    pub salary_account: SalaryAccount,
    pub gross: i64,
    pub tax: i64,
    /// Basis points of the part under the youth cap, see `employer_fee`.
    pub fee_rate: u32,
    pub fee: i64,
    pub net: i64,
}
```
Vem som gjorde vad och när finns i eventets metadata, som för andra event. `fee_rate` sparas så att en körning alltid kan läsas som den räknades, även om reglerna ändras senare.

### Tillstånd
`Payroll { employees: Vec<Employee>, runs: Vec<PayrollRun> }`, där `Employee` har `active: bool`. `PayrollRun` har status `Finalized`, `Booked(VoucherRef)` eller `Cancelled`. `Payroll::apply(event)` är `evolve`. Ström-ID:t är per företag, så både anställda och körningar finns i samma tillstånd. Det gör det enkelt att kontrollera dubbletter och avgiftstaket.

### Beslut
- `add_employee(payroll, cmd)`: om personnumret redan finns i företaget, även hos en inaktiv anställd, blir det `DuplicateEmployee`. UNIQUE på projektionen säkrar samma sak.
- `update_employee(payroll, cmd)`: en anställd som saknas ger `EmployeeNotFound` och en inaktiv ger `EmployeeInactive`. Om inget ändras returneras inga event.
- `deactivate_employee(payroll, id)`: en anställd som saknas ger `EmployeeNotFound`. Är den redan inaktiv returneras inga event.
- `compute_payroll_run(payroll, reversed, cmd) -> Result<ComputedPayrollRun>`, där `reversed: &HashSet<Uuid>` är de bokförda körningar vars verifikation har rättats, och `cmd = { pay_date, text, lines: [{ employee_id, gross, tax }] }`:
  - Inga rader ger `EmptyPayrollRun`, och samma anställd två gånger ger `DuplicatePayrollRunLine`.
  - En anställd som saknas ger `EmployeeNotFound` och en inaktiv ger `EmployeeInactive`.
  - `gross` ≤ 0 ger `InvalidSalary`. `tax` < 0 eller `tax` > `gross` ger `InvalidTax`.
  - En tom text efter trimning ersätts med "Lön {månad år}", till exempel "Lön oktober 2026".
  - Varje rad räknas med `employer_fee`, och `net = gross - tax`. `salary_account` kopieras från den anställda.
  - Resultatet är raderna plus verifikationens rader, enligt `voucher_lines` nedan.
  - Det här används både av förhandsgranskningen och när körningen färdigställs. Utbetalningsdagen får ligga i framtiden.
- `finalize_payroll_run(computed, payroll_run_id)` ger `PayrollRunFinalized`.
- `voucher_lines(lines) -> Vec<doris_ledger::domain::VoucherLine>` grupperar per konto:
  - debet på varje `salary_account` med summan av bruttolönerna,
  - kredit på 2710 med summan av skatterna,
  - kredit på 1930 med summan av nettolönerna,
  - debet på 7510 och kredit på 2731 med summan av avgifterna.

  Rader med beloppet 0 tas inte med.
- `book_payroll_run(payroll, payroll_run_id, today) -> Result<RecordVoucher>`:
  - En körning som saknas ger `PayrollRunNotFound`.
  - En redan bokförd ger `PayrollRunBooked` och en makulerad ger `PayrollRunCancelled`.
  - `pay_date` > `today` ger `PayrollRunNotDue`.
  - Annars returneras `RecordVoucher { date: pay_date, text, lines: voucher_lines(lines) }`. Anställdas nuvarande status spelar ingen roll, så en körning kan bokföras även om någon har inaktiverats efter att den färdigställdes.
- `booked(payroll_run_id, voucher)` ger `PayrollRunBooked`.
- `cancel_payroll_run(payroll, payroll_run_id)`:
  - En körning som saknas ger `PayrollRunNotFound`.
  - En bokförd ger `PayrollRunBooked`, eftersom den rättas i grundboken.
  - En redan makulerad ger inga event.
  - Annars blir det `PayrollRunCancelled`.

### Arbetsgivaravgift
`employer_fee(birth_year, pay_date, gross, earlier_gross_same_month) -> (fee_rate, fee)`. Året `Y` är utbetalningsdagens år.

| Villkor | Sats |
|---|---|
| `birth_year` ≤ 1937 | 0 % |
| `birth_year` ≤ `Y` − 68, alltså fyllt 67 vid årets ingång | 10,21 % (endast ålderspensionsavgift) |
| `Y` − 23 ≤ `birth_year` ≤ `Y` − 19, och `pay_date` från 2026-04-01 till 2027-09-30 | 20,81 % på belopp upp till 25 000 kr per kalendermånad, 31,42 % på resten |
| Övriga | 31,42 % |

- Satserna anges i baspunkter (3142, 1021, 2081). `fee = round_half_up((under_cap * rate + over_cap * 3142) / 10000)` i öre, en gång per rad.
- Taket på 25 000 kr per kalendermånad delas mellan alla körningar med utbetalningsdag i samma månad. `earlier_gross_same_month` är summan av den anställdas bruttolön i tidigare färdigställda körningar i den månaden. Körningar som är makulerade eller rättade räknas inte. En rättelse syns inte i lönernas egen ström, så lib-lagret läser de rättade körningarna (join mot `vouchers.corrects`) och skickar dem som `reversed` till `compute_payroll_run`.
- `// ponytail:` Reglerna ligger i koden, giltiga från 2026. En ändrad sats kräver en ny version av Doris. Satser per datum i en tabell blir aktuellt först när de ändras oftare än Doris släpps.
- AGI räknar avgiften på summan per sats och avrundar nedåt till hela kronor. Verifikationen avrundar per rad till hela öre. Skillnaden hanteras i AGI-steget.

Källa: Skatteverket, "Arbetsgivaravgifter" (2026).

## Lagring och transaktioner

### Migration `migrations/0009_payroll.sql`
```sql
-- Projections of the payroll-{company_id} streams. Rebuildable from events.
CREATE TABLE employees (
    company_id               TEXT    NOT NULL,
    employee_id              TEXT    NOT NULL,
    name                     TEXT    NOT NULL,
    personal_identity_number TEXT    NOT NULL,
    monthly_salary           INTEGER NOT NULL,
    salary_account           INTEGER NOT NULL,
    active                   INTEGER NOT NULL,
    PRIMARY KEY (company_id, employee_id),
    UNIQUE (company_id, personal_identity_number)
);

CREATE TABLE payroll_runs (
    company_id        TEXT    NOT NULL,
    payroll_run_id    TEXT    NOT NULL,
    pay_date          TEXT    NOT NULL,
    text              TEXT    NOT NULL,
    finalized_at      TEXT    NOT NULL,
    finalized_by      TEXT    NOT NULL,
    -- Set together by PayrollRunBooked.
    fiscal_year_start TEXT,
    voucher_number    INTEGER,
    booked_at         TEXT,
    booked_by         TEXT,
    cancelled_at      TEXT,
    cancelled_by      TEXT,
    PRIMARY KEY (company_id, payroll_run_id),
    FOREIGN KEY (company_id, fiscal_year_start, voucher_number)
        REFERENCES vouchers (company_id, fiscal_year_start, number),
    CHECK (voucher_number IS NULL OR cancelled_at IS NULL)
);

CREATE TABLE payroll_run_lines (
    company_id     TEXT    NOT NULL,
    payroll_run_id TEXT    NOT NULL,
    employee_id    TEXT    NOT NULL,
    salary_account INTEGER NOT NULL,
    gross          INTEGER NOT NULL,
    tax            INTEGER NOT NULL,
    fee_rate       INTEGER NOT NULL,
    fee            INTEGER NOT NULL,
    net            INTEGER NOT NULL,
    PRIMARY KEY (company_id, payroll_run_id, employee_id),
    FOREIGN KEY (company_id, payroll_run_id)
        REFERENCES payroll_runs (company_id, payroll_run_id)
);
```
`reversed` lagras inte. Den räknas ut när den läses: en bokförd körning är rättad om det finns en verifikation vars `corrects` pekar på körningens verifikation.

### Skrivflöde (`crates/payroll/src/lib.rs`)
Varje skrivning sker i en `BEGIN IMMEDIATE` (`doris_eventstore::begin`) och följer samma ordning som ledgern: medlemskap (`doris_company::get_company_in`), ladda strömmen, besluta, `append` och projicera.
- `add_employee`, `update_employee` och `deactivate_employee` är enkla kommandon.
- `preview_payroll_run(pool, company_id, actor, cmd)` laddar tillståndet, hämtar de rättade körningarna och kör `compute_payroll_run`. Den skriver ingenting.
- `finalize_payroll_run(pool, company_id, actor, cmd) -> Result<Uuid>` kör `compute_payroll_run` i transaktionen och lägger till `PayrollRunFinalized`. Ingen verifikation skapas.
- `cancel_payroll_run(pool, company_id, actor, payroll_run_id)` lägger till `PayrollRunCancelled`.
- `book_payroll_run(pool, company_id, actor, payroll_run_id, today) -> Result<VoucherRef>`:
  1. Kör `domain::book_payroll_run` i transaktionen, som ger `RecordVoucher` eller `PayrollRunNotDue` med flera.
  2. Anropar `doris_ledger::record_voucher_in(&mut tx, company_id, actor, record_voucher, today)`.
  3. Lägger till `PayrollRunBooked` med `VoucherRef`.
  4. Projicerar och gör commit.

  Om något steg misslyckas rullas allt tillbaka, och då används inget verifikationsnummer. Körningen förblir färdigställd och kan bokföras när felet har åtgärdats, till exempel när ett inaktivt konto har aktiverats. Ledgerns fel (`FiscalYearClosed`, `AccountInactive`, …) skickas vidare som `Error::Ledger(doris_ledger::Error)`.
- `rebuild_projections` tömmer och bygger om `employees`, `payroll_runs` och `payroll_run_lines` från `read_all`. Ledgerns projektioner måste byggas först, på grund av främmande nyckeln mot `vouchers`.

### Läsningar (`queries.rs`)
- `list_employees(conn, company_id, actor)` ger alla anställda, även inaktiva, sorterade på namn.
- `list_payroll_runs(conn, company_id, actor)` ger alla körningar med rader och summor, nyast utbetalningsdag först. Status är `finalized`, `booked`, `reversed` eller `cancelled`. Bokförda körningar har också räkenskapsår och verifikationsnummer.
  - `// ponytail:` Listan filtreras inte per räkenskapsår, eftersom en färdigställd körning ännu inte har något år. Paginering behövs först när ett företag har flera år av körningar.

## API (`proto/doris/payroll/v1/payroll.proto`, `PayrollService`)
```proto
message Employee {
  string id = 1;
  string name = 2;
  string personal_identity_number = 3; // ÅÅÅÅMMDD-NNNN
  int64 monthly_salary = 4;            // öre
  uint32 salary_account = 5;
  bool active = 6;
}

rpc ListEmployees(ListEmployeesRequest) returns (ListEmployeesResponse);
rpc AddEmployee(AddEmployeeRequest) returns (AddEmployeeResponse);         // returns the new id
rpc UpdateEmployee(UpdateEmployeeRequest) returns (UpdateEmployeeResponse); // name, salary, account
rpc DeactivateEmployee(DeactivateEmployeeRequest) returns (DeactivateEmployeeResponse);

message PayrollRunLineInput {
  string employee_id = 1;
  int64 gross = 2;
  int64 tax = 3;
}
message PayrollRunInput {
  string company_id = 1;
  string pay_date = 2;
  string text = 3;
  repeated PayrollRunLineInput lines = 4;
}
message PayrollRunLine {
  string employee_id = 1;
  string employee_name = 2;
  uint32 salary_account = 3;
  int64 gross = 4;
  int64 tax = 5;
  uint32 fee_rate = 6; // basis points
  int64 fee = 7;
  int64 net = 8;
}

rpc PreviewPayrollRun(PayrollRunInput) returns (PreviewPayrollRunResponse);
message PreviewPayrollRunResponse {
  string text = 1;
  repeated PayrollRunLine lines = 2;
  repeated doris.ledger.v1.VoucherLine voucher_lines = 3;
}

rpc FinalizePayrollRun(PayrollRunInput) returns (FinalizePayrollRunResponse);
message FinalizePayrollRunResponse { string payroll_run_id = 1; }

rpc BookPayrollRun(PayrollRunRef) returns (BookPayrollRunResponse);
message BookPayrollRunResponse {
  string fiscal_year_start = 1;
  uint32 voucher_number = 2;
}

rpc CancelPayrollRun(PayrollRunRef) returns (CancelPayrollRunResponse);
message CancelPayrollRunResponse {}

message PayrollRunRef {
  string company_id = 1;
  string payroll_run_id = 2;
}

rpc ListPayrollRuns(ListPayrollRunsRequest) returns (ListPayrollRunsResponse);
message ListPayrollRunsRequest { string company_id = 1; }
enum PayrollRunStatus {
  PAYROLL_RUN_STATUS_UNSPECIFIED = 0;
  PAYROLL_RUN_STATUS_FINALIZED = 1;
  PAYROLL_RUN_STATUS_BOOKED = 2;
  PAYROLL_RUN_STATUS_REVERSED = 3;
  PAYROLL_RUN_STATUS_CANCELLED = 4;
}
message PayrollRun {
  string id = 1;
  string pay_date = 2;
  string text = 3;
  repeated PayrollRunLine lines = 4;
  PayrollRunStatus status = 5;
  repeated doris.ledger.v1.VoucherLine voucher_lines = 6;
  string fiscal_year_start = 7; // set once booked
  uint32 voucher_number = 8;    // set once booked
}
message ListPayrollRunsResponse { repeated PayrollRun payroll_runs = 1; }
```
`doris-proto` kompilerar den nya filen i `build.rs`. Varje request har ett `company_id`.

### Server (`crates/server`)
- `payroll.rs` innehåller `PayrollService` med `status` och `domain_status` på samma sätt som `ledger.rs`. Ledgerns fel mappas med ledgerns befintliga mappning, så koderna (`fiscal_year_closed`, `account_inactive`, …) blir desamma som i grundboken.
- Tjänsten registreras bredvid de andra. Tonics standardgräns (4 MiB) gäller, och `session_gate` behöver inte ändras. Handlarna kontrollerar sessionen själva, som `CompanyService`.
- Nya koder:
  - `invalid_personal_identity_number`, `invalid_employee_name`, `invalid_salary`, `invalid_salary_account`, `invalid_tax`, `empty_payroll_run` och `duplicate_payroll_run_line` mappas till `InvalidArgument`.
  - `duplicate_employee`, `employee_inactive`, `payroll_run_not_due`, `payroll_run_booked` och `payroll_run_cancelled` mappas till `FailedPrecondition`.
  - `employee_not_found` och `payroll_run_not_found` mappas till `NotFound`.
- Personnummer och namn loggas aldrig.

## Frontend
En ny grupp i menyn, "Lön", med två sidor. Båda arbetar mot det aktiva företaget.

### `/employees`, Anställda
- Tabellen har kolumnerna Namn, Personnummer, Månadslön och Konto. Inaktiva anställda visas bara när kryssrutan "Visa inaktiva" är ikryssad.
- Formuläret "Ny anställd" har fälten Namn, Personnummer, Månadslön (kr) och Lönekonto. Lönekontot väljs i en lista med 7210 Löner till tjänstemän (förvalt), 7010 Löner till kollektivanställda och 7220 Löner till företagsledare.
- "Redigera" på en rad öppnar samma formulär utan fältet Personnummer. "Inaktivera" kräver en bekräftelse.

### `/payroll-runs/new`, Ny lönekörning
- Fälten Utbetalningsdag och Text. Utbetalningsdagen får ligga i framtiden. Om texten lämnas tom blir den "Lön {månad år}".
- En rad per aktiv anställd med kryssruta (ikryssad), namn, Brutto (förifyllt från månadslönen) och Skatt (tomt, och obligatoriskt för ikryssade rader).
- "Förhandsgranska" anropar `PreviewPayrollRun` och visar avgiftssats, avgift och netto per rad, summor och verifikationens rader.
- "Färdigställ" anropar `FinalizePayrollRun` och går till `/payroll-runs`. Knappen är spärrad tills förhandsgranskningen stämmer med det som står i formuläret.

### `/payroll-runs`, Lönekörningar
- En lista med Utbetalningsdag, Text, Brutto, Skatt, Avgift, Netto, Status och Ver.
- Status visas som märken:
  - "Färdigställd": utbetalningsdagen ligger i framtiden.
  - "Att bokföra": utbetalningsdagen har kommit och körningen är inte bokförd.
  - "Bokförd": med länk till verifikationen.
  - "Makulerad": körningen har makulerats eller rättats.
- En färdigställd körning har knapparna "Bokför" och "Makulera". "Bokför" är bara aktiv från utbetalningsdagen och har annars förklaringen "Kan bokföras från {datum}". Servern kontrollerar datumet ändå. "Makulera" kräver en bekräftelse.
- En rad kan expanderas och visar då raderna per anställd och verifikationens rader.

### Övrigt
`src/errors.rs` får en svensk text för varje ny kod:
- `invalid_personal_identity_number`: "Personnumret är ogiltigt (ÅÅÅÅMMDD-NNNN)."
- `invalid_employee_name`: "Ange ett namn (högst 100 tecken)."
- `invalid_salary`: "Lönen måste vara större än noll."
- `invalid_salary_account`: "Välj ett lönekonto."
- `invalid_tax`: "Skatten får inte vara negativ eller större än bruttolönen."
- `empty_payroll_run`: "Välj minst en anställd."
- `duplicate_payroll_run_line`: "Samma anställd finns två gånger i körningen."
- `duplicate_employee`: "Det finns redan en anställd med det personnumret."
- `employee_inactive`: "Den anställda är inaktiverad."
- `employee_not_found`: "Den anställda hittades inte."
- `payroll_run_not_found`: "Lönekörningen hittades inte."
- `payroll_run_not_due`: "Lönekörningen kan inte bokföras före utbetalningsdagen."
- `payroll_run_booked`: "Lönekörningen är redan bokförd. Rätta verifikationen i grundboken."
- `payroll_run_cancelled`: "Lönekörningen är makulerad."

## Tester
Varje beteende utvecklas med TDD: rött, grönt, refaktorering och commit.

### Domän (`crates/payroll/tests/domain.rs`)
- `PersonalIdentityNumber`:
  - giltigt med och utan bindestreck,
  - fel kontrollsiffra,
  - tio siffror,
  - ogiltigt datum,
  - samordningsnummer (dag 61–91) giltigt och dag 92 ogiltig.
- `employer_fee` med utbetalning 2026:
  - född 1937 ger 0 %, 1938 ger 10,21 %, 1958 ger 10,21 % och 1959 ger 31,42 %,
  - född 2003 och 2007 ger 20,81 %, medan 2002 och 2008 ger 31,42 %,
  - utbetalning 2026-03-31 ger 31,42 % och 2026-04-01 ger 20,81 %,
  - utbetalning 2027-09-30 ger 20,81 % och 2027-10-01 ger 31,42 %,
  - taket: 30 000 kr ger 25 000 × 20,81 % + 5 000 × 31,42 %,
  - två körningar i samma månad på 20 000 kr var: den andra får 5 000 kr till 20,81 % och 15 000 kr till 31,42 %. Om den första makuleras får den andra hela beloppet till 20,81 %,
  - avrundning av halva öre.
- `compute_payroll_run`:
  - utbetalningsdag i framtiden går bra,
  - standardtexten,
  - `salary_account` kopieras,
  - varje felkod.
- `voucher_lines`: raderna balanserar, grupperas per lönekonto och utelämnar rader med 0.
- `book_payroll_run`:
  - utbetalningsdag efter `today` ger `PayrollRunNotDue`, samma dag går bra,
  - redan bokförd ger `PayrollRunBooked` och makulerad ger `PayrollRunCancelled`,
  - lönekontot från när körningen färdigställdes används, även om den anställda har bytt konto sedan dess,
  - en anställd som inaktiverats efter färdigställandet hindrar inte bokföringen.
- `cancel_payroll_run`: en bokförd körning ger `PayrollRunBooked`, och en körning som makuleras två gånger ger inga event andra gången.
- Given/when/then för anställda: dubblett, även hos en inaktiv, uppdatering av en inaktiv, och inaktivering två gånger utan event.

### Lagring (`crates/payroll/tests/store.rs`)
- `finalize_payroll_run` med utbetalning i framtiden ger `PayrollRunFinalized` och ingen verifikation.
- `book_payroll_run` på utbetalningsdagen ger `VoucherRecorded` i `ledger-…` och `PayrollRunBooked` i `payroll-…`, och verifikationen har rätt rader och datum.
- `book_payroll_run` före utbetalningsdagen ger `PayrollRunNotDue`, och då tillkommer inga event och inget nummer.
- Ett stängt år eller ett inaktiverat lönekonto ger ledgerns fel, och ingenting skrivs. Körningen förblir färdigställd och kan bokföras när kontot har aktiverats igen.
- Ett dubblerat personnummer ger `DuplicateEmployee`.
- Om verifikationen rättas med `correct_voucher` får körningen statusen `reversed`, och dess bruttolön räknas inte längre in i avgiftstaket.
- `rebuild_projections` från `read_all` ger samma tre tabeller.
- Den som inte är medlem får `Error::NotFound`.
- `crates/ledger/tests/stress.rs` går fortfarande igenom.

### Server (gRPC-Web-integrationstester, `crates/server/tests/payroll.rs`)
- Flödet `AddEmployee` → `PreviewPayrollRun` → `FinalizePayrollRun` → `BookPayrollRun` → `ListVouchers` visar verifikationen, och `ListPayrollRuns` visar körningen som bokförd.
- `BookPayrollRun` med utbetalning i morgon ger `payroll_run_not_due`.
- Ett ogiltigt personnummer ger `invalid_personal_identity_number`.
- Ett annat företags `company_id` ger `NotFound`.

### E2E (`e2e/`)
- Lägg till en anställd och kör lön med dagens datum: förhandsgranska, färdigställ och bokför. Verifikationen syns i grundboken med 7210, 2710, 1930, 7510 och 2731.
- Färdigställ en körning med utbetalning i morgon. Den visas som "Färdigställd" med "Bokför" inaktiv, och "Makulera" gör den "Makulerad".
- Rätta en bokförd verifikation i grundboken. Körningen visas som "Makulerad".
- Ett ogiltigt personnummer ger "Personnumret är ogiltigt (ÅÅÅÅMMDD-NNNN)."

### Avslutning
- `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
- `make dist` håller sig inom `WASM_BUDGET`.
- AGENTS.md uppdateras med:
  - `crates/payroll` i Layout och `payroll.proto` under API,
  - de nya koderna och avgiftsreglerna,
  - att körningar färdigställs och sedan bokförs, tidigast på utbetalningsdagen,
  - att en bokförd körning makuleras genom att dess verifikation rättas.

## Utanför omfattningen
- Skattetabeller (nästa steg)
- AGI
- Lönebesked
- Semester och semesterskuld
- Förmåner
- Frånvaro
- Timlön
- Växa-stöd
- Pension
- En särskild löneroll
- Automatisk bokföring på utbetalningsdagen
- Att ändra en färdigställd körning (den makuleras och görs om)
