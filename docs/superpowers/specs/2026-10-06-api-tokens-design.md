# Doris – API-tokens

## Kontext
Doris går i dag bara att använda via webbläsaren: inloggningen är en
passkey, och sessionen ligger i cookien `doris_session`. Nästa steg är en
CLI (`doris-cli`) och på sikt en AI-agent som sköter bokföringen åt
användaren. De kan inte göra en WebAuthn-ceremoni. I det här steget kan en
användare därför skapa en API-token som autentiserar mot samma gRPC-Web-API
och styra per bolag vad token får göra.

### Fattade beslut
| Område | Beslut |
|---|---|
| Vem har behörigheter | Bara tokens. Medlemmar har fortsatt full åtkomst till sina bolag. En token är en delmängd av ägarens åtkomst och kan aldrig ge mer. |
| Behörigheter | Område × läs/skriv: `ledger:read`, `ledger:write`, `invoicing:read`, `invoicing:write`, `payroll:read`, `payroll:write`, `vat:read`, `vat:write`, `company:read`. `:write` innefattar inte `:read`; UI:t kryssar i läs när skriv väljs. |
| Per bolag | En token har grants: en lista med bolag och vilka behörigheter den har i vart och ett. |
| Livslängd | Utgångstid krävs, vald när token skapas: 30 dagar, 90 dagar, 1 år eller ett eget datum, högst 366 dagar fram. Token kan återkallas när som helst. |
| Ändring | En token ändras aldrig. Den som vill ändra behörigheterna återkallar token och skapar en ny. |
| Lagring | Events i identity (`ApiTokenCreated`, `ApiTokenRevoked`) och projektionen `api_tokens`. Klartexten visas en gång; bara SHA-256 sparas. |
| Kontroll | Varje handler anger sin behörighet (`Scope`) när den hämtar anroparen. Det går inte att kompilera en handler utan behörighet. |
| Historik | `Metadata` får `via_token`. `actor` är fortfarande ägaren. |

Utanför det här steget: själva `doris-cli` (egen spec), behörigheter för
medlemmar och finare behörigheter för till exempel stängning av år eller
inlämning av deklarationer. De sista kan läggas till senare som nya
`Scope`-värden utan att befintliga tokens ändrar betydelse.

## Token
- Formatet är `doris_` följt av `token::new_token()` (256 bitar,
  base64url). Prefixet gör att token känns igen av secret scanners och
  syns som en Doris-token i en konfigurationsfil.
- Token skickas som HTTP-headern `authorization: Bearer doris_…` (gRPC
  metadata). Hela strängen inklusive prefix hashas med
  `token::hash_token`.
- Token loggas aldrig, varken i klartext eller som hash.

## Domän (`doris-identity`)
Tokens ligger i identitys stream för ägaren. Händelser:

```
ApiTokenCreated { schema_version: 1, token_id, user_id, name, token_hash,
                  created_at, expires_at,
                  grants: [{ company_id, scopes: ["ledger:read", …] }] }
ApiTokenRevoked { schema_version: 1, token_id, revoked_by, revoked_at }
```

`decide`-regler, testade given/when/then utan databas:
- `name` trimmas och är 1–100 tecken (`invalid_token_name`).
- `expires_at` ligger efter `now` och högst 366 dagar fram
  (`invalid_token_expiry`).
- `grants` innehåller minst ett bolag, inget bolag mer än en gång och minst
  en behörighet per bolag. Okända behörigheter avvisas
  (`invalid_token_grants`).
- Bara ägaren eller en admin får återkalla en token. För någon annan ser
  token ut som om den inte finns (`api_token_not_found`). En token som redan är återkallad ger inga events, så
  återkallandet är idempotent.

Att ägaren är medlem i varje bolag i grants kontrolleras i servern via
`doris_company::get_company`, eftersom identity inte läser companys
tabeller. Medlemskapet kontrolleras dessutom på nytt vid varje anrop, se
nedan.

`Scope` är en enum i identity med `as_str`/`parse` mot strängarna ovan.
Grants sparas som strängar i eventet, så att en ny `Scope` senare inte
ändrar gamla events.

## Lagring
Migration `0015_api_tokens.sql`:

```sql
-- Projection of ApiTokenCreated/ApiTokenRevoked. Rebuildable.
CREATE TABLE api_tokens (
    token_id    TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    name        TEXT NOT NULL,
    token_hash  TEXT NOT NULL UNIQUE,
    grants      TEXT NOT NULL,      -- JSON, as in the event
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    revoked_at  INTEGER
);
CREATE INDEX api_tokens_user ON api_tokens (user_id);

-- Operational: when a token was last used. Safe to purge.
CREATE TABLE api_token_usage (
    token_id     TEXT PRIMARY KEY,
    last_used_at INTEGER NOT NULL
);
```

- Projektionen uppdateras i samma transaktion som eventet, och ett test
  bygger om den från `read_all`.
