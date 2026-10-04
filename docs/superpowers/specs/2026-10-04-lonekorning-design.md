# Doris – Steg 10: Anställda och lönekörning

## Kontext
Doris har i dag kontoplan, verifikationer, huvudbok, bokslut och underlag, men inget stöd för löner. Det här steget inför lönehantering i sin minsta form: ett register över anställda och en månatlig lönekörning som räknar fram arbetsgivaravgift och nettolön och bokförs som en verifikation. Preliminärskatten skrivs in för hand av användaren, som hämtar beloppet från Skatteverkets skattetabell.

Lönedatan sparas per anställd och körning, eftersom de följande stegen bygger på den:
1. **Nästa steg:** skattetabeller. Varje anställd får tabell och kolumn, och skatten räknas fram från Skatteverkets tabell i stället för att skrivas in.
2. Därefter AGI, alltså arbetsgivardeklaration på individnivå, samt lönebesked, semester och förmåner.

### Fattade beslut
| Område | Beslut |
|---|---|
| Placering | Ny crate `doris-payroll` med egen event-ström. Den anropar `doris_ledger::record_voucher_in` och `correct_voucher_in` i samma transaktion, så ledgern vet inget om löner. |
| Åtkomst | Alla medlemmar i företaget, som för bokföringen. En särskild löneroll ligger utanför omfattningen. |
| Lönetyp | Månadslön. Bruttolönen förifylls från registret och kan ändras i körningen. |
| Skatt | Skrivs in i kronor och öre per anställd och körning. Den får inte vara negativ eller större än bruttolönen. |
| Arbetsgivaravgift | Räknas fram i koden från födelseåret och utbetalningsdagen (se nedan). |
| Livscykel | En körning har tre statusar: **Öppen**, **Färdigställd** och **Bokförd**. Bara en öppen körning kan ändras. Att färdigställa låser belopp, avgifter och lönekonton och går att göra när som helst, även före utbetalningsdagen. En färdigställd körning som inte är bokförd kan öppnas igen. Den bokförs med knappen "Bokför", tidigast på utbetalningsdagen. En bokförd körning kan inte öppnas förrän bokföringen har backats. |
| Bokföring | En verifikation per bokföring, daterad på utbetalningsdagen, mot 1930. Den bygger på de låsta beloppen och kontona. Ledgerns regler gäller oförändrade: inga stängda år och inga inaktiva konton. Bokföringen görs av en person och sker aldrig automatiskt. |
| Backa bokföring | Bokföringsverifikationen rättas med en rättelse, antingen med "Backa bokföring" på körningen eller med Rätta i grundboken. Då blir körningen Färdigställd igen. Statusen räknas fram från `vouchers.corrects`, så de två vägarna kan inte ge olika svar. |
| Flera körningar | Tillåtna i samma månad, till exempel för en extra utbetalning. |
| Radera | En körning tas aldrig bort. En öppen körning som inte behövs får ligga kvar, eftersom den inte påverkar bokföringen. |
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

    /// A new, open run.
    PayrollRunCreated { payroll_run_id: Uuid, draft: PayrollRunDraft },
    /// The open run's contents replaced.
    PayrollRunUpdated { payroll_run_id: Uuid, draft: PayrollRunDraft },
    /// Amounts, fees and accounts locked; may precede the pay date.
    PayrollRunFinalized { payroll_run_id: Uuid, lines: Vec<PayrollRunLine> },
    /// Open again; the locked lines are dropped.
    PayrollRunReopened { payroll_run_id: Uuid },
    /// Booked as a voucher, on or after the pay date.
    PayrollRunBooked { payroll_run_id: Uuid, voucher: VoucherRef },
}

