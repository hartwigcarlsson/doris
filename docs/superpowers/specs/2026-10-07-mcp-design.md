# Doris – MCP över Streamable HTTP

## Kontext
doris-cli (`2026-10-07-doris-cli-design.md`) och skillen
`doris-bookkeeping` låter en AI-agent bokföra med en API-token. Det
kräver att agenten har ett skal och en installerad binär i rätt version.
I det här steget får Doris en egen MCP-server på `/mcp`, i samma binär och
på samma port som API:t och webben. Agenten ansluter med en URL och en
token. Verktygen följer serverns version, och inget behöver installeras
hos agenten.

### Fattade beslut
| Område | Beslut |
|---|---|
| Transport | Streamable HTTP utan tillstånd: `POST /mcp`, ett JSON-svar per begäran, ingen SSE och inga sessioner. |
| Protokollversioner | 2026-07-28 (`server/discover`) och 2025-11-25 (`initialize`). |
| Autentisering | Endast `Authorization: Bearer doris_…`, alltså de API-tokens som finns. Ingen OAuth och ingen sessionscookie. |
| Klienter | Klienter som kan skicka en egen header: Claude Code, Agent SDK, Cursor med flera. Connectors i claude.ai och Desktop kräver OAuth och ingår inte. |
| Omfattning | Samma som doris-cli: företag, räkenskapsår, kontoplan, verifikationer och rapporter. Verktygen växer med CLI:t, område för område. |
| Underlag | Ingår inte. Agenten bokför, och en människa lägger till underlaget i webben. |
| Arkitektur | Verktygen kör doris-cli:s kommandofunktioner mot serverns egen gRPC-router inne i processen. Varje inre anrop går genom `auth_gate` som vanligt. |

### Utanför det här steget
- OAuth 2.1 (protected resource metadata, Client ID Metadata Documents,
  samtycke med passkey). Läggs till när connectors i claude.ai eller
  Desktop behövs. Verktygen ändras inte då.
- Underlag via MCP (base64 i argument eller en egen uppladdningsendpoint).
- Fakturering, lön och moms. De kommer när motsvarande CLI-kommandon kommer.
- `structuredContent`/`outputSchema`, resources och prompts.

## Endpoint och transport
- `POST /mcp` i doris-server. Routen läggs till i `app()` före frontendens
  fallback och svarar oavsett om frontenden serveras eller inte.
- `GET /mcp` och andra metoder svarar `405 Method Not Allowed`. Servern
  skickar aldrig något på eget initiativ, och utan sessioner finns inget
  att avsluta med `DELETE`.
- En begäran är ett JSON-RPC-meddelande (`Content-Type: application/json`).
  Batchar (arrayer) besvaras med 400.
- Ett svar är alltid ett enda `application/json`-objekt.
  - En notis (ett meddelande utan `id`) besvaras med `202 Accepted` utan
    innehåll.
- Ingen `Mcp-Session-Id` skapas eller läses, och servern sparar inget
  mellan begäranden.
- En begäran får vara högst 1 MiB. Större ger `413`.

### Versioner
- **2026-07-28:**
  - Headern `MCP-Protocol-Version` måste finnas och vara lika med
    `params._meta["io.modelcontextprotocol/protocolVersion"]`. Annars blir
    svaret 400 med ett JSON-RPC-fel (`HeaderMismatch`).
  - `Mcp-Method` måste vara lika med `method`. För `tools/call` måste
    `Mcp-Name` vara lika med `params.name`. Annars 400.
  - En version som servern inte stöder ger 400
    (`UnsupportedProtocolVersionError`, med de versioner som stöds).
- **2025-11-25:** `initialize` svarar med `protocolVersion: "2025-11-25"`,
  och `notifications/initialized` ger 202. Senare begäranden från en sådan
  klient har `MCP-Protocol-Version: 2025-11-25` och saknar `Mcp-Method`.
  Det godtas.
- Saknas `MCP-Protocol-Version` helt behandlas begäran som 2025-11-25.

### Metoder
| Metod | Svar |
|---|---|
| `server/discover` | `supportedVersions`, `capabilities: {tools: {}}`, serverinfo (`name: "doris"`, `version`: Doris version), `instructions`. |
| `initialize` | Samma innehåll i 2025-11-25-formen (`protocolVersion`, `capabilities`, `serverInfo`, `instructions`). |
| `tools/list` | Alla verktyg. Ingen sidindelning. |
| `tools/call` | Verktygets resultat (se nedan). |
| `ping` | `{}` |
| övriga | JSON-RPC-fel `-32601` (Method not found), HTTP 404. |

## Autentisering
- Varje begäran, även `server/discover` och `initialize`, måste ha
  `Authorization: Bearer <token>`.
