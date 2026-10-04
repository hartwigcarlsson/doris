# Doris – Steg 12: Kundfakturor

## Kontext
Steg 9 gav kundregistret och steg 11 leverantörsfakturorna. Det här steget registrerar företagets utgående fakturor, bokför dem och tar emot inbetalningar. Det är delprojekt 3 av fakturorna. Doris registrerar fakturor som redan har skickats. Att skapa och skriva ut dem är delprojekt 4.

Modellen speglar leverantörsfakturorna (`docs/superpowers/specs/2026-10-04-leverantorsfakturor-design.md`). Det som är gemensamt bryts ut i stället för att kopieras.

### Fattade beslut
| Område | Beslut |
|---|---|
| Fakturanummer | Doris föreslår nästa nummer (det högsta numeriska + 1, annars 1), och det går att ändra. Numret är unikt per företag, även mot makulerade fakturor, eftersom en utfärdad fakturas nummer aldrig återanvänds. |
| Bokföringsmetod | Följer företagets metod. **Faktureringsmetoden:** registreringen bokförs mot 1510, och inbetalningen bokför betalkontot mot 1510. **Kontantmetoden:** inget bokförs vid registreringen, och inbetalningen bokför betalkontot mot intäkt och utgående moms. |
| Moms | Räknas ut per sats och avrundas till hela ören. Den bokförs på 2611 (25 %), 2621 (12 %) och 2631 (6 %). Momsen **går inte att ändra**: den har inget givet konto, och fakturan är företagets egen. |
| Förfallodatum | Förslaget är fakturadatum + kundens betalningsvillkor. |
| Fel | *Makulera* (bara obetald, med anledning) och *Ångra betalning* (med anledning), med rättelser som för leverantörsfakturor. Handrättelser följs. Kreditfakturor ingår inte, utan kommer ihop med fakturautskriften. |
| Betalning | En inbetalning per faktura, alltid hela beloppet. |
| Kod | Steg 11:s tillståndsmaskin, verifikationsbyggare, rättelsedatum och lagringshjälpare bryts ut till gemensam kod. Varje riktning har egna händelser, egen motpartskopia, egen numrering och egen dubblettregel. Leverantörsfakturornas händelser och beteende ändras inte. |
| Wasm-budget | Hanteras i en annan session. Det här steget ändrar inte budgeten. Om `make dist` går över budgeten rapporteras det men stoppar inte steget. |
| Utanför steget | Kreditfakturor, delbetalningar, påminnelser, fakturautskrift och PDF, utländsk valuta, omvänd skattskyldighet och EU-försäljning, årsskiftesbokning med kontantmetoden och att lägga till underlag i efterhand. |

## Gemensam kod

### `crates/invoicing/src/invoices.rs` (ny, ren)
- `Status { Unpaid, Paid { date, account, voucher }, Cancelled }` flyttas hit från `supplier_invoices.rs` med samma serde-form (`#[serde(tag = "status", rename_all = "snake_case")]`). Sparade leverantörsfakturor läses alltså oförändrat.
- `trait InvoiceKind { const NOT_FOUND, PAID, NOT_PAID, CANCELLED: DomainError; }` har en implementation per riktning.
- `unpaid::<K>(status: Option<&Status>) -> Result<(), DomainError>` och `paid::<K>(…) -> Result<VoucherRef, DomainError>`: övergångsreglerna, skrivna en gång.
- `Side { Debit, Credit }` och `posting(lines: &[InvoiceLine], vat: &[(AccountNumber, i64)], side: Side) -> Vec<VoucherLine>`: raderna och momsposterna på samma sida.
- `correction_date(fiscal_year_end, today)` och `voucher_text(String) -> String` (kapar till 200 tecken) flyttas hit.

### `vat.rs`
- `by_rate(lines) -> Vec<(VatRate, i64)>`: momsen per sats, i fallande sats och bara satser med moms > 0. `computed` blir summan av den.
- Kontrollen av otillåtna konton flyttas ut ur `InvoiceLine::new`. Raden kontrollerar bara 1000–8999, belopp och sats. Varje riktning spärrar sina egna konton:
  - leverantör: 2440 och 2600–2699
  - kund: 1510 och 2600–2699

  Båda ger `InvalidInvoiceAccount`.

### `lib.rs`
`load`, `link_all` och `correct` (som följer handrättelser) används av båda riktningarna. Leverantörsfakturornas befintliga tester ska gå igenom **oförändrade** efter utbrytningen.