pub struct PayrollRunDraft {
    pub pay_date: Date,
    pub text: String,
    pub lines: Vec<DraftLine>, // { employee_id, gross, tax }
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
- Vem som gjorde vad och när finns i eventets metadata, som för andra event.
- Att bokföringen backas blir ingen lönehändelse. Det är rättelseverifikationen i ledgern, med vem och när i dess metadata.
- `fee_rate` sparas så att en körning alltid kan läsas som den räknades, även om reglerna ändras senare.

### Tillstånd
`Payroll::from_events(events, reversed: &HashSet<VoucherRef>)` ger `Payroll { employees, runs }`. `reversed` är de verifikationer som har rättats och läses av lib-lagret från ledgerns projektion.

`PayrollRun` innehåller:
- `draft`,
- `lines: Option<Vec<PayrollRunLine>>`, som sätts av `Finalized` och tas bort av `Reopened`,
- `bookings: Vec<VoucherRef>`.

`status()` räknas fram:
- **Bokförd** om den senaste bokföringen inte finns i `reversed`.
- Annars **Färdigställd** om `lines` är satt.
- Annars **Öppen**.

En körning kan alltså ha flera bokföringar i sin historik, men högst en som gäller.

### Beslut
- **Anställda:**
  - `add_employee(payroll, cmd)`: om personnumret redan finns i företaget, även hos en inaktiv anställd, blir det `DuplicateEmployee`. UNIQUE på projektionen säkrar samma sak.
  - `update_employee(payroll, cmd)`: en anställd som saknas ger `EmployeeNotFound` och en inaktiv ger `EmployeeInactive`. Om inget ändras returneras inga event.
  - `deactivate_employee(payroll, id)`: en anställd som saknas ger `EmployeeNotFound`. Är den redan inaktiv returneras inga event.
- **`validate_draft(payroll, draft)`** används av både `create` och `update`:
  - Inga rader ger `EmptyPayrollRun`, och samma anställd två gånger ger `DuplicatePayrollRunLine`.
  - En anställd som saknas ger `EmployeeNotFound` och en inaktiv ger `EmployeeInactive`.
  - `gross` ≤ 0 ger `InvalidSalary`. `tax` < 0 eller `tax` > `gross` ger `InvalidTax`.
  - En tom text efter trimning ersätts med "Lön {månad år}", till exempel "Lön oktober 2026". En text på över 200 tecken ger `InvalidText`, som mappas till ledgerns befintliga kod `invalid_voucher_text`. Felet kommer alltså direkt och inte först vid bokföringen.
  - Utbetalningsdagen får ligga i framtiden.
- **`compute_lines(payroll, payroll_run_id, draft) -> Result<Vec<PayrollRunLine>>`:** kör `validate_draft`, räknar varje rad med `employer_fee` och sätter `net = gross - tax`. `salary_account` kopieras från den anställda. Avgiftstaket räknas mot andra körningar som är **bokförda** och har utbetalningsdag i samma kalendermånad, där körningen själv inte räknas med. Funktionen används av förhandsgranskningen, av färdigställandet och vid bokföringen.
- **`voucher_lines(lines) -> Vec<doris_ledger::domain::VoucherLine>`** grupperar per konto:
  - debet på varje `salary_account` med summan av bruttolönerna,
  - kredit på 2710 med summan av skatterna,
  - kredit på 1930 med summan av nettolönerna,
  - debet på 7510 och kredit på 2731 med summan av avgifterna.

