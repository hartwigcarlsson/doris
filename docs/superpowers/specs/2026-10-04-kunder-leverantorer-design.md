# Doris – Steg 9: Kunder och leverantörer

## Kontext
Doris ska få kundfakturor och leverantörsfakturor. Arbetet delas upp i delprojekt med var sin spec:

1. **Kunder och leverantörer** (det här steget): ett register för vardera, per företag.
2. **Leverantörsfakturor**: registrera med underlag, bokföra och betala.
3. **Kundfakturor**: registrera, bokföra och ta emot betalning. Samma modell som 2.
4. Senare: fakturautskrift och PDF, påminnelser och reskontrarapporter.

Fakturorna i 2–3 registreras i Doris, men Doris skapar dem inte. Att skapa och skriva ut kundfakturor är delprojekt 4.

Det här steget bygger bara registren. Fakturor, bokföring och betalningar ingår inte.

### Fattade beslut
| Område | Beslut |
|---|---|
| Register | Två separata register, ett för kunder och ett för leverantörer, med var sina fält och sidor. |
| Plats | Ny crate `crates/invoicing` (`doris-invoicing`) och en ny `InvoicingService`. Fakturorna i steg 2–3 hamnar i samma crate. Beroenden går invoicing → company/ledger, aldrig tvärtom. |
| Nummer | Kundnummer och leverantörsnummer löper 1, 2, 3 … per företag och register. Servern sätter numret i skrivtransaktionen, aldrig klienten. |
| Varaktighet | En post tas aldrig bort. Den kan inaktiveras och återaktiveras, precis som ett konto. Fakturor kommer att peka på numret. |
| Ändringar | Alla fält utom numret går att ändra. En faktura (steg 2–3) sparar sin egen kopia av motpartens uppgifter, så gamla fakturor påverkas inte. |
| Dubbletter | Samma org.nr får förekomma flera gånger, eftersom en motpart kan ha flera adresser eller avdelningar. |
| Inaktiva | En inaktiv post syns i listan och går att ändra. Att den inte kan väljas på nya fakturor är en regel för steg 2–3. |
| Åtkomst | Alla medlemmar i företaget får läsa och ändra. En icke-medlem får `company_not_found`, som i ledger. |
| Momsreg.nr | Bara formatkontroll. Ingen VIES-kontroll mot EU, eftersom Doris inte har några externa tjänster utöver Bolagsverket. |
| Personuppgifter | Org.nr kan vara ett personnummer (privatperson eller enskild firma), och e-post är en personuppgift. Ingen av dem loggas. |

## Domän (`crates/invoicing/src/domain.rs`, ren och utan I/O)

### Värdeobjekt
Dessa delas av båda registren:

- `PartyName::parse(raw)`: trimmas, 1–200 tecken, annars `InvalidName`.
- `OrgNr` och `Address`: återanvänds från `doris_company::domain`. Felen `InvalidOrgNr` och `InvalidAddress` mappas om till invoicings `DomainError`. Org.nr är valfritt (`Option<OrgNr>`), så att utländska kunder fungerar. Personnummer accepteras.
- `Email::parse(raw)`: valfri. Samma regel som `doris_identity::domain::Email`: trimmas och görs gemen, högst 254 tecken, exakt ett `@`, något före det, en domän med punkt (inte först eller sist) och inga blanksteg, annars `InvalidEmail`. Regeln kopieras, eftersom invoicing inte ska bero på identity och dess webauthn-beroenden. `Debug` är maskad.
- `VatNumber::parse(raw)`: valfri. Blanksteg tas bort och bokstäverna görs versala. Numret ska vara två bokstäver följda av 2–12 tecken A–Z/0–9. Med prefixet `SE` måste det vara `SE` + tio siffror som klarar `OrgNr::parse` + `01`. Annars blir det `InvalidVatNumber`.

Bara för kunder:
- `PaymentTerms::new(days)`: 0–365 dagar, annars `InvalidPaymentTerms`. Formuläret föreslår 30.

