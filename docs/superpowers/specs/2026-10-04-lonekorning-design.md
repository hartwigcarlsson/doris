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
| Bokföring | En verifikation per körning, daterad på utbetalningsdagen, mot 1930. Ledgerns regler gäller oförändrade: inget framtida datum, inga stängda år, inga inaktiva konton. |
| Flera körningar | Tillåtna i samma månad, till exempel för en extra utbetalning. |
| Rättelse | Ingen egen funktion. Verifikationen rättas i grundboken som vanligt, och då räknas körningen som makulerad. |
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
    /// A payroll run and the voucher it was booked as.
    PayrollRunApproved {
        payroll_run_id: Uuid,
        pay_date: Date,
        text: String,
        lines: Vec<PayrollRunLine>,
        fiscal_year_start: Date,
        voucher_number: u32,
    },
}

pub struct PayrollRunLine {
    pub employee_id: Uuid,
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
`Payroll { employees: Vec<Employee>, runs: Vec<PayrollRun> }`, där `Employee` har `active: bool`. `Payroll::apply(event)` är `evolve`. Ström-ID:t är per företag, så både anställda och körningar finns i samma tillstånd. Det gör det enkelt att kontrollera dubbletter och avgiftstaket.

### Beslut
- `add_employee(payroll, cmd)`: om personnumret redan finns i företaget, även hos en inaktiv anställd, blir det `DuplicateEmployee`. UNIQUE på projektionen säkrar samma sak.
- `update_employee(payroll, cmd)`: en anställd som saknas ger `EmployeeNotFound` och en inaktiv ger `EmployeeInactive`. Om inget ändras returneras inga event.
- `deactivate_employee(payroll, id)`: en anställd som saknas ger `EmployeeNotFound`. Är den redan inaktiv returneras inga event.
- `compute_payroll_run(payroll, reversed, cmd) -> Result<ComputedPayrollRun>`, där `reversed: &HashSet<Uuid>` är de makulerade körningarna och `cmd = { pay_date, text, lines: [{ employee_id, gross, tax }] }`:
  - Inga rader ger `EmptyPayrollRun`, och samma anställd två gånger ger `DuplicatePayrollRunLine`.
  - En anställd som saknas ger `EmployeeNotFound` och en inaktiv ger `EmployeeInactive`.
  - `gross` ≤ 0 ger `InvalidSalary`. `tax` < 0 eller `tax` > `gross` ger `InvalidTax`.
  - En tom text efter trimning ersätts med "Lön {månad år}", till exempel "Lön oktober 2026".
  - Varje rad räknas med `employer_fee`, och `net = gross - tax`.
  - Resultatet är raderna plus verifikationens rader (`Vec<doris_ledger::domain::VoucherLine>`), grupperade per konto:
    - debet på varje lönekonto med summan av bruttolönerna,
    - kredit på 2710 med summan av skatterna,
    - kredit på 1930 med summan av nettolönerna,
    - debet på 7510 och kredit på 2731 med summan av avgifterna.

    Rader med beloppet 0 tas inte med.
  - Det här används både av förhandsgranskningen och av godkännandet.
- `approve_payroll_run(computed, payroll_run_id, voucher)` ger `PayrollRunApproved`.

### Arbetsgivaravgift
`employer_fee(birth_year, pay_date, gross, earlier_gross_same_month) -> (fee_rate, fee)`. Året `Y` är utbetalningsdagens år.

| Villkor | Sats |
|---|---|
| `birth_year` ≤ 1937 | 0 % |
| `birth_year` ≤ `Y` − 68, alltså fyllt 67 vid årets ingång | 10,21 % (endast ålderspensionsavgift) |
| `Y` − 23 ≤ `birth_year` ≤ `Y` − 19, och `pay_date` från 2026-04-01 till 2027-09-30 | 20,81 % på belopp upp till 25 000 kr per kalendermånad, 31,42 % på resten |
| Övriga | 31,42 % |

- Satserna anges i baspunkter (3142, 1021, 2081). `fee = round_half_up((under_cap * rate + over_cap * 3142) / 10000)` i öre, en gång per rad.
- Taket på 25 000 kr per kalendermånad delas mellan alla körningar med utbetalningsdag i samma månad. `earlier_gross_same_month` är summan av bruttolönen för den anställda i tidigare godkända körningar i den månaden som inte har makulerats. En makulering syns inte i lönernas egen ström, så lib-lagret läser de makulerade körningarna (join mot `vouchers.corrects`) och skickar dem som `reversed` till `compute_payroll_run`.
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
    fiscal_year_start TEXT    NOT NULL,
    voucher_number    INTEGER NOT NULL,
    approved_at       TEXT    NOT NULL,
    approved_by       TEXT    NOT NULL,
    PRIMARY KEY (company_id, payroll_run_id),
    FOREIGN KEY (company_id, fiscal_year_start, voucher_number)
        REFERENCES vouchers (company_id, fiscal_year_start, number)
);