## Domän (`crates/invoicing/src/customer_invoices.rs`, ren)

### Värden
- `InvoiceNumber`: återanvänds från steg 11 (trimmat, 1–50 tecken).
- `PaymentReference`, `payment_account` och `reason`: återanvänds.
- `CustomerSnapshot { number, name, org_nr, vat_number, address, email }` byggs från `Customer` vid registreringen. En inaktiv kund ger `CustomerInactive`, och en okänd ger `CustomerNotFound`.
- `NewCustomerInvoice<'a> { invoice_number, invoice_date, due_date, reference, lines }` har inget momsfält.
- `CustomerRegistration { customer, invoice_number, invoice_date, due_date, reference, lines, vat: Vec<(VatRate, i64)>, total }`:
  - `CustomerRegistration::new` kontrollerar fakturanumret, förfallodatum (`InvalidDueDate`), referensen, raderna (1–50) och de otillåtna kontona.
  - `total` är netto plus summan av momsen.
- `next_invoice_number(existing: impl Iterator<Item = &InvoiceNumber>) -> String`: högsta numret som bara består av siffror + 1, annars `"1"`. Ledande nollor räknas bort, men ett förslag efter `"0017"` blir `"18"`.

### Händelser (`schema_version` 1, ström `customer-invoices-{company_id}`)
```rust
#[serde(tag = "type")]
pub enum CustomerInvoiceEvent {
    CustomerInvoiceRegistered { number: u32, invoice: CustomerRegistration, attachments: Vec<Attachment>, voucher: Option<VoucherRef> },
    CustomerInvoicePaid { number: u32, date: Date, account: AccountNumber, voucher: VoucherRef },
    CustomerInvoiceCancelled { number: u32, reason: String, voucher: Option<VoucherRef> },
    CustomerInvoicePaymentReversed { number: u32, reason: String, voucher: VoucherRef },
}
```

### Tillstånd och beslut
`CustomerInvoices` och `CustomerInvoice { number, invoice, attachments, status, vouchers, registration_voucher }` speglar `SupplierInvoices`.
- `register(state, &CustomerRegistration) -> Result<u32>`: nästa interna nummer. Samma fakturanummer som en befintlig faktura, **makulerad eller inte**, ger `DuplicateCustomerInvoice`.
- `unpaid` och `paid` kommer från `invoices.rs`. Felen är `CustomerInvoiceNotFound`, `CustomerInvoicePaid`, `CustomerInvoiceNotPaid` och `CustomerInvoiceCancelled`.

### Verifikationer
Texten blir `"Kundfaktura {fakturanummer}, {kund}"`, kapad till 200 tecken. Momsposterna är `(2611, moms 25 %)`, `(2621, moms 12 %)` och `(2631, moms 6 %)`, och bara de som är > 0 tas med.

| Händelse | Faktureringsmetoden | Kontantmetoden |
|---|---|---|
| Registrera (fakturadatum) | debet 1510 (total), kredit raderna och momsposterna | ingen |
| Inbetalning (betaldatum) | debet betalkontot (total), kredit 1510 | debet betalkontot (total), kredit raderna och momsposterna |
| Makulera | `correct` av registreringen | ingen |
| Ångra betalning | `correct` av inbetalningen | `correct` av inbetalningen |

Underlaget sparas vid registreringen och knyts till registreringsverifikationen med faktureringsmetoden, och till varje inbetalningsverifikation med kontantmetoden. Samma fil två gånger i ett anrop ger `duplicate_attachment`. Med kontantmetoden kontrolleras radernas konton vid registreringen med `check_accounts_in`.

## Lagring

### Skrivordning
Varje skrivning är en `BEGIN IMMEDIATE`-transaktion:
1. Medlemskontroll.
2. Ladda kundregistret vid registrering, och fakturaströmmen.
3. `decide`.
4. Bokför verifikationen och spara och knyt underlagen.
5. Append med förväntad version och uppdatera projektionen.

En avvisad registrering förbrukar inget internt nummer och inget verifikationsnummer.

### Migration `migrations/0012_customer_invoices.sql`
```sql
CREATE TABLE customer_invoices (
    company_id      TEXT    NOT NULL REFERENCES companies(company_id),
    number          INTEGER NOT NULL,
    customer_number INTEGER NOT NULL,
    invoice_number  TEXT    NOT NULL,
    status          TEXT    NOT NULL,
    details         TEXT    NOT NULL,
    PRIMARY KEY (company_id, number)
);
-- An issued invoice number is never reused, not even after cancelling.
CREATE UNIQUE INDEX customer_invoices_unique_number
    ON customer_invoices (company_id, invoice_number);
```
Projektionen går att bygga om från `read_all`. Underlag läses som för leverantörsfakturor: först företagets egen faktura, sedan `attachment_files`.