Bara för leverantörer:
- `Bankgiro::parse(raw)`: valfri. Bindestreck och blanksteg tas bort, och resten ska vara 7–8 siffror som klarar Luhn-kontrollen. Annars blir det `InvalidBankgiro`. `formatted()` ger `NNN-NNNN` eller `NNNN-NNNN`.
- `Plusgiro::parse(raw)`: valfri. Bindestreck och blanksteg tas bort, och resten ska vara 2–8 siffror som klarar Luhn-kontrollen. Annars blir det `InvalidPlusgiro`. `formatted()` sätter bindestreck före sista siffran.
- `Iban::parse(raw)`: valfri. Blanksteg tas bort och bokstäverna görs versala. Ska vara 15–34 tecken: två bokstäver, två siffror och sedan A–Z/0–9, och mod-97 av det omflyttade talet ska vara 1. Annars blir det `InvalidIban`. `formatted()` grupperar fyra tecken i taget.
- `Bic::parse(raw)`: valfri. Blanksteg tas bort och bokstäverna görs versala. Ska vara 8 eller 11 tecken: 4 bokstäver, 2 bokstäver (land) och 2 tecken A–Z/0–9, plus valfritt 3 tecken A–Z/0–9. Annars blir det `InvalidBic`.

Luhn-funktionen i `doris_company` är privat. Den görs `pub` och återanvänds i stället för att kopieras.

### Detaljer
```rust
pub struct CustomerDetails {
    pub name: PartyName,
    pub org_nr: Option<OrgNr>,
    pub vat_number: Option<VatNumber>,
    pub address: Address,
    pub email: Option<Email>,
    pub payment_terms: PaymentTerms,
}

pub struct SupplierDetails {
    pub name: PartyName,
    pub org_nr: Option<OrgNr>,
    pub vat_number: Option<VatNumber>,
    pub address: Address,
    pub email: Option<Email>,
    pub bankgiro: Option<Bankgiro>,
    pub plusgiro: Option<Plusgiro>,
    pub iban: Option<Iban>,
    pub bic: Option<Bic>,
}
```
Ett tomt fält i formuläret blir `None`.

### Event (`schema_version` 1)
```rust
pub enum CustomerEvent {
    CustomerAdded { number: u32, details: CustomerDetails },
    /// The full new set of details, not a diff.
    CustomerUpdated { number: u32, details: CustomerDetails },
    CustomerDeactivated { number: u32 },
    CustomerReactivated { number: u32 },
}
```
`SupplierEvent` har samma form: `SupplierAdded`, `SupplierUpdated`, `SupplierDeactivated` och `SupplierReactivated`.

### Tillstånd och beslut
`Customers { parties: BTreeMap<u32, Customer> }`, där `Customer { number, details, active }`. `Suppliers` ser likadant ut.

- `add_customer(state, details)` ger `CustomerAdded` med `number = max(number) + 1`, eller 1 i ett tomt register.
- `update_customer(state, number, details)` ger `CustomerUpdated`. Samma uppgifter som förut ger inga event (no-op), som när man byter till samma kontonamn. Ett okänt nummer ger `CustomerNotFound`.
- `set_customer_active(state, number, active)` ger `CustomerDeactivated` eller `CustomerReactivated`. Om posten redan har det läget blir det inga event (no-op). Ett okänt nummer ger `CustomerNotFound`.

Leverantörer fungerar likadant och får `SupplierNotFound` för okända nummer.

## Lagring och transaktioner

### Strömmar
`customers-{company_id}` och `suppliers-{company_id}`: en ström per företag och register, som kontoplanens `accounts-{company_id}`. Varje skrivning sker i en `BEGIN IMMEDIATE`-transaktion i den här ordningen: medlemskontroll, ladda strömmen, `decide`, `append` med förväntad version och uppdatering av projektionen. Numret bestäms alltså inne i transaktionen.