- `/mcp` slår upp token med `doris_identity::token_user` innan något annat
  görs. En token som saknas, är okänd, återkallad eller har gått ut ger
  `401` med `WWW-Authenticate: Bearer` och ett JSON-RPC-fel med meddelandet
  `not_signed_in`. Det finns ingen `resource_metadata`, eftersom det inte
  finns något OAuth-flöde att peka på.
- Sessionscookien läses inte på `/mcp`. Bara en cookie ger 401, så en
  webbsida kan inte få en inloggad användares webbläsare att anropa
  verktygen.
- `Origin`: finns headern måste den vara `DORIS_RP_ORIGIN` eller finnas i
  `DORIS_CORS_ORIGINS`. Annars blir svaret `403` (MCP-specifikationens
  skydd mot DNS rebinding). Agenter skickar normalt ingen `Origin`.
- `/mcp` kontrollerar inga scopes själv. Varje verktyg gör ett eller flera
  inre gRPC-anrop med samma token, och `auth_gate` kontrollerar dem precis
  som för CLI:t: `access.rs`, scopes per företag, medlemskap, `via_token`
  på varje händelse, och senaste användning (inte vid `dry_run`).
  `access.rs` ändras inte.
- Token finns bara i headern och i `Doris`-klienten. Den loggas aldrig och
  tas aldrig med i ett svar eller felmeddelande.

## Verktyg
Verktygen motsvarar doris-cli:s kommandon ett mot ett. `skill` har inget
verktyg.

| Verktyg | Kommando | Parametrar (obligatoriska i fetstil) | Annotations |
|---|---|---|---|
| `whoami` | `auth status` | – | `readOnlyHint` |
| `list_companies` | `company list` | – | `readOnlyHint` |
| `get_company` | `company view` | `company` | `readOnlyHint` |
| `list_fiscal_years` | `year list` | `company` | `readOnlyHint` |
| `list_accounts` | `account list` | `company` | `readOnlyHint` |
| `list_vouchers` | `ver list` | `company`, `year` | `readOnlyHint` |
| `get_voucher` | `ver view` | `company`, `year`, **`number`** | `readOnlyHint` |
| `record_voucher` | `ver new` | `company`, **`date`**, **`text`**, **`lines`**, `dry_run` | `destructiveHint: false`, `idempotentHint: false` |
| `correct_voucher` | `ver correct` | `company`, `year`, **`number`**, **`date`**, `dry_run` | `destructiveHint: true`, `idempotentHint: false` |
| `trial_balance` | `report trial-balance` | `company`, `year` | `readOnlyHint` |
| `account_ledger` | `report ledger` | `company`, `year`, **`account`** | `readOnlyHint` |
| `financial_statements` | `report statements` | `company`, `year` | `readOnlyHint` |

Alla verktyg har `openWorldHint: false`. `correct_voucher` markeras som
destructive eftersom den upphäver ett verifikats verkan, så att klienten
frågar användaren först.

### Parametrar
- Varje verktyg har ett `inputSchema` (JSON Schema, `type: object`,
  `additionalProperties: false`) och en kort beskrivning på engelska. Av
  beskrivningen framgår att svaret har samma form som motsvarande kommando
  i doris-cli:s `reference.md`.
- `company`: organisationsnummer (med eller utan bindestreck) eller id.
  Utelämnas den används tokenens enda företag. Når token flera blir felet
  `company_ambiguous`, och meddelandet listar företagen, som i CLI:t.
  Servern har ingen motsvarighet till `DORIS_COMPANY`.
- `year`: `"ÅÅÅÅ"` eller ett startdatum `"ÅÅÅÅ-MM-DD"`, som `--year`.
  Utelämnas den gäller året som innehåller dagens datum i Sverige.
- `date`: `"ÅÅÅÅ-MM-DD"`.
- `lines`: `[{"account": 6110, "debit": "800.00", "credit": "0.00"}]`.
  `debit` eller `credit` får utelämnas (räknas som 0). Belopp är
  kronor-strängar eller tal, som i `ver new --input`.
- `number` och `account`: heltal.
- `dry_run`: boolesk, standard `false`. Med `true` kör servern alla regler
  i den riktiga transaktionen och rullar tillbaka. Det som skulle ha
  bokförts returneras med `"dry_run": true`.
- `record_voucher` tar inga underlag. Svaret har `"attachments": []`.

### Svar
- Ett lyckat anrop ger
  `{"content": [{"type": "text", "text": <JSON>}], "isError": false}`, där
  `<JSON>` är exakt det värde som `doris-cli --json` skriver för samma
  kommando.
