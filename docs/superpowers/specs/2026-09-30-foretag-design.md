# Doris – Steg 2: Företag (flera företag, hämtning från Bolagsverket)

## Kontext
En användare ska kunna lägga till de företag hen sköter bokföringen åt och hantera flera företag. Uppgifterna ska kunna hämtas med organisationsnumret. allabolag.se har bara ett betalt API (UC Affärsinformation) och tillåter inte skrapning. Därför hämtar vi från samma källa som allabolag: Bolagsverkets kostnadsfria API för **värdefulla datamängder**. Det kräver inget avtal, bara en OAuth2-klient (client credentials) som registreras i Bolagsverkets utvecklarportal. Gränsen är 60 anrop per minut.

Registeruppgifterna räcker inte för att bokföra enligt BFL. Två uppgifter behövs innan den första verifikationen kan bokföras, och båda ingår i det här steget:
- **Räkenskapsår** (3 kap.): det väljs vid registreringen, och Bolagsverkets data innehåller det inte på ett tillförlitligt sätt.
- **Bokföringsmetod** (5 kap. 2 §): kontantmetoden eller faktureringsmetoden.

Följande hör till senare steg: kontoplan och systembeskrivning (5 kap. 11 §), verifikationsserier (5 kap. 6–7 §), ingående balanser, moms, och bokslutsform (6 kap., som härleds ur juridisk form och storlek).

### Fattade beslut
| Område | Beslut |
|---|---|
| Datakälla | Bolagsverket, värdefulla datamängder. Hämtningen är valfri: utan konfigurerade inloggningsuppgifter fylls uppgifterna i för hand. Inget sparas förrän användaren klickar på Spara. |
| Åtkomst | Den som skapar ett företag blir medlem i det. En medlem kan lägga till andra befintliga användare (via e-postadress). Användaren ser bara de företag hen är medlem i. |
| Omfattning | Registeruppgifter, räkenskapsår och bokföringsmetod. |
| Enskild firma | Tillåten. Organisationsnumret är ägarens personnummer och därmed en personuppgift, så det loggas aldrig (det gäller alla organisationsnummer, för enkelhetens skull). |
| Ändringar | Namnbyte, adressändring och borttagning av medlemmar kommer i ett senare steg. |

## Arkitektur
En ny crate, `crates/company` (`doris-company`), med samma form som `doris-identity`:

```
crates/company/src/
  domain.rs       rena decide/evolve, värdeobjekt, DomainError
  projections.rs  apply(conn, &RecordedEvent), rebuild_projections
  queries.rs      list_companies(user), get_company(user, id), list_members
  lib.rs          ladda → decide → append → projicera i en transaktion (eventstore::begin)
migrations/0005_companies.sql
proto/doris/company/v1/company.proto
crates/server/src/bolagsverket.rs   HTTP-klient mot Bolagsverket
```

`doris-company` beror inte på `doris-identity`. Användare refereras med sitt användar-id (UUID som sträng). Servern slår upp e-post → användar-id med `identity::find_user_by_email` innan `AddMember` anropas.

## Domän

### Ström och händelser
Strömmen heter `company-{uuid}` och `SCHEMA_VERSION = 1`. Händelserna är en serde-enum med `#[serde(tag = "type")]`:

- `CompanyRegistered { org_nr, name, legal_form, address, first_fiscal_year_start, first_fiscal_year_end, accounting_method, created_by }`
- `MemberAdded { user_id }`

`CompanyRegistered` följs i samma append av `MemberAdded { user_id: created_by }`, så att medlemskap bara har en källa.