### Migration `migrations/0009_invoicing.sql`
```sql
CREATE TABLE customers (
    company_id TEXT    NOT NULL REFERENCES companies(company_id),
    number     INTEGER NOT NULL,
    details    TEXT    NOT NULL,  -- the event's details as JSON
    active     INTEGER NOT NULL,
    PRIMARY KEY (company_id, number)
);
-- suppliers: same columns.
```
Uppgifterna lagras som JSON, i samma form som i eventet, i stället för en kolumn per fält. Det ger en enda kod för båda tabellerna, och SQLites JSON-funktioner räcker om en fråga senare behöver ett enskilt fält.

Båda projektionerna går att bygga om från `read_all` (`rebuild_projections` i crate:n), och ett test kontrollerar att ombyggnaden ger samma rader.

### Publik API (`crates/invoicing/src/lib.rs`)
```rust
pub async fn list_customers(pool, company_id, actor) -> Result<Vec<Customer>>
pub async fn add_customer(pool, company_id, details, actor) -> Result<u32>
pub async fn update_customer(pool, company_id, number, details, actor) -> Result<()>
pub async fn set_customer_active(pool, company_id, number, active, actor) -> Result<()>
// samma fyra för suppliers
```
Listorna innehåller alla poster, även inaktiva, sorterade på nummer.
`// ponytail: no pagination; add it when a company has thousands of parties.`

## API (`proto/doris/invoicing/v1/invoicing.proto`, package `doris.invoicing.v1`)
```proto
service InvoicingService {
  rpc ListCustomers(ListCustomersRequest) returns (ListCustomersResponse);
  rpc AddCustomer(AddCustomerRequest) returns (AddCustomerResponse);
  rpc UpdateCustomer(UpdateCustomerRequest) returns (UpdateCustomerResponse);
  rpc SetCustomerActive(SetCustomerActiveRequest) returns (SetCustomerActiveResponse);
  rpc ListSuppliers(ListSuppliersRequest) returns (ListSuppliersResponse);
  rpc AddSupplier(AddSupplierRequest) returns (AddSupplierResponse);
  rpc UpdateSupplier(UpdateSupplierRequest) returns (UpdateSupplierResponse);
  rpc SetSupplierActive(SetSupplierActiveRequest) returns (SetSupplierActiveResponse);
}

message CustomerDetails {
  string name = 1;
  string org_nr = 2;       // "" = none
  string vat_number = 3;
  string street = 4;
  string postal_code = 5;
  string city = 6;
  string email = 7;
  uint32 payment_terms = 8;
}
message Customer { uint32 number = 1; CustomerDetails details = 2; bool active = 3; }

message SupplierDetails {
  string name = 1;
  string org_nr = 2;
  string vat_number = 3;
  string street = 4;
  string postal_code = 5;
  string city = 6;
  string email = 7;
  string bankgiro = 8;
  string plusgiro = 9;
  string iban = 10;
  string bic = 11;
}
message Supplier { uint32 number = 1; SupplierDetails details = 2; bool active = 3; }
```
Varje request har `string company_id`. Add-, Update- och SetActive-anropen har dessutom `details` respektive `number` och `active`. `AddCustomerResponse` och `AddSupplierResponse` returnerar `number`. Värdena skickas tillbaka i formaterad form, till exempel `NNNNNN-NNNN` och bankgiro med bindestreck.

`doris-proto` genererar klienten och, med `server`-featuren, servern, som för de andra tjänsterna. Tjänsten behåller tonics gräns på 4 MiB.

## Server (`crates/server/src/invoicing.rs`)
Följer mönstret i `company.rs` och `ledger.rs`: sessionen kontrolleras i varje handler, och felen mappas i `status` och `domain_status`. Tjänsten registreras i routern bakom `GrpcWebLayer`.

