# Doris – Momsdeklaration

## Kontext
Doris bokför moms på 2611/2621/2631 och 2640 via kund- och
leverantörsfakturor och verifikationer, men redovisar den inte. I det här
steget tar Doris fram momsdeklarationen per redovisningsperiod ur
bokföringen, visar den som Skatteverkets blankett (SKV 4700), skriver en fil
som användaren laddar upp i Skatteverkets e-tjänst och bokför
momsavräkningen när perioden markeras inlämnad. Doris kommer ihåg vad som
lämnades, visar när en period ändrats och tar då fram en ny deklaration.

### Fattade beslut
| Område | Beslut |
|---|---|
| Underlag | Ledgerns verifikationsrader i perioden. Faktura- och kontantmetoden blir rätt av sig själva, eftersom momsen är bokförd när den ska redovisas. Doris egna avräkningsverifikationer (och rättelser av dem) räknas inte. |
| Rutor | Varje konto har högst en ruta på blanketten. Standard från BAS i `bas.rs`; användaren kan ändra eller ta bort rutan per konto (händelse i kontoplanen). Alla rutor 05–62 går att välja, utom 49 som räknas fram. |
| Period | Inställning per räkenskapsår: månad, kvartal, helår eller ej momsregistrerad. Standard kvartal. Månad och kvartal är kalenderperioder även vid brutet räkenskapsår; helår är räkenskapsåret. Byts periodtyp mellan två år börjar årets första period dagen efter förra årets sista period (räknad utan denna justering), så att ingen månad deklareras två gånger eller aldrig. |
| Avrundning | Varje ruta summeras i öre, sedan stryks ören (mot noll). Ruta 49 räknas på de avrundade rutorna. |
| Avräkning | Bokförs när perioden markeras inlämnad, i samma transaktion: momskontona mot 2650, öresdifferensen på 3740, daterad periodens sista dag. En ny inlämning bokför bara skillnaden. |
| Inlämning | Doris skriver filen; användaren laddar upp och signerar hos Skatteverket och markerar sedan perioden inlämnad. Doris skickar inget själv. |
| Ändringar | Status räknas fram mot senaste inlämningen. En ändrad period lämnas in igen i sin helhet, som Skatteverket vill. |
| Format | eSKD `eSKDUpload Version="6.0"`, ISO-8859-1, skrivet som text utan XML-modul. |
| Deklarationsdag | Månad och kvartal (omsättning ≤ 40 MSEK): den 12:e i andra månaden efter perioden, den 17:e om den månaden är januari eller augusti, flyttad till nästa vardag. Helår: ingen dag visas, bara en länk till Skatteverket. |
| Förhandsvisning | Följer SKV 4700: samma sektioner (A–I), rubriker, radtexter och rutnummer, i två kolumner som blanketten. Endast tokens, så ljust och mörkt fungerar. |

Källor: Skatteverket, "Lämna momsdeklaration via fil i e-tjänsten"
(elementnamn och exempelfil), "När ska jag deklarera moms?"
(deklarationsdagar) och blanketten SKV 4700.

## Skatteverkets format (det som används)
```xml
<?xml version="1.0" encoding="ISO-8859-1"?>
<eSKDUpload Version="6.0">
<OrgNr>556000-0175</OrgNr>
<Moms>
<Period>202403</Period>
<ForsMomsEjAnnan>100000</ForsMomsEjAnnan>
…
<MomsBetala>225500</MomsBetala>
</Moms>
</eSKDUpload>
```
- `OrgNr`: `NNNNNN-NNNN`. För enskild firma ägarens personnummer i samma
  form (det 10-siffriga `OrgNr`).
- `Period`: `ÅÅÅÅMM`, periodens sista månad (även för kvartal och helår).
- Belopp: hela kronor, minustecken direkt före siffrorna. Rutor som är noll
  utelämnas; `MomsBetala` skrivs alltid.
- Elementen skrivs i exempelfilens ordning:

| Ruta | Element | Ruta | Element |
|---|---|---|---|
| 05 | `ForsMomsEjAnnan` | 38 | `ForsVaruMellan3p` |
| 06 | `UttagMoms` | 39 | `ForsTjSkskAnnatEg` |
| 07 | `UlagMargbesk` | 40 | `ForsTjOvrUtomEg` |
| 08 | `HyrinkomstFriv` | 41 | `ForsKopareSkskSverige` |
| 20 | `InkopVaruAnnatEg` | 42 | `ForsOvrigt` |
| 21 | `InkopTjanstAnnatEg` | 10 | `MomsUtgHog` |
| 22 | `InkopTjanstUtomEg` | 11 | `MomsUtgMedel` |
| 23 | `InkopVaruSverige` | 12 | `MomsUtgLag` |
| 24 | `InkopTjanstSverige` | 30 | `MomsInkopUtgHog` |
| 50 | `MomsUlagImport` | 31 | `MomsInkopUtgMedel` |
| 35 | `ForsVaruAnnatEg` | 32 | `MomsInkopUtgLag` |
| 36 | `ForsVaruUtomEg` | 60 | `MomsImportUtgHog` |
| 37 | `InkopVaruMellan3p` | 61 | `MomsImportUtgMedel` |
| | | 62 | `MomsImportUtgLag` |
| | | 48 | `MomsIngAvdr` |
| | | 49 | `MomsBetala` |

(Ordningen i tabellen är kolumnvis: vänster kolumn först, sedan höger.)

## Ledger: momsruta per konto

### Domän (`crates/ledger/src/vat_box.rs`, ren)
```rust
/// A box on Skatteverket's momsdeklaration (SKV 4700). 49 is computed
/// and never set on an account.
pub enum VatBox { B05, B06, B07, B08, B10, B11, B12, B20, B21, B22, B23,
                  B24, B30, B31, B32, B35, B36, B37, B38, B39, B40, B41,
                  B42, B48, B50, B60, B61, B62 }
```
- Serialiseras som rutnumret (`u32`); `VatBox::parse(n)` ger
  `InvalidVatBox` för allt annat, även 49.
- `VatBox::side()`: `Credit` för 05–08, 10–12, 30–32, 35–42, 60–62,
  `Debit` för 20–24, 48, 50.
- `VatBox::is_vat()`: sant för 10–12, 30–32, 60–62 och 48, de konton
  avräkningen nollar.
- `bas::default_vat_box(number) -> Option<VatBox>`, slås upp på
  kontonumret (även konton som läggs till senare):

| Konton | Ruta |
|---|---|
| 3001, 3002, 3003, 3106 | 05 |
| 3004 | 42 |
| 3108 | 35 |
| 3305 | 40 |
| 3308 | 39 |
| 2611 | 10 |
| 2621 | 11 |
| 2631 | 12 |
| 4515, 4516, 4517 | 20 |
| 4535, 4536, 4537 | 21 |
| 4531, 4532, 4533 | 22 |
| 4415, 4416, 4417 | 23 |
| 4425, 4426, 4427 | 24 |
| 2614 | 30 |
| 2624 | 31 |
| 2634 | 32 |
| 4545, 4546, 4547 | 50 |
| 2615 | 60 |
| 2625 | 61 |
| 2635 | 62 |
| 2640, 2641, 2645, 2647 | 48 |

### Händelse (kontoplanens ström, `schema_version` 1)
```rust
/// The account's box on the momsdeklaration. None: no box, even where
/// BAS has one.
AccountVatBoxSet { number: AccountNumber, vat_box: Option<VatBox> },
```
- `Account` får `vat_box: Option<VatBox>`: den senaste händelsen om det
  finns någon, annars `default_vat_box`. Befintliga företag behöver ingen
  migrering.
- `set_account_vat_box(chart, number, vat_box)`: kontot måste finnas
  (`AccountNotFound`). En ruta på 2650 eller 3740, som avräkningen själv
  bokför på, ger `InvalidVatBox`; att ta bort rutan går alltid. Samma ruta
  som nu ger inga händelser.

### Lagring
Migration `migrations/0015_vat.sql`: `accounts` får kolumnen
`vat_box INTEGER` (NULL = ingen). `rebuild_projections` sätter den från
`seed_chart`, `AccountAdded` (standardvärdet) och `AccountVatBoxSet`.

### Query (`crates/ledger/src/queries.rs`)
```rust
/// Each account with a box: its box and its saldo (debit − credit, öre)
/// over the company's vouchers dated from..=to, leaving out `exclude`
/// and every voucher that corrects one of them. Read in the caller's
/// transaction.
pub async fn vat_box_totals_in(
    tx: &mut SqliteConnection, company_id: Uuid, from: Date, to: Date,
    exclude: &[(Date, u32)],   // (fiscal year start, voucher number)
) -> Result<Vec<VatAccountTotal>>  // { number, name, vat_box, saldo }
```
Konton med ruta men utan rader i perioden kommer inte med.

### API
`LedgerService.SetAccountVatBox(company_id, number, vat_box)`;
`Account.vat_box` (0 = ingen). Koder: `invalid_vat_box`, och
`account_not_found` som förut.