### Värdeobjekt
- **`OrgNr`**: 10 siffror. Bindestreck och mellanslag tas bort vid inläsning, och ett 12-siffrigt nummer med sekelsiffror (`16`/`19`/`20`) kortas till 10. Kontrollsiffran valideras med Luhn. Numret lagras utan bindestreck och visas som `NNNNNN-NNNN`.
- **`CompanyName`**: trimmas och får vara 1–200 tecken.
- **`LegalForm`**: `Aktiebolag`, `Handelsbolag`, `Kommanditbolag`, `EnskildFirma`, `EkonomiskForening`, `IdeellForening`, `Stiftelse` eller `Other`. Bolagsverkets kod mappas till en av dem.
- **`Address`**: `{ street, postal_code, city }`, där alla fält är valfria.
- **`AccountingMethod`**: `Cash` (kontantmetoden) eller `Invoice` (faktureringsmetoden). Gränsen på 3 MSEK i nettoomsättning kan Doris inte kontrollera. Gränssnittet visar den som en upplysning.
- **`FiscalYear`**: `{ start, end }` som `chrono::NaiveDate`. Om chrono inte redan finns i beroendeträdet använder vi `time`, beroende på vad som redan finns.

### Regler för räkenskapsåret (3 kap. BFL)
- `start` är den första dagen i en månad och `end` den sista dagen i en månad.
- Det första räkenskapsåret är 1–18 månader långt (3 kap. 3 §).
- Följande räkenskapsår är 12 månader och slutar i samma månad som det första. Det lagras inte, utan beräknas.
- `EnskildFirma` och `Handelsbolag` (samt `Kommanditbolag`) kräver att räkenskapsåret är kalenderår, det vill säga att `end` ligger den 31 december (3 kap. 1 §). Undantaget för handelsbolag med juridiska personer som delägare stöds inte i det här steget.
- Om någon regel bryts blir felet `DomainError::InvalidFiscalYear`.

### Beslut (decide)
- `register_company(cmd, created_by) -> Result<Vec<CompanyEvent>>`
- `add_member(state, user_id, actor) -> Result<Vec<CompanyEvent>>`:
  - Om `actor` inte är medlem blir felet `NotFound`, så att det inte avslöjas att företaget finns.
  - Om `user_id` redan är medlem blir resultatet inga händelser. Anropet är idempotent.

## Projektioner (`0005_companies.sql`)
```sql
CREATE TABLE companies (
  id TEXT PRIMARY KEY, org_nr TEXT NOT NULL UNIQUE, name TEXT NOT NULL,
  legal_form TEXT NOT NULL, street TEXT, postal_code TEXT, city TEXT,
  first_fiscal_year_start TEXT NOT NULL, first_fiscal_year_end TEXT NOT NULL,
  accounting_method TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE company_members (
  company_id TEXT NOT NULL REFERENCES companies(id), user_id TEXT NOT NULL,
  added_at TEXT NOT NULL, PRIMARY KEY (company_id, user_id)
);
```
- `UNIQUE(org_nr)` gäller i hela systemet. Om ett org.nr redan finns avbryter projektionen transaktionen, och felet blir `already_exists`. Den som får felet får be en befintlig medlem om åtkomst.
- `rebuild_projections` tömmer tabellerna och spelar upp alla händelser från `read_all`. Ett test verifierar att resultatet blir identiskt.

## API: `doris.company.v1.CompanyService`
Varje anrop kräver en session och använder samma mönster som `AuthApi::user()`.

| RPC | In | Ut |
|---|---|---|
| `LookupCompany` | `org_nr` | namn, juridisk form, adress (förslag, sparas inte) |
| `CreateCompany` | `org_nr`, namn, juridisk form, adress, räkenskapsår, metod | `company_id` |
| `ListCompanies` | – | företagen där användaren är medlem |
| `GetCompany` | `company_id` | uppgifter plus beräknat pågående räkenskapsår |
| `AddMember` | `company_id`, `email` | – |
| `ListMembers` | `company_id` | visningsnamn och e-post |

Nya felkoder (varje kod läggs också till i `crates/web/src/errors.rs`):

