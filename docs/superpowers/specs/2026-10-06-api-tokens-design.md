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
| Livslängd | Utgångstid krävs: en sista giltig dag (förvald om 90 dagar), högst 366 dagar fram. Token slutar gälla vid midnatt svensk tid efter den dagen och kan återkallas när som helst. |
| Ändring | En token ändras aldrig. Den som vill ändra behörigheterna återkallar token och skapar en ny. |
| Lagring | Events i identity (`ApiTokenCreated`, `ApiTokenRevoked`) och projektionen `api_tokens`. Klartexten visas en gång; bara SHA-256 sparas. |
| Kontroll | En tabell i servern (`access.rs`) anger för varje RPC vad den kräver av en token. Ett lager före tjänsterna autentiserar token en gång och nekar det tabellen inte tillåter. Tjänstens `caller()` kontrollerar bolaget och behörigheten. En RPC som saknas i tabellen nekas, och ett test jämför tabellen med `.proto`-filerna. |
| Historik | `Metadata` får `via_token`, som lagret sätter för hela anropet (tokio task-local i eventstore). `actor` är fortfarande ägaren, och modulernas funktioner ändras inte. |

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
Varje token har en egen stream, `api-token-{token_id}`, som inbjudningarna
har. Händelser (`schema_version` 1 i envelopet; tiderna är händelsens
`recorded_at`):

```
ApiTokenCreated { token_id, user_id, name, token_hash, expires_at,
                  grants: [{ company_id, scopes: ["ledger:read", …] }] }
ApiTokenRevoked { revoked_by }
```

Regler, testade given/when/then utan databas:
- `name` trimmas och är 1–100 tecken (`invalid_token_name`).
- `expires_at` ligger efter `now` och högst 367 dagar fram
  (`invalid_token_expiry`). Servern räknar fram den ur den sista giltiga
  dagen: midnatt svensk tid efter den dagen, och dagen får vara högst 366
  dagar efter i dag. 367 dagar ger plats för det och för sommartid.
- `grants` innehåller minst ett bolag, inget bolag mer än en gång och minst
  en behörighet per bolag (`invalid_token_grants`). Behörigheterna sorteras
  och dubbletter tas bort. En okänd behörighetssträng från klienten avvisas
  av servern med samma kod.
- Bara ägaren eller en admin får återkalla en token. För någon annan ser
  token ut som om den inte finns (`api_token_not_found`). En token som
  redan är återkallad ger inga events, så återkallandet är idempotent.

Att ägaren är medlem i varje bolag i grants kontrolleras i servern via
`doris_company::get_company`, eftersom identity inte läser companys
tabeller. Medlemskapet kontrolleras dessutom vid varje anrop, av modulen
själv, precis som för en session.

`Scope` är en enum i identity som serialiseras som strängarna ovan
(`ledger:read` …). En ny `Scope` senare ändrar alltså inte gamla events.

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
    created_at  TEXT NOT NULL,      -- recorded_at
    expires_at  INTEGER NOT NULL,   -- unix seconds
    revoked_at  TEXT
);
CREATE INDEX api_tokens_user ON api_tokens (user_id);