### Publik API (`doris_invoicing`)
```rust
pub async fn register_customer_invoice(pool, company_id, actor, customer: u32, new: NewCustomerInvoice<'_>, attachments: Vec<NewAttachment>, today) -> Result<u32>
pub async fn pay_customer_invoice(pool, company_id, actor, number, date, account: u32, today) -> Result<()>
pub async fn cancel_customer_invoice(pool, company_id, actor, number, reason: &str, today) -> Result<()>
pub async fn reverse_customer_invoice_payment(pool, company_id, actor, number, reason: &str, today) -> Result<()>
pub async fn list_customer_invoices(pool, company_id, actor) -> Result<(Vec<CustomerInvoice>, String)>  // newest first, next invoice number
pub async fn customer_invoice_attachment(pool, company_id, actor, number, sha256: &str) -> Result<(Attachment, Vec<u8>)>
```

## API (`InvoicingService`)
```proto
rpc ListCustomerInvoices(ListCustomerInvoicesRequest) returns (ListCustomerInvoicesResponse);
rpc RegisterCustomerInvoice(RegisterCustomerInvoiceRequest) returns (RegisterCustomerInvoiceResponse);
rpc PayCustomerInvoice(PayCustomerInvoiceRequest) returns (PayCustomerInvoiceResponse);
rpc CancelCustomerInvoice(CancelCustomerInvoiceRequest) returns (CancelCustomerInvoiceResponse);
rpc ReverseCustomerInvoicePayment(ReverseCustomerInvoicePaymentRequest) returns (ReverseCustomerInvoicePaymentResponse);
rpc GetCustomerInvoiceAttachment(GetCustomerInvoiceAttachmentRequest) returns (GetCustomerInvoiceAttachmentResponse);

message VatAmount { uint32 vat_rate = 1; int64 amount = 2; }

message CustomerInvoice {
  uint32 number = 1;
  uint32 customer_number = 2;
  string customer_name = 3;
  string invoice_number = 4;
  string invoice_date = 5;
  string due_date = 6;
  string reference = 7;
  repeated InvoiceLine lines = 8;
  repeated VatAmount vat = 9;
  int64 total = 10;
  string status = 11;                 // "unpaid", "paid" or "cancelled"
  string paid_date = 12;
  repeated VoucherRef vouchers = 13;
  repeated doris.ledger.v1.Attachment attachments = 14;
}

message ListCustomerInvoicesRequest { string company_id = 1; }
message ListCustomerInvoicesResponse {
  repeated CustomerInvoice invoices = 1;  // newest first
  bool cash_method = 2;
  string next_invoice_number = 3;
}
message RegisterCustomerInvoiceRequest {
  string company_id = 1;
  uint32 customer_number = 2;
  string invoice_number = 3;
  string invoice_date = 4;
  string due_date = 5;
  string reference = 6;
  repeated InvoiceLine lines = 7;
  repeated doris.ledger.v1.NewAttachment attachments = 8;
}
message RegisterCustomerInvoiceResponse { uint32 number = 1; }
message PayCustomerInvoiceRequest { string company_id = 1; uint32 number = 2; string date = 3; uint32 account = 4; }
message PayCustomerInvoiceResponse {}
message CancelCustomerInvoiceRequest { string company_id = 1; uint32 number = 2; string reason = 3; }
message CancelCustomerInvoiceResponse {}
message ReverseCustomerInvoicePaymentRequest { string company_id = 1; uint32 number = 2; string reason = 3; }
message ReverseCustomerInvoicePaymentResponse {}
message GetCustomerInvoiceAttachmentRequest { string company_id = 1; uint32 number = 2; string sha256 = 3; }
message GetCustomerInvoiceAttachmentResponse { doris.ledger.v1.Attachment attachment = 1; bytes data = 2; }
```

## Server (`crates/server/src/invoicing.rs`)
Följer RPC:erna för leverantörsfakturor. `InvoicingService` har redan gränserna 21/11 MiB och sessionsspärren.

| Ny kod | Status |
|---|---|
| `customer_invoice_not_found` | not_found |
| `customer_inactive` | failed_precondition |
| `duplicate_customer_invoice` | already_exists |
| `customer_invoice_paid` | failed_precondition |
| `customer_invoice_not_paid` | failed_precondition |
| `customer_invoice_cancelled` | failed_precondition |

