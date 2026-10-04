# Doris – Steg 10: Leverantörsfakturor

## Kontext
Steg 9 gav företagen ett leverantörsregister. I det här steget kan man registrera inkommande fakturor från leverantörerna, bifoga fakturan som underlag, låta Doris bokföra den och markera den som betald. Det är delprojekt 2 av fakturorna. Kundfakturor (delprojekt 3) kommer att återanvända det mesta: rader, moms, betalning och makulering.

Doris registrerar fakturor som redan finns. Den skapar dem inte.

### Fattade beslut
| Område | Beslut |
|---|---|
| Bokföringsmetod | Följer företagets metod automatiskt. **Faktureringsmetoden:** registreringen bokförs mot 2440, och betalningen bokför 2440 mot betalkontot. **Kontantmetoden:** inget bokförs vid registreringen, och betalningen bokför kostnad och moms mot betalkontot. |
| Årsskifte, kontantmetoden | BFL 5 kap. 2 § kräver att obetalda fakturor bokförs vid räkenskapsårets slut. Det ingår inte i det här steget. Sidan Räkenskapsår visar en varning när ett företag med kontantmetoden har obetalda leverantörsfakturor. |
| Rader | Konto, belopp exklusive moms och momssats (25, 12, 6 eller 0 %). Doris räknar ut momsen. Momsbeloppet får ändras med högst 1 kr. |
| Betalning | En betalning per faktura, alltid hela beloppet, med datum och betalkonto (19xx). Inga delbetalningar. |
| Fel | *Makulera* en obetald faktura och *ångra betalning* på en betald, båda med en anledning. Doris bokför rättelserna. Kreditfakturor från leverantörer ingår inte. |
| Plats | `crates/invoicing`, med strömmen `supplier-invoices-{company_id}` och `InvoicingService`. Verifikationerna bokförs i samma transaktion via `doris_ledger`. Beroendet går bara invoicing → ledger. |
| Leverantörsuppgifter | Fakturan sparar en kopia av leverantörens uppgifter vid registreringen. Att redigera leverantören ändrar inte gamla fakturor. |
| Underlag | Valfritt, precis som för verifikationer. Filen sparas när fakturan registreras och knyts till den verifikation som finns: registreringen med faktureringsmetoden, betalningen med kontantmetoden. |
| Åtkomst | Alla medlemmar i företaget. En icke-medlem får `company_not_found`. |
| Utanför steget | Delbetalningar, kreditfakturor, utländsk valuta, omvänd skattskyldighet och EU-inköp, årsskiftesbokning med kontantmetoden, betalfiler till banken, påminnelser, att lägga till underlag i efterhand och sökning. |

## Domän (`crates/invoicing/src/supplier_invoices.rs`, ren och utan I/O)

Fakturans egen logik får en egen modul, så att `domain.rs` (registren) inte växer. Det som kundfakturorna kommer att dela (rader och moms) läggs i `crates/invoicing/src/vat.rs`.

### Värdeobjekt
- `VatRate`: `Rate25`, `Rate12`, `Rate6` eller `Rate0`. Proton har 25, 12, 6 och 0 som heltal, och andra värden ger `InvalidVatRate`.
- `InvoiceLine { account: AccountNumber, net: i64, vat_rate: VatRate }`. `net` är i öre och ska vara > 0, annars `InvalidInvoiceLines`. `AccountNumber` kommer från `doris_ledger::domain`.
- `InvoiceLines`: 1–50 rader, annars `InvalidInvoiceLines`. Ett konto som är 2440 eller ligger i 2600–2699 ger `InvalidInvoiceAccount`. Att kontot finns och är aktivt kontrolleras av ledger när verifikationen bokförs. Med kontantmetoden kontrolleras det redan vid registreringen, mot kontoplanen.
- `vat::computed(lines) -> i64`: summan av nettot per momssats, gånger satsen, avrundad till hela ören (halva ören avrundas uppåt). Summan räknas med kontrollerad aritmetik.
- `vat::check(lines, given: Option<i64>) -> Result<i64>`: utan angivet belopp blir det det uträknade. Med angivet belopp måste det ligga inom ±100 öre från det uträknade och vara ≥ 0, annars `InvalidVatAmount`.
- `InvoiceNumber`: leverantörens fakturanummer, trimmat och 1–50 tecken, annars `InvalidInvoiceNumber`.
- `PaymentReference`: valfri, trimmad och högst 50 tecken, annars `InvalidReference`.
- Datum: förfallodatum före fakturadatum ger `InvalidDueDate`. Att fakturadatum inte ligger i framtiden kontrolleras av `doris_ledger` (`voucher_date_in_future`) med faktureringsmetoden, och av invoicing med samma kod med kontantmetoden.
- `PaymentAccount`: ett kontonummer 1900–1999, annars `InvalidPaymentAccount`. Att kontot är aktivt kontrolleras av ledger.
- `Reason`: återanvänder ledgers regel och kod (`invalid_reason`, 1–200 tecken).