-- Operational: when a token was last used. Safe to purge.
CREATE TABLE api_token_usage (
    token_id     TEXT PRIMARY KEY,
    last_used_at INTEGER NOT NULL   -- unix seconds
);
```

- Projektionen uppdateras i samma transaktion som eventet, och ett test
  bygger om den från `read_all`.
- `last_used_at` skrivs efter anropet, och bara om det gamla värdet är
  äldre än en timme, vilket läses först. Så tar en läsning skrivlåset
  högst en gång i timmen per token. Ett misslyckat skrivförsök loggas och
  stoppar inte anropet.

Funktioner i identity (`src/api_token.rs`):
- `create_api_token(pool, owner_id, name, expires_at, grants, now) -> (Uuid, String)`
  returnerar id och klartext en gång.
- `revoke_api_token(pool, actor_id, token_id)`.
- `list_api_tokens(pool, user_id) -> Vec<ApiTokenSummary>`, nyast först,
  med `last_used_at`.
- `token_user(pool, token, now) -> Option<(User, TokenAccess)>`: `None` om
  token är okänd, utgången eller återkallad. `TokenAccess` innehåller
  `token_id` och grants.
- `touch_api_token(pool, token_id, now)`.

## Autentisering och behörighet (`doris-server`)
- **Tabellen** (`crates/server/src/access.rs`): `access(path) -> Access`
  för varje gRPC-sökväg. `Access::Company(Scope)` gäller ett bolag och
  kräver den behörigheten. `Access::Owner` (`GetStatus`, `ListCompanies`)
  får anropas med vilken token som helst. `Access::SessionOnly` gäller allt
  annat, även sökvägar som inte finns i tabellen. Ett test läser in
  `.proto`-filerna och kräver att varje RPC står i tabellen.
- **Lagret** (`auth_gate` i `lib.rs`, ersätter `session_gate`): finns en
  `authorization: Bearer`-header gäller följande.
  1. Token slås upp med `token_user`. Okänd, utgången, återkallad eller
     felformad token ger `not_signed_in`, även om en cookie också skickas.
  2. `Access::SessionOnly` ger `permission_denied("token_not_allowed")`.
  3. Annars läggs `TokenCaller { user, token_id, grants, access }` i
     anropets extensions, och anropet körs med
     `doris_eventstore::VIA_TOKEN` satt till token-id.
  4. Efteråt körs `touch_api_token`.

  Utan bearer-header gäller samma cookiekontroll som i dag för
  `LedgerService` och `InvoicingService`. En token kan alltså också skicka
  underlag upp till 20 MiB.
- **Tjänsterna:** `grpc::signed_in_user` returnerar `TokenCaller`s
  användare om en sådan finns, annars sessionens användare som i dag. Varje
  tjänsts `caller()` anropar `grpc::company_user(pool, request, company)`,
  som för en token kontrollerar följande.
  1. Ett bolag som inte finns i grants ger samma fel som ett bolag som
     inte finns (`company_not_found`).
  2. En behörighet som saknas för bolaget, eller en RPC som inte är
     `Access::Company`, ger `permission_denied("missing_scope")`.

  Medlemskapet kontrolleras därefter av modulen, som i dag. En token
  slutar därmed fungera för ett bolag som ägaren inte längre är medlem i.

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
`doris_eventstore::Metadata` får `via_token: Option<String>`
(`#[serde(default, skip_serializing_if = "Option::is_none")]`), så att
befintliga events läses som förut. `append` fyller i det från
`VIA_TOKEN` (tokio `task_local!`) när det är satt. Lagret sätter det för
hela anropet, så modulernas funktioner behöver inte ändras. Även
verifikationer som en annan modul bokför (lön, fakturor, moms) får det
därför. `actor` är fortfarande ägarens användar-id. Verifikationslistan
ändras inte i det här steget.

## API
I `proto/doris/auth/v1/auth.proto`:

```proto
rpc CreateApiToken(CreateApiTokenRequest) returns (CreateApiTokenResponse);
rpc ListApiTokens(ListApiTokensRequest) returns (ListApiTokensResponse);
rpc RevokeApiToken(RevokeApiTokenRequest) returns (RevokeApiTokenResponse);

message TokenGrant { string company_id = 1; repeated string scopes = 2; }
message CreateApiTokenRequest {
  string name = 1;
  string expires_on = 2;            // YYYY-MM-DD, the last day it works
  repeated TokenGrant grants = 3;
}
message CreateApiTokenResponse { string token_id = 1; string secret = 2; }
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
  `api_token_not_found`. Ett felaktigt datum ger `invalid_token_expiry`.
- Nya felkoder: `invalid_token_name`, `invalid_token_expiry`,
  `invalid_token_grants`, `api_token_not_found`, `missing_scope` och
  `token_not_allowed`. De mappas i `grpc.rs` och översätts i
  `src/errors.rs`.

## UI
- I kontomenyn läggs "API-tokens" (`/settings/tokens`) till med ikonen
  lucide `square-terminal`, under Passkeys. `/settings` står redan utanför
  grupperna i `section_of`.
- `/settings/tokens` följer `docs/design/README.md`: `PageHeader` med
  `LinkButton` "Ny token", och en `TableCard` med kolumnerna Namn, Bolag,
  Giltig till, Senast använd, Status (`Badge`: Aktiv, Utgången, Återkallad)
  och knappen "Återkalla" på aktiva tokens. Knappen frågar en gång till
  med webbläsarens egen bekräftelse ("Återkalla token *namn*?") innan den
  återkallar.
- `/settings/tokens/new` är ett fullbrett `Card` med fältet Namn och
  "Giltig till och med" (`type="date"`, förvalt om 90 dagar, med hinten
  "Högst ett år."). Under dem finns en `fieldset` per bolag (bolagets namn
  som `legend`) med kryssrutorna Läsa/Skriva bokföring, fakturor, lön och
  moms samt Läsa bolagsuppgifter, i två kolumner. Kryssas Skriva i kryssas
  Läsa i också. Under bolagen står att läsbehörighet för lön omfattar
  AGI-filen med personnummer.
- Efter skapandet visas token en gång i ett `Card` med ett skrivskyddat
  fält och texten "Token visas bara nu. Spara den på ett säkert ställe."
  Med "Klar" går man tillbaka till listan.
- Sidorna startar sina tasks med `crate::task::spawn_local` och läggs in i
  `design.spec.ts` och `leaving.spec.ts`. De fungerar i ljust och mörkt
  läge och på 390 px.

## Tester
- **Domän** (identity, given/when/then): skapa med giltiga värden; tomt
  namn, för långt namn, utgångstid i dåtiden eller mer än 367 dagar fram,
  tomma grants, bolag två gånger, bolag utan behörighet; återkalla som
  ägare, som admin, som annan användare, och två gånger (inga events).
- **Projektion**: `api_tokens` byggs om från `read_all` och blir lika.
- **Identity**: `token_user` ger `None` för en utgången, återkallad och
  okänd token; `last_used_at` uppdateras högst en gång i timmen.
- **Eventstore**: metadata utan `via_token` läses som förut, och `append`
  inom `VIA_TOKEN.scope` sparar token-id.
- **Tabellen** (enhetstest i `access.rs`): varje `rpc` i varje `.proto`
  står i tabellen, och en okänd sökväg är `SessionOnly`.
- **Server** (gRPC-Web med bearer, som CLI:n kommer att göra):
  - En token med `ledger:read` listar verifikationer men får
    `missing_scope` på `RecordVoucher`. Med `ledger:write` bokförs
    verifikationen, och eventet har `via_token`.
  - Ett bolag utanför grants ger `company_not_found` och syns inte i
    `ListCompanies`.
  - Utgången, återkallad och felformad token ger `not_signed_in`, även
    tillsammans med en giltig cookie.
  - `CreateApiToken`, `CreateInvitation` och `CreateCompany` med token ger
    `token_not_allowed`. `GetStatus` svarar med ägaren.
  - En bearer-token släpps igenom av lagret med ett underlag på nära
    20 MiB.
  - `CreateApiToken` med ett bolag användaren inte är medlem i ger
    `company_not_found`; en okänd behörighet ger `invalid_token_grants`.
  - Payroll, Invoicing och Vat nekar en token med fel område
    (`missing_scope`) och släpper igenom en med rätt.
- **Webb**: grants ur kryssrutorna (skriv ger läs, tomma bolag tas bort)
  och status (Aktiv, Utgången, Återkallad) är rena funktioner med
  enhetstester.
- **E2E** (`e2e/tests/tokens.spec.ts`): skapa en token för ett bolag med
  Bokföring läsa, se token en gång, anropa `ListCompanies` med den utan
  cookie (grpc-status 0), återkalla, och se samma anrop ge 16
  (`not_signed_in`).

## AGENTS.md
Under Authentication läggs ett stycke om API-tokens: formatet, bearer-headern,
att en token är en delmängd av ägarens åtkomst, att den aldrig ändras, att
den aldrig loggas och var tabellen över behörigheter finns. Under API läggs
de nya felkoderna.