  Rader med beloppet 0 tas inte med.
- **Övergångar.** En körning som saknas ger alltid `PayrollRunNotFound`.

| Kommando | Kräver status | Annars | Ger |
|---|---|---|---|
| `create_payroll_run(payroll, id, draft)` | – | | `PayrollRunCreated` |
| `update_payroll_run(payroll, id, draft)` | Öppen | Färdigställd ger `PayrollRunNotOpen`, Bokförd ger `PayrollRunBooked` | `PayrollRunUpdated` |
| `finalize_payroll_run(payroll, id)` | Öppen | som ovan | `PayrollRunFinalized { lines: compute_lines(..) }` |
| `reopen_payroll_run(payroll, id)` | Färdigställd | Öppen ger `PayrollRunNotFinalized`, Bokförd ger `PayrollRunBooked` | `PayrollRunReopened` |
| `book_payroll_run(payroll, id, today)` | Färdigställd | som ovan | `RecordVoucher` för lib-lagret |
| `unbook_payroll_run(payroll, id)` | Bokförd | Öppen och Färdigställd ger `PayrollRunNotBooked` | `VoucherRef` att rätta |

- **Ytterligare villkor för `book_payroll_run`:**
  - `pay_date` > `today` ger `PayrollRunNotDue`.
  - `compute_lines` körs igen med det aktuella tillståndet. Om avgifterna skiljer sig från de låsta raderna blir det `PayrollRunOutdated`, och användaren öppnar och färdigställer körningen igen. Det händer bara när en annan körning i samma månad har bokförts eller backats efter färdigställandet och ungdomstaket påverkas. Kontrollen jämför bara avgifterna. Att en anställd har inaktiverats eller bytt konto efter färdigställandet hindrar inte bokföringen, eftersom de låsta raderna används.
  - Om allt stämmer returneras `RecordVoucher { date: pay_date, text, lines: voucher_lines(lines) }`. Lib-lagret lägger sedan till `PayrollRunBooked`.

### Arbetsgivaravgift
`employer_fee(birth_year, pay_date, gross, earlier_gross_same_month) -> (fee_rate, fee)`. Året `Y` är utbetalningsdagens år.

| Villkor | Sats |
|---|---|
| `birth_year` ≤ 1937 | 0 % |
| `birth_year` ≤ `Y` − 68, alltså fyllt 67 vid årets ingång | 10,21 % (endast ålderspensionsavgift) |
| `Y` − 23 ≤ `birth_year` ≤ `Y` − 19, och `pay_date` från 2026-04-01 till 2027-09-30 | 20,81 % på belopp upp till 25 000 kr per kalendermånad, 31,42 % på resten |
| Övriga | 31,42 % |

- Satserna anges i baspunkter (3142, 1021, 2081). `fee = round_half_up((under_cap * rate + over_cap * 3142) / 10000)` i öre, en gång per rad.
- `earlier_gross_same_month` är den anställdas bruttolön i andra körningar som gäller som bokförda och har utbetalningsdag i samma kalendermånad (se `compute_lines`).
- `// ponytail:` Taket räknas bara mot bokförda körningar, och bokföringen stoppas med `PayrollRunOutdated` om avgiften har ändrats. Det räcker eftersom flera körningar i samma månad för en anställd under 24 år är ovanligt.
- `// ponytail:` Reglerna ligger i koden, giltiga från 2026. En ändrad sats kräver en ny version av Doris. Satser per datum i en tabell blir aktuellt först när de ändras oftare än Doris släpps.
- AGI räknar avgiften på summan per sats och avrundar nedåt till hela kronor. Verifikationen avrundar per rad till hela öre. Skillnaden hanteras i AGI-steget.

Källa: Skatteverket, "Arbetsgivaravgifter" (2026).

## Lagring och transaktioner

### Migration `migrations/0010_payroll.sql`
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
    company_id     TEXT    NOT NULL,
    payroll_run_id TEXT    NOT NULL,
    pay_date       TEXT    NOT NULL,
    text           TEXT    NOT NULL,
    finalized      INTEGER NOT NULL, -- 1 between Finalized and Reopened
    updated_at     TEXT    NOT NULL,
    updated_by     TEXT    NOT NULL,
    PRIMARY KEY (company_id, payroll_run_id)
);

-- The draft's lines while open; the locked lines (with fee and account)
-- once finalized.
CREATE TABLE payroll_run_lines (
    company_id     TEXT    NOT NULL,
    payroll_run_id TEXT    NOT NULL,
    employee_id    TEXT    NOT NULL,
    gross          INTEGER NOT NULL,
    tax            INTEGER NOT NULL,
    salary_account INTEGER, -- NULL while open
    fee_rate       INTEGER,
    fee            INTEGER,
    net            INTEGER,
    PRIMARY KEY (company_id, payroll_run_id, employee_id),
    FOREIGN KEY (company_id, payroll_run_id)
        REFERENCES payroll_runs (company_id, payroll_run_id)
);