- `last_used_at` skrivs efter att anropet har autentiserats, bara om det
  gamla värdet är äldre än en timme. Så tar en läsning skrivlåset högst
  en gång i timmen per token. Ett misslyckat skrivförsök loggas och
  stoppar inte anropet.

Funktioner i identity:
- `create_api_token(pool, user, name, expires_at, grants, now) -> (ApiToken, String)`
  returnerar klartexten en gång.
- `list_api_tokens(pool, user_id) -> Vec<ApiToken>`, med `last_used_at`.
- `revoke_api_token(pool, actor, token_id, now)`.
- `token_user(pool, token, now) -> Option<(User, TokenAccess)>`: `None` om
  token är okänd, utgången eller återkallad. `TokenAccess` innehåller
  `token_id` och grants.

## Autentisering och behörighet (`doris-server`)
`grpc::signed_in_user` ersätts av:

```rust
pub(crate) enum Caller {
    Session(User),
    Token { user: User, token_id: Uuid, grants: Grants },
}

pub(crate) async fn authenticate(pool, headers) -> Result<Caller, Status>
```

- Finns en `authorization: Bearer`-header används den, annars cookien.
  En ogiltig bearer-token ger `not_signed_in` även om en cookie också
  skickas.
- `session_gate` använder `authenticate`, så en token kan skicka underlag
  upp till 20 MiB till `LedgerService` och `InvoicingService`.
- Tjänsternas `caller()` får behörigheten som argument:
  `caller(&request, company_id, Scope::LedgerWrite)`. För
  `Caller::Session` görs samma medlemskontroll som i dag. För
  `Caller::Token` gäller följande:
  1. Ett bolag som inte finns i grants ger samma fel som ett bolag som
     inte finns (`company_not_found`).
  2. En behörighet som saknas för bolaget ger
     `Status::permission_denied("missing_scope")`.
  3. Ägarens medlemskap kontrolleras som för en session. En token slutar
     därmed fungera för ett bolag som ägaren inte längre är medlem i.
- RPC:er som inte gäller ett bolag tar ingen `Scope`. För dem säger
  `Caller::session_only()` att tokens nekas med
  `Status::permission_denied("token_not_allowed")`.

### Behörighet per RPC
| Tjänst | Läs (`:read`) | Skriv (`:write`) |
|---|---|---|
| Ledger | `ListAccounts`, `ListFiscalYears`, `ListVouchers`, `GetAttachment`, `GetTrialBalance`, `GetAccountLedger`, `GetFinancialStatements`, `GetOpeningBalances` | `AddAccount`, `RenameAccount`, `SetAccountActive`, `SetAccountVatBox`, `RecordVoucher`, `CorrectVoucher`, `AddAttachment`, `SetOpeningBalances`, `CloseFiscalYear`, `ReopenFiscalYear` |
| Invoicing | `ListCustomers`, `ListSuppliers`, `ListSupplierInvoices`, `GetSupplierInvoiceAttachment`, `ListCustomerInvoices`, `GetCustomerInvoiceAttachment` | övriga |
| Payroll | `ListEmployees`, `PreviewPayrollRun`, `GetPayrollRun`, `ListPayrollRuns`, `GetAgiContact`, `ListAgiMonths`, `GetAgiMonth`, `ExportAgiFile` | övriga |
| Vat | `ListVatReturns`, `GetVatReturn`, `ExportVatFile` | `SetVatPeriod`, `MarkVatReturnSubmitted` |
| Company | `GetCompany`, `ListMembers` (`company:read`) | – |

- `ListCompanies` med en token visar bara bolagen i grants där ägaren
  fortfarande är medlem, oavsett vilka behörigheter de har.
- `GetStatus` fungerar med en token och svarar med ägaren, så CLI:n kan
  fråga vem den är inloggad som.
- Tokens nekas (`token_not_allowed`) för resten av `AuthService`, inklusive
  RPC:erna för tokens nedan, samt `CreateCompany`, `AddMember`,
  `LookupCompany` och `GetLookupStatus`. En token kan alltså varken skapa
  en ny token, bjuda in någon eller ge sig själv tillgång till fler bolag.
- `ExportAgiFile` innehåller personnummer. Den kräver `payroll:read`, och
  den som ger en agent den behörigheten ger den också personnumren. Det
  står i UI:t vid kryssrutan för lön.

## Behandlingshistorik
`doris_eventstore::Metadata` får `via_token: Option<Uuid>`
(`#[serde(default, skip_serializing_if = "Option::is_none")]`), så att
befintliga events läses som förut. `actor` är fortfarande ägarens
användar-id. Varje skrivande handler skickar med `via_token` från
`Caller`. Därmed syns i eventloggen både vem som gjorde något och att det
gjordes med en viss token. Verifikationslistan ändras inte i det här
steget.

## API
I `proto/doris/auth/v1/auth.proto`:

```proto
rpc CreateApiToken(CreateApiTokenRequest) returns (CreateApiTokenResponse);
rpc ListApiTokens(ListApiTokensRequest) returns (ListApiTokensResponse);
rpc RevokeApiToken(RevokeApiTokenRequest) returns (RevokeApiTokenResponse);

message TokenGrant { string company_id = 1; repeated string scopes = 2; }
message CreateApiTokenRequest {
  string name = 1;
  string expires_at = 2;            // RFC 3339
  repeated TokenGrant grants = 3;
}
message CreateApiTokenResponse { ApiToken token = 1; string secret = 2; }
message ApiToken {
  string id = 1; string name = 2; repeated TokenGrant grants = 3;
  string created_at = 4; string expires_at = 5;
  optional string last_used_at = 6; optional string revoked_at = 7;
}
message ListApiTokensRequest {}
message ListApiTokensResponse { repeated ApiToken tokens = 1; }
message RevokeApiTokenRequest { string token_id = 1; }
message RevokeApiTokenResponse {}
```

- `ListApiTokens` visar den inloggade användarens egna tokens, även
  utgångna och återkallade. Admin återkallar en annan användares token
  med dess id. Det behövs för att kunna stänga en läcka, men det byggs
  inget admin-UI för det i det här steget.
- En okänd token, eller någon annans token för den som inte är admin, ger
  `api_token_not_found`.
- Nya felkoder: `invalid_token_name`, `invalid_token_expiry`,
  `invalid_token_grants`, `api_token_not_found`, `missing_scope` och
  `token_not_allowed`. De mappas i `grpc.rs` och översätts i
  `src/errors.rs`.

## UI
- I kontomenyn läggs "API-tokens" till (`/tokens`), och raden läggs in i
  `section_of`.
- Sidan följer `docs/design/README.md`: `PageHeader` med `LinkButton`
  "Ny token", och en `TableCard` med kolumnerna Namn, Bolag, Skapad,
  Upphör, Senast använd, Status (`Badge`: Aktiv, Utgången, Återkallad)
  och knappen "Återkalla", som frågar en gång till innan den återkallar.
- "Ny token" (`/tokens/new`) är ett fullbrett `Card` med fältet Namn och
  Giltighet (30 dagar, 90 dagar, 1 år, eget datum med `<input type="date">`).
  Under dem finns en ruta med ett bolag per rad och kolumnerna Bokföring,
  Fakturor, Lön, Moms och Bolag, var och en med Läsa och Skriva. Kryssas
  Skriva i kryssas Läsa i också. Vid Lön står att läsbehörigheten omfattar
  AGI-filen med personnummer.
- Efter skapandet visas token en gång i ett `Card` med en kopieringsknapp
  och texten "Token visas bara nu. Spara den på ett säkert ställe." Med
  "Klar" går man tillbaka till listan.
- Sidorna startar sina tasks med `crate::task::spawn_local` och läggs in i
  `e2e/tests/leaving.spec.ts`. De fungerar i ljust och mörkt läge och på
  390 px.

## Tester
- **Domän** (identity, given/when/then): skapa med giltiga värden; tomt
  namn, för långt namn, utgångstid i dåtiden eller mer än 366 dagar fram,
  tomma grants, bolag två gånger, okänd behörighet; återkalla som ägare,
  som admin, som annan användare, och två gånger (inga events).
- **Projektion**: `api_tokens` byggs om från `read_all` och blir lika.
- **Identity**: `token_user` ger `None` för en utgången, återkallad och
  okänd token; `last_used_at` uppdateras högst en gång i timmen.
- **Eventstore**: metadata utan `via_token` läses som förut.
- **Server** (gRPC-Web med bearer, som CLI:n kommer att göra):
  - En token med `ledger:read` listar verifikationer men får
    `missing_scope` på `RecordVoucher`. Med `ledger:write` bokförs
    verifikationen, och eventet har `via_token`.
  - Ett bolag utanför grants ger `company_not_found` och syns inte i
    `ListCompanies`.
  - Utgången och återkallad token ger `not_signed_in`.
  - `CreateApiToken`, `CreateInvitation` och `CreateCompany` med token ger
    `token_not_allowed`. `GetStatus` svarar med ägaren.
  - En bearer-token släpps igenom av `session_gate` med ett underlag på
    nära 20 MiB.
  - `CreateApiToken` med ett bolag användaren inte är medlem i ger
    `company_not_found`.
  - Varje RPC i tabellen ovan anropas med en token som har alla
    behörigheter utom den som krävs och ger `missing_scope`, så att
    mappningen är testad och inte bara skriven.
- **E2E** (`e2e/tests/tokens.spec.ts`): skapa en token för ett bolag med
  Bokföring läsa, se token en gång, anropa `ListVouchers` med den från
  testet, återkalla, och se samma anrop ge `not_signed_in`.

## AGENTS.md
Under Authentication läggs ett stycke om API-tokens: formatet, bearer-headern,
att en token är en delmängd av ägarens åtkomst, att den aldrig ändras, och
att den aldrig loggas. Under API läggs behörigheterna och de nya felkoderna.
