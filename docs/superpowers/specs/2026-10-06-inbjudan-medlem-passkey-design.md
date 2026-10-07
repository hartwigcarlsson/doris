# Doris – Inbjudan och ny medlem bekräftas med passkey

## Kontext
Att skapa och ändra en API-token och att lägga till en passkey bekräftas
med en av användarens egna passkeys
(`2026-10-06-api-token-andring-design.md`). En stulen session kan ändå
skaffa sig varaktig åtkomst via ett andra konto:

- en admin bjuder in angriparens e-post (`CreateInvitation`), och
  angriparen registrerar sig;
- en medlem lägger till angriparens konto i ett bolag (`AddMember`).

Det nya kontot har sedan en egen passkey och kan skapa egna tokens. I det
här steget bekräftas även dessa två åtgärder med en passkey, så att en
stulen cookie inte kan bli varaktig åtkomst till bokföringen.

### Fattade beslut
| Område | Beslut |
|---|---|
| Vad som kräver passkey | `CreateInvitation` och `AddMember`, utöver token (skapa, ändra) och ny passkey. Att skapa bolag, lista, logga ut och återkalla ger ingen ny åtkomst och är oförändrat. |
| Vad passkeyn bekräftar | Exakt åtgärden som begärdes vid begin. Den sparas i ceremonin, och finish genomför den. |
| Ceremoni | En gemensam sort för allt som bekräftas: `Ceremony::Confirm { user_id, action, state }`. Den ersätter `Ceremony::ApiToken`. |
| Gränssnitt | Begin och finish ersätter de två RPC:erna. Knapptexterna i webben är oförändrade. |
| Branch | Samma branch som API-tokens (PR #26), eftersom det fullbordar samma löfte. |

## Domän (`doris-identity`)
En ren typ för det som bekräftas, med JSON-taggen `action`:

```rust
pub enum Confirmation {
    ApiToken { request: TokenRequest },
    Invitation { email: Email },
    AddMember { company_id: Uuid, email: Email },
}
```

`AddMember` bär bara data; identity läser inga bolagstabeller.

## Ceremonin (`doris_identity::Auth`)
- `Ceremony::ApiToken` ersätts av `Ceremony::Confirm { user_id, action:
  Confirmation, state }` med serde-sorten `confirm`. Ceremonier är
  operativ data som lever i 5 minuter, så inga gamla rader behöver läsas.
- **`begin_confirmation(user_id, action, now)`** gör följande:
  1. Den kör identitys egna kontroller för åtgärden:
     - `ApiToken`: `check_new_api_token` eller `check_api_token_change`,
       som i dag;
     - `Invitation`: den nya `check_invitation(pool, creator_id, email,
       now)`, som kräver admin och att e-posten inte redan är registrerad
       eller har en giltig inbjudan, utan att spara något;
     - `AddMember`: inget här, eftersom bolaget kontrolleras i servern.
  2. Den startar `start_user_authentication` mot användarens egna
     passkeys.
- **`finish_confirmation(user_id, ceremony_id, credential, now)`** tar
  ceremonin. En okänd, använd, utgången eller annan användares ceremoni,
  eller en ceremoni av en annan sort, ger `CeremonyNotFound` eller
  `CeremonyExpired`. Den verifierar sedan assertionen med `verified_use`
  (`CredentialRejected` om det misslyckas) och returnerar den godkända
  `Confirmation`.
- `begin_api_token` och `finish_api_token` tas bort. Servern anropar
  `begin_confirmation` och `finish_confirmation` med
  `Confirmation::ApiToken`.

## Servern
- `Auth` delas som `Arc<Auth>` mellan `AuthApi` och `CompanyApi`.
  `CompanyApi::new` tar den, och `main.rs` och testerna skapar den en
  gång.
- **Inbjudan:** `CreateInvitation` ersätts av
  `BeginCreateInvitation(CreateInvitationRequest)`, som returnerar
  `BeginCeremonyResponse`, och
  `FinishCreateInvitation(FinishConfirmationRequest)`, som returnerar
  `CreateInvitationResponse`.
  - Begin kräver en admin-session och validerar e-posten via
    `begin_confirmation`.
  - Finish kräver att den godkända åtgärden är en `Invitation` och skapar
    inbjudan med `doris_identity::create_invitation`, som kontrollerar
    allt igen.
- **Medlem:** `AddMember` ersätts av `BeginAddMember(AddMemberRequest)`,
  som returnerar `doris.auth.v1`s `BeginCeremonyResponse`, och
  `FinishAddMember(FinishAddMemberRequest { ceremony_id, credential_json
  })`, som returnerar `AddMemberResponse`.
  - Begin kontrollerar först att anroparen är medlem i bolaget och slår
    sedan upp e-posten, som i dag. Ordningen gör att en icke-medlem inte
    kan se vilka e-postadresser som finns. Fel: `company_not_found`,
    `user_not_found`.
  - Finish kräver en `AddMember`, kontrollerar medlemskapet igen och
    lägger till med `doris_company::add_member`, som också kontrollerar
    medlemskapet.
- `FinishConfirmationRequest { ceremony_id, credential_json }` ersätter
  `FinishApiTokenRequest` i `auth.proto`. Token-RPC:erna tar den i
  stället, med samma fält. Company-protot har ett eget meddelande med
  samma fält, så att det inte importerar `auth.proto`.
- En godkänd åtgärd av fel sort ger `ceremony_expired`. Alla fyra nya
  RPC:er är `SessionOnly` i `access.rs`, och tabelltestet kräver det.
- Nya felkoder behövs inte. `ceremony_expired`, `credential_rejected`,
  `not_admin`, `already_exists`, `invalid_email`, `company_not_found` och
  `user_not_found` används som de är.

## Webben
- **Inbjudningar:** "Skapa inbjudan" kör begin, sedan `passkey::get` och
  sedan finish. Länken visas som förut. Kortet får raden "Du bekräftar
  med din passkey."
- **Företag:** "Lägg till medlem" kör samma flöde med samma rad.
- Knappen är låst medan flödet pågår. Ett avbrutet passkey-steg visar
  felet, och ingenting skapas.

## Tester
- **identity:**
  - en inbjudan och en medlem bekräftas med användarens egen passkey och
    returnerar exakt åtgärden;
  - en annan användares assertion ger `CredentialRejected`;
  - en ceremoni som en annan användare avslutar, en som används två
    gånger och en som har gått ut nekas;
  - en inbjudan stoppas redan vid begin för en icke-admin, en ogiltig
    e-post och en upptagen e-post, och ingen ceremonirad skapas;
  - de befintliga token-ceremonitesterna fortsätter att gå igenom via
    `Confirmation::ApiToken`.
- **server** (gRPC-Web, SoftPasskey):
  - båda flödena fungerar från början till slut;
  - en token-ceremoni som avslutas som en inbjudan eller en medlem ger
    `ceremony_expired`, och omvänt;
  - `BeginAddMember` för en icke-medlem ger `company_not_found` innan
    e-posten slås upp;
  - testhjälparna `invite`, `invite_as` och `invite_with` går via
    ceremonin.
- **e2e:** befintliga tester som skapar inbjudningar och lägger till
  medlemmar (`auth.spec.ts`, `companies.spec.ts`) går igenom med den
  virtuella autentiseraren, utan ändrade selektorer.

## AGENTS.md
Under Authentication: "Inviting someone and adding a member to a company
are confirmed with a passkey too (`Ceremony::Confirm`, kind `confirm`),
like creating or changing a token and adding a passkey, so a stolen
session cannot give a second account of its own lasting access." Under
API: de nya RPC:erna.