## `doris-vat` (`crates/vat`)

### Värdeobjekt
- `VatPeriodKind`: `Monthly | Quarterly | Yearly | NotRegistered`.
- `VatPeriod { start: Date, end: Date }`. `periods(fiscal_year, kind)`
  ger årets perioder: månader och kalenderkvartal vars **sista månad**
  ligger i räkenskapsåret, med hela kalenderperioden (en period kan börja i
  året innan); helår ger räkenskapsåret; `NotRegistered` ger inga.
  `periods_after(fiscal_year, kind, previous)` låter årets första period
  börja dagen efter förra årets sista (förra årets egna `periods`); det är
  den listan som används. Första året och ett år efter ett ej
  momsregistrerat år får de naturliga perioderna.
  Perioden identifieras av `end`; i URL och fil som `ÅÅÅÅMM`.
- `Boxes`: belopp i hela kronor per ruta (alla 29 rutor), plus ruta 49.

### Händelser (ström `vat-{company_id}`, `schema_version` 1)
```rust
/// How often the company declares VAT in a fiscal year.
VatPeriodSet { fiscal_year_start: Date, kind: VatPeriodKind },
/// What was declared for a period, as the user confirms after uploading
/// the file, and the settlement voucher booked with it.
VatReturnSubmitted {
    period_end: Date,
    accounts: Vec<(u32, VatBox, i64)>, // every account with a box, saldo in öre
    boxes: Boxes,                      // what was declared
    settled: Vec<(u32, i64)>,          // what the voucher booked per VAT account
    vat_due: i64,                      // what it booked on 2650 (öre, + = to pay)
    voucher: (Date, u32),              // fiscal year start, number
},
```
Vem och när ligger i händelsens metadata.

### Beräkning (ren, `crates/vat/src/domain.rs`)
- `boxes(totals) -> Boxes`: per ruta summan av kontonas saldo, med omvänt
  tecken för `Credit`-rutor; ören stryks mot noll.
  Ruta 49 = 10+11+12+30+31+32+60+61+62 − 48.
- `settlement_lines(totals, boxes, earlier) -> Vec<VoucherLine>`, där
  `earlier` är de inlämningar för perioden vars verifikation inte är rättad:
  - varje konto som nu har en momsruta (`is_vat`) eller finns i ett
    tidigare `settled`: rad = −(saldo nu, 0 om kontot inte längre har en
    momsruta, − summan av tidigare `settled`);
  - 2650: ruta 49 i öre (×100) − tidigare `vat_due`, kredit när positiv;
  - 3740: det som återstår för att verifikationen ska balansera.
  - Rader som blir noll tas bort.
- `fingerprint(period_end, accounts)`: hex SHA-256 av den kanoniska JSON som
  `VatReturnSubmitted.accounts` skulle få.
- `status(period, today, latest, corrected)`:
  - `InProgress` om `period.end >= today`,
  - `ToSubmit` om ingen inlämning finns,
  - `Changed` om `accounts` nu skiljer sig från senaste inlämningen eller
    någon inlämnings verifikation är rättad,
  - annars `Submitted`.
- `due_date(period, kind) -> Option<Date>`: `None` för helår; annars den 12:e
  i andra månaden efter `period.end` (17:e om den månaden är januari eller
  augusti), flyttad framåt över lördag, söndag och helgdag. Bara
  långfredagen, annandag påsk och Kristi himmelsfärdsdag kan infalla den
  12:e eller 17:e; påskdagen räknas fram (Gauss).
  `// ponytail:` omsättning över 40 MSEK (den 26:e) och helårsdagar stöds
  inte; lägg till när ett sådant företag använder Doris.
- `eskd_xml(org_nr, period_end, boxes) -> Vec<u8>`: filen i ISO-8859-1.
- `vat_number(org_nr)`: `SE` + de tio siffrorna + `01`.

### Kommandon (`decide`)
- `set_vat_period(state, fiscal_year_start, kind)`: `VatPeriodLocked` om
  någon period i året har en inlämning.
- `submit_vat_return(state, period, today, totals, fingerprint, corrected)`:
  - `VatNotRegistered` om året är `NotRegistered`,
  - `VatPeriodNotEnded` om `period.end >= today`,
  - `VatReturnOutdated` om fingerprinten inte stämmer,
  - `VatReturnUnchanged` om status är `Submitted`,
  - annars voucher-raderna att bokföra. Händelsen skapas när ledgern gett
    verifikationsnumret.

