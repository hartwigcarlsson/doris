# Doris – Ändra API-tokens, med passkey

## Kontext
API-tokens (`2026-10-06-api-tokens-design.md`) ger en CLI eller en agent
behörigheter per bolag. En token kan i dag inte ändras: den som vill ge
den fler eller färre behörigheter återkallar den och skapar en ny, och
måste då byta hemligheten överallt där den används. I det här steget kan
ägaren ändra en tokens namn, sista giltiga dag och behörigheter utan att
hemligheten byts. Eftersom både att skapa och att ändra en token ger
åtkomst till bokföringen, bekräftas båda med en passkey: en stulen session
räcker inte för att ge sig själv en token.

### Fattade beslut
| Område | Beslut |
|---|---|
| Vad passkeyn bekräftar | Exakt den ändring som visas. Ändringen sparas i ceremonin vid begin och genomförs vid finish; inget förhöjt läge ligger kvar efteråt. |
| Vad som kräver passkey | Att skapa och att ändra en token. Att återkalla gör det inte, så att en läcka stängs snabbt. |
| Vad som kan ändras | Namn, sista giltiga dag och behörigheter per bolag, allt på en gång (ersätter). Hemligheten ändras aldrig. |
| Vem | Bara ägaren, med sin egen passkey. En admin kan fortfarande återkalla andras tokens men inte ändra dem. |
| Återkallad, utgången | En återkallad token kan inte ändras. En utgången kan få en ny sista dag och fungerar då igen, med samma hemlighet. |
| Modellen | Oförändrad: områden × läs/skriv per bolag, flera bolag per token. |

Utanför det här steget: ett förhöjt "sudo-läge", passkey vid återkallande
och ändring av andras tokens.

## Ny passkey kräver en befintlig
Utan detta kunde en stulen session först registrera sin egen passkey och
sedan bekräfta en token med den. Därför kräver även "lägg till passkey" en
assertion från en av användarens befintliga passkeys:

1. `BeginAddPasskey(passkey_name)` validerar namnet och startar en
   autentisering mot användarens passkeys. Ceremonin är
   `ConfirmAddPasskey { user_id, passkey_name, state }`, och svaret är request
   options.
2. `ContinueAddPasskey(ceremony_id, credential_json)` (ny) verifierar
   assertionen (`verified_use`, och användningen registreras). Den startar
   sedan registreringen som förut (`AddPasskey`, med befintliga passkeys
   undantagna) och svarar med creation options.
3. `FinishAddPasskey` är oförändrad.

Ceremonin tillhör den som startade den, gäller i 5 minuter och kan
avslutas en gång. Fel: `credential_rejected` och `ceremony_expired`.
`ContinueAddPasskey` är `SessionOnly`. Sidan Passkeys kör
`navigator.credentials.get` och sedan `navigator.credentials.create` på
samma klick.

## Domän (`doris-identity`)
Ett nytt event i tokenens ström `api-token-{token_id}`, med
`schema_version` 1. Det är en ny händelsetyp, så `ApiTokenCreated` och
`ApiTokenRevoked` betyder samma sak som förut:

```
ApiTokenChanged { name, expires_at, grants }
```

- `ApiToken::from_events` ersätter namn, utgång och grants vid
  `ApiTokenChanged`.
- `change_api_token(token, actor, cmd, now)`, där `cmd` har namn, utgång
  och grants, gäller följande:
  - `actor` måste vara ägaren. Annars blir det `NotTokenOwner`, som syns
    som `api_token_not_found`, även för en admin.
  - En återkallad token ger `TokenRevoked` (`api_token_revoked`).
  - Namn, utgång och grants valideras och normaliseras precis som i
    `create_api_token`, med samma fel och samma gräns (`now <
    expires_at <= now + MAX_TOKEN_LIFETIME`). En utgången token får alltså
    en ny sista dag.
  - En ändring som inte ändrar något ger inga events.
- Testas given/when/then utan databas.

## Lagring
- Ingen ny migration behövs för tokens. `ApiTokenChanged` uppdaterar
  `name`, `expires_at` och `grants` i `api_tokens`, i samma transaktion som
  appenden. Rebuild-testet täcker en ändrad token.
- `change_api_token(pool, actor_id, token_id, name, expires_at, grants, now)`
  laddar strömmen, beslutar och skriver i en `BEGIN IMMEDIATE`.

## Ceremonin (`doris_identity::Auth`)
Två nya sorter i `webauthn_ceremonies`, som gäller i 5 minuter och kan
avslutas en gång precis som de befintliga:

```
CreateApiToken { user_id, name, expires_at, grants, state: PasskeyAuthentication }
ChangeApiToken { user_id, token_id, name, expires_at, grants, state: PasskeyAuthentication }
```

- **Begin** (`begin_create_api_token`, `begin_change_api_token`) tar
  sessionens användare och ändringen.
  - Den kör domänreglerna utan att spara, som `check_registration` gör, så
    att fel kommer innan passkeyn efterfrågas.
  - Servern har redan kontrollerat medlemskapet i varje bolag (via
    `doris_company`, som för en ny token).
  - En användare utan passkeys kan inte vara inloggad, men skulle få
    `credential_rejected`.
  - Den startar sedan `start_passkey_authentication` mot användarens egna
    passkeys och returnerar ceremonins id och WebAuthn-alternativen.