| Kod | gRPC-status | Betydelse |
|---|---|---|
| `invalid_org_nr` | `invalid_argument` | orgnr har fel format eller fel kontrollsiffra |
| `invalid_company_name` | `invalid_argument` | tomt namn eller mer än 200 tecken |
| `invalid_address` | `invalid_argument` | ett adressfält är längre än 200 tecken |
| `invalid_legal_form` | `invalid_argument` | bolagsform saknas |
| `invalid_accounting_method` | `invalid_argument` | redovisningsmetod saknas |
| `invalid_fiscal_year` | `invalid_argument` | räkenskapsåret bryter mot 3 kap. |
| `company_exists` | `already_exists` | orgnr finns redan |
| `company_not_found` | `not_found` | företaget saknas eller användaren är inte medlem |
| `user_not_found` | `not_found` | ingen användare med den e-postadressen |
| `lookup_unavailable` | `failed_precondition` | Bolagsverket är inte konfigurerat |
| `lookup_personal_number` | `failed_precondition` | orgnr är ett personnummer (enskild firma) |
| `lookup_not_found` | `not_found` | Bolagsverket känner inte till orgnr |
| `lookup_failed` | `unavailable` | nätverksfel eller felsvar från Bolagsverket (loggas utan orgnr) |

## Bolagsverket-klienten
- Ett nytt beroende i `doris-server`: `reqwest` med `native-tls` (samma system-OpenSSL som webauthn-rs redan länkar), `json` och `form`, utan standardfeatures. Det finns ingen HTTP-klient i servern i dag. Klienten används bara av servern och påverkar inte wasm-storleken.
- Konfigurationen sker med miljövariabler eller CLI-flaggor: `DORIS_BOLAGSVERKET_CLIENT_ID`, `DORIS_BOLAGSVERKET_CLIENT_SECRET` och `DORIS_BOLAGSVERKET_TOKEN_URL` och `DORIS_BOLAGSVERKET_API_URL` (med produktion som standard). Utan id och secret svarar `LookupCompany` med `lookup_unavailable`.
- Ett organisationsnummer som är ett personnummer skickas aldrig till Bolagsverket (`lookup_personal_number`).
- En OAuth2-token hämtas med client credentials och cachas i minnet tills 60 sekunder före `expires_in`.
- Den exakta endpointen, scope och JSON-formen verifieras mot `api.bolagsverket.se` när planen skrivs. Mappningen till `LegalForm` och `Address` hålls i en ren funktion som testas med ett sparat exempelsvar.
- Testerna startar en falsk Bolagsverket-server (axum) och pekar token- och API-URL:erna på den.

## Frontend
- `/companies` visar en lista med dina företag (namn, orgnr) och knappen **Lägg till företag**. `/` länkar dit.
- `/companies/new` har följande:
  - ett fält för organisationsnummer och knappen **Hämta**, som fyller i namn, juridisk form och adress. Om hämtningen misslyckas visas felet, och fälten går ändå att fylla i för hand.
  - fälten för räkenskapsårets början och slut (`<input type="date">`). Standard är innevarande kalenderår.
  - valet av bokföringsmetod (radioknappar) med en upplysning om gränsen på 3 MSEK.
  - knappen **Spara**. Därefter kommer man till `/companies/{id}`.
- `/companies/{id}` visar uppgifterna, det pågående räkenskapsåret och en lista över medlemmar. Där finns ett formulär för att lägga till en medlem med e-postadress.
- Sidorna återanvänder `Field`, `Button`, `Card` och `ErrorAlert` från `ui.rs`. Radioknappar och select genereras från shadcn-presetet vid behov. `api.rs` får en `company_api()` bredvid `api()`.

## Tester
- **Domän** (given/when/then, utan databas):
  - `OrgNr`: Luhn, format och 12-siffriga nummer
  - räkenskapsårets regler: månadsgränser, 18 månader, kalenderår för EF/HB/KB
  - `add_member`: medlem, icke-medlem och idempotens
- **Crate** (SQLite i minnet):
  - skapa och lista
  - unikt orgnr
  - åtkomst per användare
  - ombyggnad av projektioner
- **Server** (gRPC-Web, som i `crates/server/tests`):
  - alla RPC:er
  - felkoderna
  - `LookupCompany` mot en falsk Bolagsverket-server, med och utan konfiguration
- **E2E** (Playwright):
  - lägga till ett företag för hand, se det i listan och i detaljvyn
  - en andra användare ser inte företaget förrän hen har lagts till som medlem

## Utanför omfattningen
Ändring av företagsuppgifter, borttagning av medlemmar, moms, kontoplan, verifikationsserier, ingående balanser, bokslutsform, samt undantaget för handelsbolag med juridiska personer som delägare.