- När Doris vägrar ger anropet `isError: true` och texten
  `{"error":{"code","message"}}`, med samma koder och svenska texter som i
  CLI:t (`doris_proto::messages`). Detta är ett verktygsresultat och inget
  JSON-RPC-fel, så att modellen kan läsa felet och rätta sig.
- Ogiltiga argument (okänt fält, fält som saknas, fel typ, ogiltigt `year`
  eller datum) ger `isError: true` med koden `usage` och ett meddelande som
  namnger fältet. Inget inre anrop görs.
- Ett okänt verktygsnamn ger JSON-RPC-felet `-32602` (Invalid params).
- En token som återkallas mellan kontrollen i `/mcp` och det inre anropet
  ger `isError: true` med `not_signed_in`.

### Instruktioner
`server/discover` och `initialize` returnerar `instructions` från
`crates/cli/src/tools.md`. Texten innehåller samma bokföringsregler som
skillen, men med verktygsnamn i stället för kommandorader:
- provbokför alltid med `dry_run: true` först, och fortsätt bara om svaret
  har `"dry_run": true`;
- ett verifikat kan aldrig ändras eller tas bort, utan rättas med
  `correct_voucher`, daterad idag;
- momsberäkningen och de vanliga BAS-kontona, från SKILL.md;
- räkna om själv vid `voucher_unbalanced`, och fråga ägaren bara när ett
  faktum saknas;
- om ett svar uteblir efter ett skrivande anrop, kontrollera med
  `list_vouchers` innan något görs om;
- underlag läggs till i webben: säg till ägaren vilka verifikat som saknar
  underlag.

Texten ligger utanför `skills/doris-bookkeeping/`, eftersom allt i den
mappen installeras som skill. Den ändras tillsammans med SKILL.md.

## Dataflöde
```
klient ──POST /mcp, Bearer──▶ doris-server: mcp.rs
  1. Origin, versionsheaders och JSON-RPC
  2. doris_identity::token_user: ogiltig → 401
  3. tools/call → doris_cli::tools::call(doris, namn, argument)
       └─ samma kommandofunktion som CLI:t, Output till en Vec<u8>
            └─ Doris-klient med intern transport (ingen socket)
                 └─ serverns gRPC-router: GrpcWebLayer → auth_gate → tjänsten
  4. bufferten → content[0].text, fel → isError: true
```

### doris-cli
- `client.rs`: `Transport` blir en boxad tower-tjänst
  (`BoxCloneService<http::Request<tonic::body::Body>,
  http::Response<tonic::body::Body>, BoxError>`), som tonic-klienterna tar
  emot.
  - `Doris::new(origin, token)` bygger som i dag hyper med https och
    gRPC-Web.
  - `Doris::with_transport(token, transport)` tar emot vilken tjänst som
    helst, med en fast intern origin.
- `tools.rs` (ny, `pub mod tools`):
  - `list() -> Value` returnerar alla verktyg med `name`, `description`,
    `inputSchema` och `annotations`.
  - `call(doris: Doris, name: &str, args: Value) -> Option<Result<Value,
    Value>>`. `None` betyder okänt verktyg. Funktionen validerar
    argumenten, bygger en `Context` (`company`, `dry_run`) och anropar
    kommandofunktionen med `Output { json: true, out: Vec<u8>, … }`.
    Lyckat anrop: värdet tolkas ur bufferten. `Failure`: samma JSON som
    `Output::fail`.
  - `instructions() -> &'static str` returnerar `tools.md`.
- `commands::ver::new` delas upp. Tolkningen av flaggor, `--input` och
  filer stannar i `new`, och själva bokföringen blir
  `ver::record(context, output, voucher)`, som både `new` och verktyget
  anropar. Övriga kommandofunktioner anropas oförändrade.
- `check_year` flyttas från `lib.rs` till `commands`, så att verktygen
  använder samma kontroll.

### doris-server
- `mcp.rs` (ny) innehåller bara protokollet: HTTP, JSON-RPC, versioner och
  headers, `Origin`, tokenkontrollen, `server/discover`/`initialize` och
  mappningen av verktygsresultat. Ingen bokföringslogik.
- `app()` i `lib.rs` bygger först gRPC-routern med `auth_gate`,
  `GrpcWebLayer` och `hide_internal_messages`. En klon av den (via
  `GrpcWebClientLayer`, som i doris-cli) blir den interna transporten för
  `/mcp`. Därefter läggs `/mcp` och frontendens fallback till. Det inre
  anropet tar alltså samma väg som ett anrop från doris-cli över
  nätverket, utan socket och utan CORS-lagret.
- doris-cli flyttas från `dev-dependencies` till `dependencies` i
  doris-server. hyper-tls och clap följer med. native-tls finns redan i
  servern via reqwest. Release-binärens storlek jämförs före och efter, och
  ökningen redovisas i PR:en.