### Lagring
Migration `0015_vat.sql` skapar också:
```sql
CREATE TABLE vat_periods (
    company_id TEXT NOT NULL, fiscal_year_start TEXT NOT NULL,
    kind TEXT NOT NULL, PRIMARY KEY (company_id, fiscal_year_start));
CREATE TABLE vat_returns (
    company_id TEXT NOT NULL, period_end TEXT NOT NULL, seq INTEGER NOT NULL,
    payload TEXT NOT NULL,          -- VatReturnSubmitted as JSON
    submitted_at TEXT NOT NULL, submitted_by TEXT NOT NULL,
    PRIMARY KEY (company_id, period_end, seq));
```
Ingen främmande nyckel mot `vouchers`. Båda byggs om från `read_all`.

### `doris-vat` (lib)
- `set_vat_period`, `list_vat_returns`, `get_vat_return`, `export_vat_file`
  och `mark_vat_return_submitted`.
- `mark_vat_return_submitted` gör allt i en `BEGIN IMMEDIATE`:
  1. läs strömmen,
  2. `doris_ledger::vat_box_totals_in` (alla tidigare avräkningar uteslutna),
  3. `doris_ledger::corrected_vouchers_in` för avräkningarna,
  4. `decide`,
  5. `doris_ledger::record_voucher_in` med texten "Momsavräkning
     juli–september 2026" och periodens sista dag,
  6. lägg till `VatReturnSubmitted` och uppdatera projektionen.

  Nekar ledgern, till exempel `fiscal_year_closed`, rullas allt tillbaka.
- Företagets uppgifter (organisationsnummer, räkenskapsår) frågas från
  `doris-company`, och det sätter servern ihop.

## API (`proto/doris/vat/v1/vat.proto`, `VatService`)
```proto
rpc SetVatPeriod(SetVatPeriodRequest) returns (SetVatPeriodResponse);
  // company_id, fiscal_year_start, kind
rpc ListVatReturns(ListVatReturnsRequest) returns (ListVatReturnsResponse);
  // company_id, fiscal_year_start → kind, locked,
  // periods { start, end, status, due_date?, vat_due (kr) }
rpc GetVatReturn(GetVatReturnRequest) returns (GetVatReturnResponse);
  // company_id, period_end → period, status, due_date?, org_nr, vat_number,
  // boxes { box, amount (kr), accounts { number, name, saldo (öre) } },
  // booked_vat (öre), rounding (öre), fingerprint,
  // submissions { submitted_at, submitted_by_name, voucher_number, corrected }
rpc ExportVatFile(ExportVatFileRequest) returns (ExportVatFileResponse);
  // company_id, period_end → file_name, content (bytes), fingerprint
rpc MarkVatReturnSubmitted(MarkVatReturnSubmittedRequest)
    returns (MarkVatReturnSubmittedResponse);
  // company_id, period_end, fingerprint → voucher_number
```
- Koderna mappas i `crates/server/src/vat.rs` (`status`, `domain_status`):
  `invalid_vat_period`, `vat_period_not_ended`, `vat_period_locked`,
  `vat_return_outdated`, `vat_return_unchanged` och `vat_not_registered`.
  Ledgerns koder behålls. `invalid_vat_box` finns i `ledger.rs`.
- Den som lämnat in (`submitted_by_name`) frågas från identity, som för
  verifikationer.
- Meddelandena är små: tonics 4 MiB räcker och `session_gate` behöver inte
  utökas. Varje anrop kontrollerar session och medlemskap.

## Frontend
### `/vat?fy=`, Moms (under Bokföring)
- `PageHeader` "Moms" och räkenskapsårsväljaren (`?fy=`, standard året som
  innehåller i dag).
- Ett `narrow` `Card` med "Redovisningsperiod" (`select`: Månad, Kvartal,
  Helår, Ej momsregistrerad). Låst med förklaringen "Perioden kan inte
  ändras när en deklaration för året är inlämnad." när `locked`.
- `TableCard` med perioderna: Period, Status (`Badge`: Pågår, Att lämna,
  Inlämnad, Ändrad), Deklarationsdag, Att betala/få tillbaka och en länk.

### `/vat/{ÅÅÅÅMM}`, Momsdeklaration
- `PageHeader` "Momsdeklaration juli–september 2026" med status-`Badge`
  och åtgärderna "Ladda ner fil" och "Markera inlämnad…".
- Uppgiftsrad: Organisationsnummer, Deklarationsdag, Momsregistreringsnummer,
  Period i filen. Texten "Ange endast kronor, ej ören."