-- Every booking ever made; the latest unreversed one is in force.
CREATE TABLE payroll_run_bookings (
    company_id        TEXT    NOT NULL,
    payroll_run_id    TEXT    NOT NULL,
    fiscal_year_start TEXT    NOT NULL,
    voucher_number    INTEGER NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start, voucher_number),
    FOREIGN KEY (company_id, payroll_run_id)
        REFERENCES payroll_runs (company_id, payroll_run_id)
);
```
Tabellerna har ingen främmande nyckel mot `vouchers`, på samma sätt som ledgerns tabeller saknar en mot `companies`. Varje crate bygger om sina egna tabeller, och `doris_ledger::rebuild_projections` tömmer `vouchers`.
Status lagras inte. Den räknas ut när den läses, med samma regel som `status()`. Den senaste bokföringen är den med högst `rowid`, och den gäller om ingen verifikation har `corrects` som pekar på den.

### Skrivflöde (`crates/payroll/src/lib.rs`)
Varje skrivning sker i en `BEGIN IMMEDIATE` (`doris_eventstore::begin`):
1. medlemskap (`doris_company::get_company_in`),
2. de rättade verifikationerna (`reversed`) från ledgerns projektion,
3. ladda strömmen,
4. besluta,
5. `append` och projicera.

Funktionerna:
- `add_employee`, `update_employee` och `deactivate_employee`.
- `preview_payroll_run(pool, company_id, actor, draft)` kör `compute_lines` för en körning som inte finns, och skriver ingenting.
- `create_payroll_run`, `update_payroll_run`, `finalize_payroll_run` och `reopen_payroll_run` är rena lönekommandon.
- `book_payroll_run(pool, company_id, actor, id, today) -> Result<VoucherRef>` kör `domain::book_payroll_run`, sedan `doris_ledger::record_voucher_in(&mut tx, …, record_voucher, today)`, och lägger till `PayrollRunBooked`.
- `unbook_payroll_run(pool, company_id, actor, id, today) -> Result<VoucherRef>` kör `domain::unbook_payroll_run` och sedan `doris_ledger::correct_voucher_in(&mut tx, …, voucher.fiscal_year_start, voucher.number, date, today)`. Rättelsen dateras `today`, men aldrig senare än slutet av verifikationens räkenskapsår, eftersom ledgern kräver att en rättelse ligger i samma år. Ingen lönehändelse läggs till.

Om något steg misslyckas rullas allt tillbaka, och då används inget verifikationsnummer. Om bokföringen misslyckas förblir körningen Färdigställd och kan bokföras när felet är åtgärdat, till exempel när ett inaktivt konto har aktiverats. Ledgerns fel (`FiscalYearClosed`, `AccountInactive`, …) skickas vidare som `Error::Ledger(doris_ledger::Error)`. Att backa en bokföring i ett stängt år ger alltså `fiscal_year_closed`.

`rebuild_projections` tömmer och bygger om de fyra tabellerna från `read_all`. Ledgerns projektioner måste byggas först, på grund av främmande nyckeln mot `vouchers`.

### Läsningar (`queries.rs`)
- `list_employees(conn, company_id, actor)` ger alla anställda, även inaktiva, sorterade på namn.
- `list_payroll_runs(conn, company_id, actor)` ger alla körningar med rader, summor och status, nyast utbetalningsdag först. En bokförd körning har också sin gällande verifikation.
  - `// ponytail:` Listan filtreras inte per räkenskapsår, eftersom en öppen körning ännu inte har något år. Paginering behövs först när ett företag har flera år av körningar.
- `get_payroll_run(conn, company_id, actor, id)` ger en körning. Om den saknas blir det `PayrollRunNotFound`.

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
message PayrollRunDraft {
  string pay_date = 1;
  string text = 2;
  repeated PayrollRunLineInput lines = 3;
}
message PayrollRunRef {
  string company_id = 1;
  string payroll_run_id = 2;
}