| Kod | Status |
|---|---|
| `not_signed_in` | unauthenticated (befintlig) |
| `company_not_found` | not_found (befintlig, även för icke-medlem) |
| `invalid_name` | invalid_argument |
| `invalid_org_nr` | invalid_argument (befintlig) |
| `invalid_address` | invalid_argument (befintlig) |
| `invalid_email` | invalid_argument (befintlig i identity) |
| `invalid_vat_number` | invalid_argument |
| `invalid_payment_terms` | invalid_argument |
| `invalid_bankgiro` | invalid_argument |
| `invalid_plusgiro` | invalid_argument |
| `invalid_iban` | invalid_argument |
| `invalid_bic` | invalid_argument |
| `customer_not_found` | not_found |
| `supplier_not_found` | not_found |

Ett ogiltigt `company_id` (ingen UUID) ger `company_not_found`, som i ledger.

## Frontend (`crates/web`)
- `src/api.rs`: `invoicing_api()`, med cookies som de andra klienterna.
- `src/pages/customers.rs` på `/customers` och `src/pages/suppliers.rs` på `/suppliers`, med `accounts.rs` som förebild:
  - En tabell med nummer, namn, org.nr, ort (för leverantörer: bankgiro) och status ("Aktiv"/"Inaktiv"). Raderna har knapparna "Redigera" och "Inaktivera"/"Aktivera".
  - Knappen "Ny kund" respektive "Ny leverantör" öppnar formuläret. Samma formulär används för att redigera och är då förifyllt.
  - Formuläret har `novalidate`. Felet visas på svenska överst på sidan via `errors.rs` (`ErrorAlert`), som på övriga sidor.
  - Kundformuläret: Namn, Org.nr/personnr, Momsreg.nr, Gatuadress, Postnummer, Ort, E-post och Betalningsvillkor (dagar), med 30 förifyllt.
  - Leverantörsformuläret: Namn, Org.nr, Momsreg.nr, Gatuadress, Postnummer, Ort, E-post, Bankgiro, Plusgiro, IBAN och BIC.
- Sidorna arbetar mot det aktiva företaget från `Companies`-kontexten och skickar dess `company_id` med varje anrop.
- Headerns bokföringsrad får länkarna "Kunder" och "Leverantörer".
- `src/errors.rs` får en rad per ny kod.
- `make dist` måste hålla wasm-budgeten (500 KB gzip).

## Tester (TDD: röd → grön → refaktor, en commit per cykel)
- **Domän** (`domain.rs`, utan databas, given/when/then):
  - Numreringen går 1..n i varje register.
  - Uppdatering ersätter alla detaljer.
  - Inaktivering och återaktivering, inklusive no-op.
  - Okänt nummer ger `CustomerNotFound` respektive `SupplierNotFound`.
  - Varje värdeobjekt testas med giltiga och ogiltiga fall: Luhn för bankgiro och plusgiro, mod-97 för IBAN, SE-momsreg.nr med och utan `01`, BIC med 8 och 11 tecken, e-post och betalningsvillkor 0, 365 och 366.
- **Crate** (`crates/invoicing/tests/`):
  - Kunder och leverantörer kan skapas, ändras och inaktiveras mot en riktig databas.
  - En icke-medlem nekas.
  - Projektionerna byggs om från `read_all` och ger samma rader.
  - Två företag får var sin nummerserie.
- **Server**: gRPC-Web-test (`GrpcWebClientLayer`) för varje RPC, med minst en felkod per RPC, och kontroll att ett anrop utan session ger `not_signed_in`.
- **E2E** (`e2e/`): skapa en kund, se den i listan, redigera den och inaktivera den. Samma för en leverantör. Elementen väljs via svenska etiketter.

## Dokumentation
`AGENTS.md` uppdateras: `crates/invoicing` i Layout, `invoicing.proto` och felkodsmappningen i `crates/server/src/invoicing.rs` under API.

## Utanför det här steget
Fakturor, bokföring, betalningar, sökning och paginering, import från Bolagsverket för motparter och VIES-kontroll.