### Leverantörskopian
`SupplierSnapshot { number, name, org_nr, bankgiro, plusgiro, iban, bic }` byggs från `Supplier` vid registreringen. En inaktiv leverantör ger `SupplierInactive`, och en okänd ger `SupplierNotFound`.

### Event (`schema_version` 1)
```rust
#[serde(tag = "type")]
pub enum SupplierInvoiceEvent {
    SupplierInvoiceRegistered {
        number: u32,
        supplier: SupplierSnapshot,
        invoice_number: InvoiceNumber,
        invoice_date: Date,
        due_date: Date,
        reference: Option<PaymentReference>,
        lines: Vec<InvoiceLine>,
        vat: i64,
        total: i64,
        attachments: Vec<Attachment>,   // doris_ledger::domain::Attachment
        voucher: Option<VoucherRef>,    // None with kontantmetoden
    },
    SupplierInvoicePaid { number: u32, date: Date, account: AccountNumber, voucher: VoucherRef },
    SupplierInvoiceCancelled { number: u32, reason: String, voucher: Option<VoucherRef> },
    SupplierInvoicePaymentReversed { number: u32, reason: String, voucher: VoucherRef },
}
```
`VoucherRef` lagras som `{ fiscal_year_start, number }`, och `doris_ledger::VoucherRef` får `Serialize`/`Deserialize`.

### Tillstånd och beslut
`SupplierInvoices { invoices: BTreeMap<u32, SupplierInvoice> }`. `SupplierInvoice` innehåller registreringens fält, status (`Unpaid`, `Paid { date, account, voucher }` eller `Cancelled`) och de verifikationer som hör till fakturan.

- `register(state, cmd) -> Registered`: nästa nummer (`max + 1`), med validerade rader, moms och total. Kontrollen av dubbletter mot icke-makulerade fakturor med samma leverantör och fakturanummer görs här (`DuplicateSupplierInvoice`), och backas upp av projektionens unika index.
- `pay(state, number)`: en obetald faktura går att betala. En betald ger `SupplierInvoicePaid`, en makulerad ger `SupplierInvoiceCancelled`, och ett okänt nummer ger `SupplierInvoiceNotFound`.
- `cancel(state, number)`: bara en obetald faktura går att makulera. En betald ger `SupplierInvoicePaid`, och en redan makulerad ger `SupplierInvoiceCancelled`.
- `reverse_payment(state, number)`: bara en betald faktura. En obetald ger `SupplierInvoiceNotPaid`, och en makulerad ger `SupplierInvoiceCancelled`.
- Efter en ångrad betalning är fakturan obetald igen och kan betalas på nytt.
- *Förfallen* sparas aldrig. Den räknas fram som obetald med förfallodatum före i dag.

### Verifikationer (`voucher_lines`, ren funktion)
Texten blir `"Leverantörsfaktura {nr}, {leverantör} ({fakturanummer})"`.

| Händelse | Faktureringsmetoden | Kontantmetoden |
|---|---|---|
| Registrera (datum = fakturadatum) | debet varje rad (netto), debet 2640 (moms, om > 0), kredit 2440 (total) | ingen |
| Betala (datum = betaldatum) | debet 2440, kredit betalkontot (total) | debet varje rad, debet 2640, kredit betalkontot |
| Makulera | `correct_voucher_in` på registreringen | ingen |
| Ångra betalning | `correct_voucher_in` på betalningen | `correct_voucher_in` på betalningen |