rpc PreviewPayrollRun(PreviewPayrollRunRequest) returns (PreviewPayrollRunResponse);
message PreviewPayrollRunRequest {
  string company_id = 1;
  PayrollRunDraft draft = 2;
}
message PreviewPayrollRunResponse {
  string text = 1;
  repeated PayrollRunLine lines = 2;
  repeated doris.ledger.v1.VoucherLine voucher_lines = 3;
}

rpc CreatePayrollRun(CreatePayrollRunRequest) returns (CreatePayrollRunResponse);
message CreatePayrollRunRequest {
  string company_id = 1;
  PayrollRunDraft draft = 2;
}
message CreatePayrollRunResponse { string payroll_run_id = 1; }

rpc UpdatePayrollRun(UpdatePayrollRunRequest) returns (UpdatePayrollRunResponse);
message UpdatePayrollRunRequest {
  PayrollRunRef run = 1;
  PayrollRunDraft draft = 2;
}
message UpdatePayrollRunResponse {}

rpc FinalizePayrollRun(PayrollRunRef) returns (FinalizePayrollRunResponse);
message FinalizePayrollRunResponse {}
rpc ReopenPayrollRun(PayrollRunRef) returns (ReopenPayrollRunResponse);
message ReopenPayrollRunResponse {}
rpc BookPayrollRun(PayrollRunRef) returns (VoucherRef);
rpc UnbookPayrollRun(PayrollRunRef) returns (VoucherRef); // the rättelse
message VoucherRef {
  string fiscal_year_start = 1;
  uint32 number = 2;
}

rpc GetPayrollRun(PayrollRunRef) returns (PayrollRun);
rpc ListPayrollRuns(ListPayrollRunsRequest) returns (ListPayrollRunsResponse);
message ListPayrollRunsRequest { string company_id = 1; }
message ListPayrollRunsResponse { repeated PayrollRun payroll_runs = 1; }