CREATE TABLE payroll_run_lines (
    company_id     TEXT    NOT NULL,
    payroll_run_id TEXT    NOT NULL,
    employee_id    TEXT    NOT NULL,
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
Flaggan `reversed` lagras inte. Den räknas ut när den läses: en körning är makulerad om det finns en verifikation vars `corrects` pekar på körningens verifikation.

### Skrivflöde (`crates/payroll/src/lib.rs`)
Varje skrivning sker i en `BEGIN IMMEDIATE` (`doris_eventstore::begin`) och följer samma ordning som ledgern: medlemskap (`doris_company::get_company_in`), ladda strömmen, besluta, `append` och projicera.
- `add_employee`, `update_employee` och `deactivate_employee` är enkla kommandon.
- `preview_payroll_run(pool, company_id, actor, cmd)` laddar tillståndet, hämtar de makulerade körningarna och kör `compute_payroll_run`. Den skriver ingenting.
- `approve_payroll_run(pool, company_id, actor, cmd, today) -> Result<VoucherRef>`:
  1. Kör `compute_payroll_run` i transaktionen.
  2. Anropar `doris_ledger::record_voucher_in(&mut tx, …, RecordVoucher { date: pay_date, text, lines }, today)`.
  3. Lägger till `PayrollRunApproved` med `VoucherRef`.
  4. Projicerar och gör commit.

  Om något steg misslyckas rullas allt tillbaka, och då används inget verifikationsnummer. Ledgerns fel (`VoucherDateInFuture`, `FiscalYearClosed`, `AccountInactive`, …) skickas vidare som `Error::Ledger(doris_ledger::Error)`.
- `rebuild_projections` tömmer och bygger om `employees`, `payroll_runs` och `payroll_run_lines` från `read_all`. Ledgerns projektioner måste byggas först, på grund av främmande nyckeln mot `vouchers`.

### Läsningar (`queries.rs`)
- `list_employees(conn, company_id, actor)` ger alla anställda, även inaktiva, sorterade på namn.
- `list_payroll_runs(conn, company_id, actor, fiscal_year_start)` ger körningarna med rader, summor, `voucher_number` och `reversed`, nyast först.

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
  int64 gross = 3;
  int64 tax = 4;
  uint32 fee_rate = 5; // basis points
  int64 fee = 6;
  int64 net = 7;
}

rpc PreviewPayrollRun(PayrollRunInput) returns (PreviewPayrollRunResponse);
message PreviewPayrollRunResponse {
  string text = 1;
  repeated PayrollRunLine lines = 2;
  repeated doris.ledger.v1.VoucherLine voucher_lines = 3;
}

rpc ApprovePayrollRun(PayrollRunInput) returns (ApprovePayrollRunResponse);
message ApprovePayrollRunResponse {
  string fiscal_year_start = 1;
  uint32 voucher_number = 2;
}