## Fel och gränsfall
| Situation | Svar |
|---|---|
| Ingen eller ogiltig token | HTTP 401, `WWW-Authenticate: Bearer`. |
| Bara sessionscookie | HTTP 401. |
| Främmande `Origin` | HTTP 403. |
| `GET`/`DELETE` | HTTP 405. |
| Över 1 MiB | HTTP 413. |
| Ogiltig JSON | HTTP 400, JSON-RPC `-32700`. |
| Batch | HTTP 400, JSON-RPC `-32600`. |
| Headers stämmer inte med innehållet (2026-07-28) | HTTP 400. |
| Okänd protokollversion | HTTP 400, med de versioner som stöds. |
| Okänd metod | HTTP 404, JSON-RPC `-32601`. |
| Okänt verktyg | JSON-RPC `-32602`. |
| Ogiltiga argument | Verktygsresultat, `isError`, `usage`. |
| Doris vägrar (`missing_scope`, `voucher_unbalanced`, `fiscal_year_closed` …) | Verktygsresultat, `isError`, serverns kod. |
| Oväntat fel i det inre anropet | Verktygsresultat, `isError`, `internal`. |

## Tester
TDD som vanligt: ett rött test i taget.

### doris-cli (`tools.rs`)
- Varje kommando i clap-trädet utom `skill` har ett verktyg, och varje
  verktyg har ett kommando, enligt en tabell i testet.
- Argumentvalidering: okänt fält, saknat obligatoriskt fält, fel typ och
  ogiltigt `year` ger `usage` med fältets namn, utan något anrop.
- `tools.md` nämner bara verktyg som `list()` har och pekar inte in i
  repot.
- Varje verktyg i `list()` har ett schema med `additionalProperties: false`
  och rätt annotations.

### doris-server (`crates/server/tests/mcp.rs`, ny)
Över riktig HTTP mot `app()`, som `http.rs`.
- `server/discover` (2026-07-28) och `initialize` (2025-11-25): versioner,
  `tools`-capability, serverversion och `instructions`.
- Felaktig `MCP-Protocol-Version`, `Mcp-Method` eller `Mcp-Name` ger 400.
  Okänd version ger 400. Notis ger 202. Okänd metod ger `-32601`. `GET`
  ger 405. Över 1 MiB ger 413.
- Saknad, okänd, återkallad och utgången token ger 401. Bara cookie ger
  401. Främmande `Origin` ger 403, och `DORIS_RP_ORIGIN` godtas.
- `tools/list` innehåller de tolv verktygen.
- `record_voucher` med `dry_run: true` sparar ingenting. Samma anrop utan
  `dry_run` bokför, och händelsen har `via_token` i metadata. (Tokenens
  senaste användning uppdateras även vid `dry_run`, eftersom verktyget
  först slår upp företaget med `ListCompanies`, precis som `ver new
  --dry-run` i CLI:t.)
- `correct_voucher` skapar en rättelse som pekar på originalet.
- En token med bara `ledger:read` får `isError` med `missing_scope` från
  `record_voucher`.
- En token med två företag och utan `company` får `company_ambiguous`.
- Paritet: `list_vouchers` och `trial_balance` via MCP ger identisk JSON
  som `doris-cli --json ver list` och `report trial-balance`.

### Befintliga tester
`crates/server/tests/doris_cli.rs` måste fortsätta passera oförändrat,
särskilt `ver new`, som nu går via `ver::record`.

### Manuell kontroll
Med `/verify` innan merge: starta servern, anslut Claude Code med
`claude mcp add --transport http doris http://localhost:3000/mcp --header
"Authorization: Bearer …"`, och låt agenten provbokföra och sedan bokföra
ett kvitto.

Inga Playwright-tester, eftersom webben inte ändras.

## Dokumentation
- **AGENTS.md:**
  - nytt avsnitt "MCP": `/mcp`, Streamable HTTP utan tillstånd,
    versionerna, bara Bearer-token, `Origin`-kontrollen, att verktygen är
    doris-cli:s kommandon via `doris_cli::tools` och att de inre anropen
    går genom `auth_gate`;
  - i doris-cli-avsnittet: varje nytt kommando får ett verktyg, och
    `crates/cli/src/tools.md` ändras tillsammans med skillen;
  - i layouten: `crates/cli` beskrivs som kommandoraden och MCP-verktygen.
- **README.md:** hur man ansluter Claude Code och andra klienter med en
  header, och att connectors i claude.ai och Desktop inte stöds förrän
  OAuth finns.
- **Skillen:** SKILL.md och reference.md ändras inte.