En rättelse dateras i dag, eller räkenskapsårets sista dag om i dag ligger efter det året. Ett stängt år ger `fiscal_year_closed`, som i ledger.

## Ledger: fyra små ändringar
- `VoucherRef` får `Serialize`, `Deserialize`, `Eq` och `Hash`.
- `pub async fn store_attachment_in(conn, new: NewAttachment) -> Result<Attachment>`: kontrollerar filtypen, storleken och namnet via `Attachment::new` och lägger filen i `attachment_files` med `INSERT OR IGNORE`. Ingen verifikation knyts till filen.
- `pub async fn link_attachment_in(conn, company_id, actor, fiscal_year_start, number, attachment: Attachment, today) -> Result<()>`: lägger till `AttachmentAdded` för en fil som redan är sparad. Den befintliga `add_attachment_in` skrivs om som de två efter varandra, och uppför sig likadant.
- `pub async fn check_accounts_in(conn, company_id, actor, accounts: &[AccountNumber]) -> Result<()>`: kontrollerar mot kontoplanen att varje konto finns (`account_not_found`) och är aktivt (`account_inactive`). Den används vid registrering med kontantmetoden, där ingen verifikation bokförs som kan kontrollera det.

## Lagring och transaktioner

### Skrivordning
Varje skrivning sker i en `BEGIN IMMEDIATE`-transaktion:
1. Medlemskontroll.
2. Ladda strömmen `supplier-invoices-{company_id}` och, vid registrering, leverantörsregistret.
3. `decide`.
4. Bokför verifikationen med `record_voucher_in`/`correct_voucher_in` och spara och knyt underlagen.
5. Append till fakturaströmmen med förväntad version, och uppdatera projektionerna.

Allt sker eller inget. En avvisad registrering förbrukar inget verifikationsnummer och inget fakturanummer.

### Migration `migrations/0010_supplier_invoices.sql`
```sql
CREATE TABLE supplier_invoices (
    company_id      TEXT    NOT NULL REFERENCES companies(company_id),
    number          INTEGER NOT NULL,
    supplier_number INTEGER NOT NULL,
    invoice_number  TEXT    NOT NULL,
    status          TEXT    NOT NULL,  -- 'unpaid', 'paid' or 'cancelled'
    details         TEXT    NOT NULL,  -- the invoice as JSON, as listed
    PRIMARY KEY (company_id, number)
);
-- One live invoice per supplier and invoice number (dubbelregistrering).
CREATE UNIQUE INDEX supplier_invoices_no_duplicates
    ON supplier_invoices (company_id, supplier_number, invoice_number)
    WHERE status <> 'cancelled';

CREATE TABLE supplier_invoice_attachments (
    company_id   TEXT    NOT NULL,
    number       INTEGER NOT NULL,
    sha256       TEXT    NOT NULL REFERENCES attachment_files(sha256),
    file_name    TEXT    NOT NULL,
    content_type TEXT    NOT NULL,
    size         INTEGER NOT NULL,
    PRIMARY KEY (company_id, number, sha256)
);
```
Båda projektionerna går att bygga om från `read_all`. En fil kan bara läsas via `supplier_invoice_attachments` för företagets egen faktura, aldrig enbart med hashen.