rpc ListPayrollRuns(ListPayrollRunsRequest) returns (ListPayrollRunsResponse);
message ListPayrollRunsRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
}
message PayrollRun {
  string id = 1;
  string pay_date = 2;
  string text = 3;
  repeated PayrollRunLine lines = 4;
  string fiscal_year_start = 5;
  uint32 voucher_number = 6;
  bool reversed = 7;
}
message ListPayrollRunsResponse { repeated PayrollRun payroll_runs = 1; }
```
`doris-proto` kompilerar den nya filen i `build.rs`. Varje request har ett `company_id`.

### Server (`crates/server`)
- `payroll.rs` innehåller `PayrollService` med `status` och `domain_status` på samma sätt som `ledger.rs`. Ledgerns fel mappas med ledgerns befintliga mappning, så koderna (`fiscal_year_closed`, `voucher_date_in_future`, `account_inactive`, …) blir desamma som i grundboken.
- Tjänsten registreras bredvid de andra. Tonics standardgräns (4 MiB) gäller, och `session_gate` behöver inte ändras. Handlarna kontrollerar sessionen själva, som `CompanyService`.
- Nya koder:
  - `invalid_personal_identity_number`, `invalid_employee_name`, `invalid_salary`, `invalid_salary_account`, `invalid_tax`, `empty_payroll_run` och `duplicate_payroll_run_line` mappas till `InvalidArgument`.
  - `duplicate_employee` och `employee_inactive` mappas till `FailedPrecondition`.
  - `employee_not_found` mappas till `NotFound`.
- Personnummer och namn loggas aldrig.

## Frontend
En ny grupp i menyn, "Lön", med två sidor. Båda arbetar mot det aktiva företaget.

### `/employees`, Anställda
- Tabellen har kolumnerna Namn, Personnummer, Månadslön och Konto. Inaktiva anställda visas bara när kryssrutan "Visa inaktiva" är ikryssad.
- Formuläret "Ny anställd" har fälten Namn, Personnummer, Månadslön (kr) och Lönekonto. Lönekontot väljs i en lista med 7210 Löner till tjänstemän (förvalt), 7010 Löner till kollektivanställda och 7220 Löner till företagsledare.
- "Redigera" på en rad öppnar samma formulär utan fältet Personnummer. "Inaktivera" kräver en bekräftelse.

### `/payroll-runs/new`, Ny lönekörning
- Fälten Utbetalningsdag och Text. Om texten lämnas tom blir den "Lön {månad år}".
- En rad per aktiv anställd med kryssruta (ikryssad), namn, Brutto (förifyllt från månadslönen) och Skatt (tomt, och obligatoriskt för ikryssade rader).
- "Förhandsgranska" anropar `PreviewPayrollRun` och visar avgiftssats, avgift och netto per rad, summor och verifikationens rader.
- "Godkänn och bokför" anropar `ApprovePayrollRun` och går till `/vouchers`. Knappen är spärrad tills förhandsgranskningen stämmer med det som står i formuläret.

### `/payroll-runs`, Lönekörningar
- En lista per räkenskapsår med Utbetalningsdag, Text, Brutto, Skatt, Avgift, Netto och Ver. En makulerad körning får märket "Makulerad".
- En rad kan expanderas och visar då raderna per anställd.

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
  - två körningar i samma månad på 20 000 kr var: den andra får 5 000 kr till 20,81 % och 15 000 kr till 31,42 %,
  - avrundning av halva öre.
- `compute_payroll_run`:
  - verifikationens rader balanserar,
  - grupperas per lönekonto,
  - utelämnar rader med 0,
  - standardtexten,
  - varje felkod.
- Given/when/then för anställda: dubblett, även hos en inaktiv, uppdatering av en inaktiv, och inaktivering två gånger utan event.

### Lagring (`crates/payroll/tests/store.rs`)
- `approve_payroll_run` ger `VoucherRecorded` i `ledger-…` och `PayrollRunApproved` i `payroll-…`, och verifikationen har rätt rader.
- Ett stängt år ger `fiscal_year_closed` och ett framtida datum `voucher_date_in_future`. I båda fallen tillkommer inga event och inget nummer.
- Ett inaktiverat lönekonto i kontoplanen ger ledgerns fel, och ingenting skrivs.
- Ett dubblerat personnummer ger `DuplicateEmployee`.
- Om verifikationen rättas med `correct_voucher` blir körningen `reversed`, och dess bruttolön räknas inte längre in i avgiftstaket.
- `rebuild_projections` från `read_all` ger samma tre tabeller.
- Den som inte är medlem får `Error::NotFound`.
- `crates/ledger/tests/stress.rs` går fortfarande igenom.

### Server (gRPC-Web-integrationstester, `crates/server/tests/payroll.rs`)
- Flödet `AddEmployee` → `PreviewPayrollRun` → `ApprovePayrollRun` → `ListVouchers` visar verifikationen och `ListPayrollRuns` visar körningen.
- Ett ogiltigt personnummer ger `invalid_personal_identity_number`, och en körning i ett stängt år ger `fiscal_year_closed`.
- Ett annat företags `company_id` ger `NotFound`.

### E2E (`e2e/`)
- Lägg till en anställd, kör lön, förhandsgranska och godkänn. Verifikationen syns i grundboken med 7210, 2710, 1930, 7510 och 2731.
- Rätta verifikationen i grundboken. Körningen visas som "Makulerad" under Lönekörningar.
- Ett ogiltigt personnummer ger "Personnumret är ogiltigt (ÅÅÅÅMMDD-NNNN)."

### Avslutning
- `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
- `make dist` håller sig inom `WASM_BUDGET`.
- AGENTS.md uppdateras med `crates/payroll` i Layout, `payroll.proto` under API, de nya koderna, avgiftsreglerna och regeln att en körning makuleras genom att dess verifikation rättas.

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
- Att förbereda en körning före utbetalningsdagen