Dessa återanvänds: `customer_not_found`, `invalid_invoice_number`, `invalid_due_date`, `invalid_reference`, `invalid_invoice_lines`, `invalid_vat_rate`, `invalid_invoice_account`, `invalid_payment_account`, `invalid_reason`, `voucher_date_in_future`, `invalid_date` och ledgers koder.

## Frontend (`crates/web`)
- **Gemensamma komponenter** för list- och formulärsidorna, i en ny modul `src/invoice_ui.rs`:
  - fakturaraderna med förhandsvisning av moms (`preview_vat` flyttas hit)
  - statusetiketten
  - panelerna för betalning och anledning
  - underlagsknapparna och visningen av underlag

  Leverantörssidorna går över till dem, och deras e2e-tester ska gå igenom oförändrade.
- **`/customer-invoices`, Kundfakturor:**
  - Tabell med Fakturanr, Kund, Fakturadatum, Förfaller, Belopp och Status (Obetald, Förfallen, Betald eller Makulerad).
  - Kryssrutan "Visa betalda och makulerade".
  - Detaljer visar rader, moms per sats, att betala, verifikationer och underlag.
  - Knapparna *Registrera inbetalning* (betaldatum i dag, konto 1930), *Makulera* och *Ångra betalning*.
- **`/customer-invoices/new`, Ny kundfaktura:**
  - Kund (aktiva kunder) och fakturanummer, förifyllt med `next_invoice_number`.
  - Fakturadatum, och förfallodatum: fakturadatum + kundens betalningsvillkor, som följer fakturadatum och vald kund.
  - OCR/meddelande, rader och underlag.
  - Netto, moms per sats och att betala visas medan man skriver.
  - Knappen heter "Registrera". Formuläret töms vid byte av företag.
- **Räkenskapsår:** varningen visas när ett företag med kontantmetoden har obetalda kund- eller leverantörsfakturor. Texten blir "Det finns obetalda kund- eller leverantörsfakturor. Med kontantmetoden ska de bokföras vid räkenskapsårets slut (BFL 5 kap. 2 §). Doris gör inte det än."
- Länken "Kundfakturor" läggs till i bokföringsraden.
- `errors.rs` får de nya koderna. Meddelandet för `invalid_invoice_account` blir "Raderna kan inte bokföras på reskontrakontot (1510/2440) eller ett momskonto."

## Tester (TDD)
- **Domän:**
  - `by_rate` och momsposterna på 2611/2621/2631.
  - Verifikationsraderna för båda metoderna och för en faktura utan moms.
  - Att ett nummer inte återanvänds efter makulering.
  - `next_invoice_number` med tomt register, med ledande nollor och med icke-numeriska nummer.
  - Spärren mot 1510 och 26xx för kund, och mot 2440 och 26xx för leverantör.
  - Övergångarna och felen.
- **Utbrytningen:** alla tester för leverantörsfakturor (domän, lagring, server och e2e) går igenom oförändrade. `stress.rs` går igenom.
- **Lagring:**
  - Med faktureringsmetoden: registrering och inbetalning ger 1510 = 0 och rätt utgående moms.
  - Med kontantmetoden: ingen verifikation förrän inbetalningen, och sedan med underlaget.
  - Makulering och ångrad betalning, även när verifikationen rättats för hand.
  - En avvisad registrering lämnar inget efter sig och förbrukar inga nummer.
  - Samtidiga registreringar av samma fakturanummer ger exakt en faktura.
  - Stängt år.
  - Ombyggnad från `read_all`.
  - Underlag bara via den egna fakturan.
  - En icke-medlem nekas.
  - `list_customer_invoices` ger rätt nästa nummer.
- **Server:** gRPC-Web för varje RPC och ny kod.
- **E2E** (`e2e/tests/customer_invoices.spec.ts`):
  - Med faktureringsmetoden: registrera med föreslaget nummer och underlag → verifikationen i grundboken → inbetalning → ångra → inbetalning.
  - Makulera och att numret inte går att återanvända.
  - Med kontantmetoden, inklusive varningen på Räkenskapsår.
  - Att formuläret töms vid byte av företag.

## Dokumentation
AGENTS.md uppdateras:
- Kundfakturor: ström, projektion, unikt nummer även efter makulering, kontona 1510/2611/2621/2631 och felkoderna.
- Den gemensamma koden i `invoices.rs`.