## API (`InvoicingService`)
```proto
import "doris/ledger/v1/ledger.proto";

rpc ListSupplierInvoices(ListSupplierInvoicesRequest) returns (ListSupplierInvoicesResponse);
rpc RegisterSupplierInvoice(RegisterSupplierInvoiceRequest) returns (RegisterSupplierInvoiceResponse);
rpc PaySupplierInvoice(PaySupplierInvoiceRequest) returns (PaySupplierInvoiceResponse);
rpc CancelSupplierInvoice(CancelSupplierInvoiceRequest) returns (CancelSupplierInvoiceResponse);
rpc ReverseSupplierInvoicePayment(ReverseSupplierInvoicePaymentRequest) returns (ReverseSupplierInvoicePaymentResponse);
rpc GetSupplierInvoiceAttachment(GetSupplierInvoiceAttachmentRequest) returns (GetSupplierInvoiceAttachmentResponse);

message InvoiceLine { uint32 account = 1; int64 net = 2; uint32 vat_rate = 3; }
message VoucherRef { string fiscal_year_start = 1; uint32 number = 2; }

message SupplierInvoice {
  uint32 number = 1;
  uint32 supplier_number = 2;
  string supplier_name = 3;
  string invoice_number = 4;
  string invoice_date = 5;
  string due_date = 6;
  string reference = 7;
  repeated InvoiceLine lines = 8;
  int64 vat = 9;
  int64 total = 10;
  string status = 11;                 // "unpaid", "paid" or "cancelled"
  string paid_date = 12;              // "" unless paid
  repeated VoucherRef vouchers = 13;  // registration, payments, corrections, in order
  repeated doris.ledger.v1.Attachment attachments = 14;
  string bankgiro = 15;               // from the supplier copy, for paying
  string plusgiro = 16;
  string iban = 17;
}

message RegisterSupplierInvoiceRequest {
  string company_id = 1;
  uint32 supplier_number = 2;
  string invoice_number = 3;
  string invoice_date = 4;
  string due_date = 5;
  string reference = 6;
  repeated InvoiceLine lines = 7;
  optional int64 vat = 8;             // unset: computed
  repeated doris.ledger.v1.NewAttachment attachments = 9;
}
message RegisterSupplierInvoiceResponse { uint32 number = 1; }
message PaySupplierInvoiceRequest { string company_id = 1; uint32 number = 2; string date = 3; uint32 account = 4; }
message CancelSupplierInvoiceRequest { string company_id = 1; uint32 number = 2; string reason = 3; }
message ReverseSupplierInvoicePaymentRequest { string company_id = 1; uint32 number = 2; string reason = 3; }
message GetSupplierInvoiceAttachmentRequest { string company_id = 1; uint32 number = 2; string sha256 = 3; }
message GetSupplierInvoiceAttachmentResponse { doris.ledger.v1.Attachment attachment = 1; bytes data = 2; }
```
`ListSupplierInvoices` returnerar alla fakturor, sorterade med nyast först.
`// ponytail: no pagination; add it when a company has thousands of invoices.`
Datum skickas som `YYYY-MM-DD`, och ett ogiltigt datum ger `invalid_date`.

## Server
- `InvoicingService` tar emot meddelanden på upp till 21 MiB och skickar upp till 11 MiB, som `LedgerService`. Samma gränser gäller för underlagen: 10 MiB per fil och 20 MiB per anrop.
- `session_gate` (`crates/server/src/lib.rs`) gäller även `/doris.invoicing.v1.InvoicingService/`, så ett anrop utan session får `not_signed_in` innan kroppen läses.
- `crates/server/src/ledger.rs`: `status` blir `pub(crate)`. `doris_invoicing::Error::Ledger(doris_ledger::Error)` mappas med den, så fel från bokföringen behåller sina koder. Det gäller till exempel `account_not_found`, `account_inactive`, `voucher_date_in_future`, `fiscal_year_closed`, `invalid_reason`, `unsupported_attachment_type` och `attachment_too_large`.

| Ny kod | Status |
|---|---|
| `supplier_invoice_not_found` | not_found |
| `supplier_inactive` | failed_precondition |
| `invalid_invoice_number` | invalid_argument |
| `duplicate_supplier_invoice` | already_exists |
| `invalid_due_date` | invalid_argument |
| `invalid_reference` | invalid_argument |
| `invalid_invoice_lines` | invalid_argument |
| `invalid_vat_rate` | invalid_argument |
| `invalid_vat_amount` | invalid_argument |
| `invalid_invoice_account` | invalid_argument |
| `invalid_payment_account` | invalid_argument |
| `supplier_invoice_paid` | failed_precondition |
| `supplier_invoice_not_paid` | failed_precondition |
| `supplier_invoice_cancelled` | failed_precondition |

## Frontend (`crates/web`)
- **`/supplier-invoices`, Leverantörsfakturor:**
  - En tabell med Nr, Leverantör, Fakturanr, Fakturadatum, Förfaller, Belopp och Status (Obetald, Förfallen, Betald eller Makulerad).
  - Kryssrutan "Visa betalda och makulerade" är urkryssad från början.
  - Knappen "Ny leverantörsfaktura" leder till formuläret.
  - En rad kan fällas ut och visar:
    - raderna (konto, netto och momssats), moms och att betala, samt leverantörens bankgiro, plusgiro och IBAN
    - verifikationerna som länkar till grundboken
    - underlagen, som öppnas i ny flik via `attachments.rs`
    - knapparna *Betala* (formulär med datum i dag och betalkonto 1930), *Makulera* och *Ångra betalning*. De två sista visar ett fält för anledning och en knapp för att bekräfta.