enum PayrollRunStatus {
  PAYROLL_RUN_STATUS_UNSPECIFIED = 0;
  PAYROLL_RUN_STATUS_OPEN = 1;
  PAYROLL_RUN_STATUS_FINALIZED = 2;
  PAYROLL_RUN_STATUS_BOOKED = 3;
}
message PayrollRunLine {
  string employee_id = 1;
  string employee_name = 2;
  int64 gross = 3;
  int64 tax = 4;
  // Set once finalized (and in a preview).
  uint32 salary_account = 5;
  uint32 fee_rate = 6; // basis points
  int64 fee = 7;
  int64 net = 8;
}
message PayrollRun {
  string id = 1;
  string pay_date = 2;
  string text = 3;
  PayrollRunStatus status = 4;
  repeated PayrollRunLine lines = 5;
  repeated doris.ledger.v1.VoucherLine voucher_lines = 6; // once finalized
  VoucherRef voucher = 7;                                  // while booked
}
```
`doris-proto` kompilerar den nya filen i `build.rs`. Varje request har ett `company_id`.

### Server (`crates/server`)
- `payroll.rs` innehåller `PayrollService` med `status` och `domain_status` på samma sätt som `ledger.rs`. Ledgerns fel mappas med ledgerns befintliga mappning, så koderna (`fiscal_year_closed`, `account_inactive`, …) blir desamma som i grundboken.
- Tjänsten registreras bredvid de andra. Tonics standardgräns (4 MiB) gäller, och `session_gate` behöver inte ändras. Handlarna kontrollerar sessionen själva, som `CompanyService`.
- Nya koder:
  - `invalid_personal_identity_number`, `invalid_employee_name`, `invalid_salary`, `invalid_salary_account`, `invalid_tax`, `empty_payroll_run` och `duplicate_payroll_run_line` mappas till `InvalidArgument`.
  - `duplicate_employee`, `employee_inactive`, `payroll_run_not_open`, `payroll_run_not_finalized`, `payroll_run_booked`, `payroll_run_not_booked`, `payroll_run_not_due` och `payroll_run_outdated` mappas till `FailedPrecondition`.
  - `employee_not_found` och `payroll_run_not_found` mappas till `NotFound`.
- Personnummer och namn loggas aldrig.

## Frontend
En ny grupp i menyn, "Lön", med sidorna Anställda och Lönekörningar. Båda arbetar mot det aktiva företaget.

### `/employees`, Anställda
- Tabellen har kolumnerna Namn, Personnummer, Månadslön och Konto. Inaktiva anställda visas bara när kryssrutan "Visa inaktiva" är ikryssad.
- Formuläret "Ny anställd" har fälten Namn, Personnummer, Månadslön (kr) och Lönekonto. Lönekontot väljs i en lista med 7210 Löner till tjänstemän (förvalt), 7010 Löner till kollektivanställda och 7220 Löner till företagsledare.
- "Redigera" på en rad öppnar samma formulär utan fältet Personnummer. "Inaktivera" kräver en bekräftelse.

### `/payroll-runs`, Lönekörningar
- En lista med Utbetalningsdag, Text, Brutto, Skatt, Avgift, Netto, Status och Ver.
- Status visas som märken:
  - "Öppen",
  - "Färdigställd": utbetalningsdagen ligger i framtiden,
  - "Att bokföra": färdigställd och utbetalningsdagen har kommit,
  - "Bokförd": med länk till verifikationen.
- Knappen "Ny lönekörning" leder till `/payroll-runs/new`, och ett klick på en rad leder till `/payroll-runs/:id`.

### `/payroll-runs/new` och `/payroll-runs/:id`, Lönekörning
Sidan ser olika ut beroende på status.

- **Ny eller Öppen:** ett formulär.
  - Fälten Utbetalningsdag (får ligga i framtiden) och Text (tom ger "Lön {månad år}").
  - En rad per aktiv anställd med kryssruta, namn, Brutto och Skatt. I en ny körning är alla rader ikryssade och Brutto förifyllt från månadslönen. En öppnad körning visar sina sparade rader.
  - Knapparna:
    - "Förhandsgranska" anropar `PreviewPayrollRun` och visar avgiftssats, avgift och netto per rad, summor och verifikationens rader.
    - "Spara" anropar `CreatePayrollRun` eller `UpdatePayrollRun`.
    - "Färdigställ" sparar och anropar sedan `FinalizePayrollRun`.
- **Färdigställd:** skrivskyddad, med rader, summor och verifikationens rader. Knapparna:
  - "Öppna" anropar `ReopenPayrollRun`.
  - "Bokför" anropar `BookPayrollRun` och är bara aktiv från utbetalningsdagen. Före den visas "Kan bokföras från {datum}". Servern kontrollerar datumet ändå.
- **Bokförd:** skrivskyddad, med en länk till verifikationen. "Öppna" finns inte.
  - Knappen "Backa bokföring" visar en förklaring, "En rättelse bokförs med dagens datum. Körningen blir färdigställd igen.", och knappen "Bekräfta backning". Den anropar `UnbookPayrollRun`. Bekräftelsen fungerar på samma sätt som "Bekräfta rättelse" i grundboken.

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
- `payroll_run_not_open`: "Lönekörningen är färdigställd. Öppna den för att ändra."
- `payroll_run_not_finalized`: "Lönekörningen är inte färdigställd."
- `payroll_run_booked`: "Lönekörningen är bokförd. Backa bokföringen först."
- `payroll_run_not_booked`: "Lönekörningen är inte bokförd."
- `payroll_run_not_due`: "Lönekörningen kan inte bokföras före utbetalningsdagen."
- `payroll_run_outdated`: "Avgifterna har ändrats sedan körningen färdigställdes. Öppna och färdigställ den igen."

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
  - tidigare bruttolön i månaden på 20 000 kr och nu 20 000 kr ger 5 000 kr till 20,81 % och 15 000 kr till 31,42 %,
  - avrundning av halva öre.
- `validate_draft` och `compute_lines`:
  - varje felkod,
  - standardtexten,
  - utbetalningsdag i framtiden går bra,
  - `salary_account` kopieras,
  - taket räknas bara mot bokförda körningar i samma månad, inte mot öppna, färdigställda eller backade.
- `voucher_lines`: raderna balanserar, grupperas per lönekonto och utelämnar rader med 0.
- Status och övergångar, given/when/then:
  - Skapa → Öppen. Färdigställ → Färdigställd. Öppna → Öppen, och de låsta raderna är borta. Bokför → Bokförd. Med verifikationen i `reversed` → Färdigställd. Bokför igen → Bokförd med två bokföringar i historiken.
  - Varje rad i övergångstabellen med fel status ger rätt fel.
  - `book_payroll_run`: utbetalningsdag efter `today` ger `PayrollRunNotDue`, samma dag går bra, och de låsta kontona används även om den anställda har bytt konto.
  - `book_payroll_run` ger `PayrollRunOutdated` när en annan körning i samma månad med samma unga anställda har bokförts efter färdigställandet.
- Anställda, given/when/then: dubblett, även hos en inaktiv, uppdatering av en inaktiv, och inaktivering två gånger utan event.

### Lagring (`crates/payroll/tests/store.rs`)
- Skapa och färdigställ med utbetalning i framtiden. Ingen verifikation skapas.
- `book_payroll_run` före utbetalningsdagen ger `PayrollRunNotDue`. På utbetalningsdagen ger den `VoucherRecorded` i `ledger-…` och `PayrollRunBooked` i `payroll-…`, och verifikationen har rätt rader och datum.
- `reopen_payroll_run` på en bokförd körning ger `PayrollRunBooked`.
- `unbook_payroll_run` ger en rättelseverifikation, och körningen blir Färdigställd. Den kan därefter öppnas, ändras, färdigställas och bokföras igen med ett nytt verifikationsnummer.
- Om verifikationen rättas direkt med `doris_ledger::correct_voucher` ger det också status Färdigställd.
- Ett stängt år eller ett inaktiverat lönekonto ger ledgerns fel, och ingenting skrivs. Körningen förblir Färdigställd.
- Ett dubblerat personnummer ger `DuplicateEmployee`.
- `rebuild_projections` från `read_all` ger samma fyra tabeller.
- Den som inte är medlem får `Error::NotFound`.
- `crates/ledger/tests/stress.rs` går fortfarande igenom.

### Server (gRPC-Web-integrationstester, `crates/server/tests/payroll.rs`)
- Flödet `AddEmployee` → `CreatePayrollRun` → `FinalizePayrollRun` → `BookPayrollRun` → `ListVouchers` visar verifikationen. `UnbookPayrollRun` → `ReopenPayrollRun` → `UpdatePayrollRun` går igenom.
- `BookPayrollRun` med utbetalning i morgon ger `payroll_run_not_due`, och `UpdatePayrollRun` på en färdigställd körning ger `payroll_run_not_open`.
- Ett ogiltigt personnummer ger `invalid_personal_identity_number`.
- Ett annat företags `company_id` ger `NotFound`.

### E2E (`e2e/`)
- Lägg till en anställd och skapa en körning med dagens datum: förhandsgranska, färdigställ och bokför. Verifikationen syns i grundboken med 7210, 2710, 1930, 7510 och 2731.
- Färdigställ en körning med utbetalning i morgon. "Bokför" är inaktiv. "Öppna" gör formuläret redigerbart igen.
- På en bokförd körning finns inte "Öppna". "Backa bokföring" ger en rättelse i grundboken och gör körningen färdigställd.
- Ett ogiltigt personnummer ger "Personnumret är ogiltigt (ÅÅÅÅMMDD-NNNN)."

### Avslutning
- `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
- `make dist` håller sig inom `WASM_BUDGET`.
- AGENTS.md uppdateras med:
  - `crates/payroll` i Layout och `payroll.proto` under API,
  - de nya koderna och avgiftsreglerna,
  - livscykeln Öppen → Färdigställd → Bokförd,
  - att bokföring sker tidigast på utbetalningsdagen,
  - att en bokförd körning backas med en rättelse innan den kan öppnas.

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
- Att kasta en öppen körning