- **Finish** (`finish_create_api_token`, `finish_change_api_token`) tar
  sessionens användare, ceremonins id och assertionen.
  1. Den tar ceremonin. En okänd, använd eller utgången ceremoni, en
     ceremoni av en annan sort och en ceremoni som en annan användare
     startade ger alla `ceremony_expired`.
  2. Den verifierar assertionen. Misslyckas det blir det
     `credential_rejected`. Passkeyn måste vara en av användarens, och
     användningen registreras med `PasskeyUsed`, där räknaren uppdateras
     som vid inloggning.
  3. Den beslutar igen och skriver `ApiTokenCreated` eller
     `ApiTokenChanged` i skrivtransaktionen. En token som återkallats
     under tiden ger `api_token_revoked`.

  Finish för en ny token returnerar id och hemlighet, som förut.
- Medlemskapet kontrolleras av servern vid begin och igen vid finish (med
  grants från ceremonin), så ett bolag användaren lämnat under de fem
  minuterna inte följer med.

## API
I `proto/doris/auth/v1/auth.proto` ersätts `CreateApiToken` av:

```proto
rpc BeginCreateApiToken(CreateApiTokenRequest) returns (BeginCeremonyResponse);
rpc FinishCreateApiToken(FinishApiTokenRequest) returns (CreateApiTokenResponse);
rpc BeginChangeApiToken(ChangeApiTokenRequest) returns (BeginCeremonyResponse);
rpc FinishChangeApiToken(FinishApiTokenRequest) returns (ChangeApiTokenResponse);

message ChangeApiTokenRequest {
  string token_id = 1;
  string name = 2;
  string expires_on = 3;            // YYYY-MM-DD, the last day it works
  repeated TokenGrant grants = 4;
}
message FinishApiTokenRequest {
  string ceremony_id = 1;
  string credential_json = 2;
}
message ChangeApiTokenResponse {}
```

- Alla fyra kräver en session och är `SessionOnly` i `access.rs`. Det är
  tabelltestet som kräver det. `CreateApiToken` tas bort, eftersom bara
  webben använder den.
- `expires_on` räknas om till midnatt svensk tid efter dagen
  (`token_expiry`), som för en ny token.
- Nya felkoder: `api_token_revoked`. De befintliga `ceremony_expired`,
  `credential_rejected`, `invalid_ceremony`, `invalid_credential`,
  `api_token_not_found` och tokenkoderna används som de är. Koden mappas i
  `grpc.rs` och översätts i `src/errors.rs`.

## UI
- I listan på `/settings/tokens` får varje token som inte är återkallad
  knappen "Ändra", som leder till `/settings/tokens/:id`.
- `/settings/tokens/:id` är samma formulär som "Ny token" (`PageHeader`
  "Ändra token"):
  - det är förifyllt med namn, sista dag (datumet i `expires_at`, som
    redan är den sista dagen) och kryssrutor ur tokenens grants;
  - knappen heter "Spara med passkey";
  - efter sparandet går man tillbaka till listan.

  Ett bolag i grants som inte längre finns i användarens bolagslista visas
  inte och följer inte med.
- "Ny token" får knappen "Skapa med passkey". Hemligheten visas efter
  skapandet, som förut.
- Båda kör begin, sedan `passkey::get(options_json)`
  (`navigator.credentials.get`) och sedan finish. Avbryter man
  passkey-dialogen visas webbläsarens fel som för inloggningen, och inget
  ändras. Under tiden är knappen låst.
- Formuläret delas mellan sidorna i `pages/api_tokens.rs`, som får ta emot
  en befintlig token. Den nya sidan läggs in i `design.spec.ts` och
  `leaving.spec.ts`, och `/settings` ligger redan utanför grupperna i
  `section_of`. Den fungerar i ljust och mörkt läge och på 390 px.

## Tester
- **Domän:**
  - ändring med giltiga värden ger `ApiTokenChanged` med normaliserade
    grants;
  - samma valideringsfel som vid skapande;
  - en annan användare och en admin ger `NotTokenOwner`;
  - en återkallad token ger `TokenRevoked`;
  - en utgången token kan förlängas;
  - en ändring utan skillnad ger inga events;
  - `from_events` med `ApiTokenChanged`.
- **Projektion:** rebuild-testet täcker en ändrad token.
- **Identity:** `token_user` ger de ändrade grants efter en ändring och
  hittar en förlängd utgången token igen.
- **Server** (gRPC-Web med testernas SoftPasskey):
  - Skapa och ändra kräver en giltig assertion från användarens egen
    passkey. En annan användares passkey ger `credential_rejected`.
  - En token som bara kan läsa ändras till att skriva och bokför sedan med
    samma hemlighet. En borttagen behörighet ger `missing_scope` vid nästa
    anrop.
  - Följande ger `ceremony_expired`:
    - en ceremoni som avslutas två gånger;
    - en ceremoni som en annan användare avslutar;
    - en ceremoni för att skapa som avslutas som en ändring.
  - Valideringsfel (namn, utgång, grants, bolag användaren inte är medlem
    i) kommer redan vid begin.
  - En token som återkallas mellan begin och finish ger `api_token_revoked`.
  - `BeginCreateApiToken` med en bearer-token ger `token_not_allowed`.
- **Webb:** formuläret förifylls ur en token (en ren funktion från grants
  till kryssrutor, med enhetstest).
- **E2E** (`e2e/tests/tokens.spec.ts`, med den virtuella autentiseraren):
  1. skapa en token med passkey;
  2. ändra den från Läsa bokföring till Skriva bokföring med passkey;
  3. se att samma hemlighet nu kan bokföra en verifikation (grpc-status 0
     på `RecordVoucher`).

## AGENTS.md
Under Authentication: att skapa och ändra en token kräver en
passkey-ceremoni som bekräftar just den ändringen (`CreateApiToken`,
`ChangeApiToken` i `webauthn_ceremonies`), att bara ägaren ändrar, och att
ändringen är `ApiTokenChanged`. Under API: de nya RPC:erna och
`api_token_revoked`.