- Blanketten i två kolumner (vänster A, C, H, E; höger B, D, I, F, G),
  en kolumn under `md`. Varje sektion är ett kort med sektionens rubrik
  ordagrant från SKV 4700; varje rad har radtext, rutnummer och belopp.
  Tom ruta visas som "–". Ruta 48 visas med minustecken.
- En ruta med belopp är en knapp som fäller ut kontona bakom (nummer, namn,
  belopp i öre) med `aria-expanded`.
- Ruta 49 har ram i `--primary` och under sig "Bokförd moms" och
  "Öresavrundning (3740)".
- "Markera inlämnad…" öppnar en bekräftelse: "Doris bokför
  momsavräkningen daterad 2026-09-30." Efter svaret visas
  verifikationsnumret och statusen uppdateras.
- Tidigare inlämningar listas sist: när, vem, verifikation (länk), och
  "Rättad" om den är det.
- Endast tokens ur `input.css` (`--muted` för sektionsrubriker, `--accent`
  för en utfälld ruta, `--primary` för ruta 49); följer
  `docs/design/README.md`, även ljust, mörkt och 390 px.

### Övrigt
- Kontoplanen (`/accounts`) får kolumnen "Momsruta" med ett `select`
  (tom, och "05 Momspliktig försäljning" osv.).
- `nav.rs`: menyposten "Moms" under Bokföring; `section_of` får `/vat`.
- Översikten (`overview.rs`) får regler under "Att göra" från
  `ListVatReturns`: "Momsdeklaration juli–september ska lämnas senast
  12 november" (Att lämna) och "Momsdeklaration för april är ändrad – lämna
  en ny" (Ändrad).
- Räkenskapsår varnar för årets perioder som är Att lämna eller Ändrad.
- `errors.rs` får en rad per ny kod.
- Sidorna startar uppgifter med `crate::task::spawn_local`.

## Tester
### Domän (`crates/ledger/tests`, `crates/vat/tests/domain.rs`)
- `VatBox::parse` (49 och okända nummer nekas), `side`, `is_vat`;
  standardkartan; `AccountVatBoxSet` gäller före standard, även `None`;
  2650 och 3740 kan inte få en ruta.
- `boxes`: tecken per ruta, öresstrykning mot noll även för negativa
  belopp, ruta 49 från avrundade rutor.
- `settlement_lines`: balanserar; 3740 när moms ska betalas och tillbaka;
  andra inlämningen bokför bara skillnaden; en rättad avräkning räknas inte;
  ett konto som tappat sin momsruta nollas tillbaka.
- `periods`: kvartal vid brutet räkenskapsår, perioden hör till året där den
  slutar, helår, ej registrerad.
- `due_date`: 12:e, 17:e i januari och augusti, helg, långfredag/annandag
  påsk och Kristi himmelsfärd.
- `eskd_xml` mot en sparad fil (ISO-8859-1, nollrutor utelämnade,
  `MomsBetala` alltid, negativa belopp).
- `status` och `fingerprint`.
- `decide`: alla koder.

### Lagring och server
- `vat_box_totals_in` utesluter angivna verifikationer och deras rättelser.
- Projektionerna (`accounts.vat_box`, `vat_periods`, `vat_returns`) byggs
  om från `read_all`.
- `mark_vat_return_submitted` bokför verifikation och händelse i samma
  transaktion; nekar ledgern läggs ingen händelse till.
- Hela flödet med faktureringsmetoden och kontantmetoden: fakturor → rutor →
  inlämning → 26xx-kontona med momsruta har saldo 0, 2650 = ruta 49.
- Servern (gRPC-Web): varje RPC kräver session och medlemskap; kodmappning.

### E2E (`e2e/tests/vat.spec.ts`)
- Kundfaktura med moms och leverantörsfaktura → Moms → perioden visar
  ruta 05, 10, 48 och 49 → ladda ner filen → markera inlämnad →
  verifikationen syns i Verifikationer.
- En ny verifikation i perioden ger Ändrad; ny inlämning bokför bara
  skillnaden.
- Byt momsruta på ett konto i kontoplanen och se den i deklarationen.
- Designtesterna och `leaving.spec.ts` täcker `/vat` och `/vat/{period}`.

### Avslutning
- AGENTS.md: `doris-vat` i Layout, händelser och regler under Event
  sourcing, `VatService` och koderna under API.

## Utanför omfattningen
- Periodisk sammanställning (EU-försäljning).
- Omsättning över 40 MSEK (deklarationsdag den 26:e) och helårsdagar.
- Att skicka deklarationen direkt till Skatteverket (API).
- Upplysningsfältet (`TextUpplysningMoms`).
- Betalning av momsen (2650 mot bank bokförs som vanlig verifikation).