- **`/supplier-invoices/new`, Ny leverantörsfaktura:**
  - Fälten är leverantör (aktiva leverantörer), fakturanummer, fakturadatum, förfallodatum (fakturadatum + 30 fylls i när fakturadatum anges), OCR/meddelande och underlag (som i Ny verifikation).
  - Rader med konto (förslagslista från kontoplanen), belopp exklusive moms och momssats (25 % förvalt), plus en knapp för att lägga till en rad.
  - Momsfältet förifylls med det uträknade beloppet så länge man inte har ändrat det själv.
  - Netto och att betala visas och uppdateras medan man skriver.
  - Knappen heter "Registrera". När registreringen lyckas kommer man tillbaka till listan.
- **Räkenskapsår:** för ett företag med kontantmetoden och obetalda leverantörsfakturor visas texten "Det finns obetalda leverantörsfakturor. Med kontantmetoden ska de bokföras vid räkenskapsårets slut (BFL 5 kap. 2 §). Doris gör inte det än."
- Länken "Leverantörsfakturor" läggs till i bokföringsraden.
- `errors.rs` får en rad per ny kod.
- `make dist` måste hålla wasm-budgeten (500 KB gzip).

## Tester (TDD: röd → grön → refaktor, en commit per cykel)
- **Domän** (utan databas):
  - Moms per sats med avrundning, till exempel 3 rader à 0,33 kr med 25 % och hur 12 % och 6 % avrundas.
  - Ett ändrat momsbelopp: ±100 öre godtas, 101 öre avvisas, och ett negativt belopp avvisas.
  - Verifikationsraderna för båda metoderna vid registrering och betalning, inklusive en faktura utan moms.
  - Alla övergångar och deras fel, samt dubbletter.
  - Varje valideringsfel ger sin egen kod.
- **Crate** (`crates/invoicing/tests/supplier_invoices.rs`):
  - Med faktureringsmetoden: registrering → verifikation med underlag → betalning → 2440 har saldo 0 i saldobalansen.
  - Med kontantmetoden: ingen verifikation vid registreringen, och vid betalningen en verifikation med kostnad, moms och underlaget.
  - Makulering och ångrad betalning skapar rättelser. En ångrad betalning gör fakturan obetald, och den kan betalas igen.
  - Ett inaktivt konto i en rad ger `account_inactive`, och efteråt finns varken faktura, verifikation eller förbrukat nummer.
  - En faktura i ett stängt år ger `fiscal_year_closed`.
  - Dubbletter: två samtidiga registreringar av samma fakturanummer från samma leverantör ger exakt en faktura. Efter en makulering går samma nummer att registrera igen.
  - Projektionerna byggs om från `read_all`.
  - En icke-medlem nekas.
  - `crates/ledger/tests/stress.rs` ska fortfarande gå igenom.
- **Server:**
  - gRPC-Web-test för varje RPC och varje ny kod, plus en ledger-kod som skickas vidare.
  - Ett anrop utan session till `InvoicingService` med en stor kropp får `not_signed_in` innan kroppen läses.
  - Ett underlag går bara att hämta via företagets egen faktura.
- **E2E** (`e2e/tests/supplier_invoices.spec.ts`):
  - Med faktureringsmetoden: registrera med underlag → verifikationen syns i grundboken → betala → ångra betalningen → betala igen.
  - Med kontantmetoden: registrera → ingen verifikation → betala → verifikationen har underlaget. Varningen visas på Räkenskapsår så länge fakturan är obetald.
  - Makulera.

## Dokumentation
AGENTS.md uppdateras:
- `InvoicingService` för leverantörsfakturor.
- Gränserna 21/11 MiB.
- Regeln om underlag: de läses bara via företagets egen verifikation eller egen leverantörsfaktura.
- De nya felkoderna.
