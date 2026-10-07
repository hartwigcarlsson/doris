# Doris – doris-cli

## Kontext
API-tokens (`2026-10-06-api-tokens-design.md`) låter ett program anropa
Doris gRPC-Web-API med behörigheter per bolag. I det här steget kommer
programmet: `doris-cli`, ett kommandoradsverktyg som en människa kan
använda i terminalen och som en AI-agent kan använda för att sköta
bokföringen. Första versionen omfattar bokföringen och läsning. Fakturor,
lön, AGI och moms läggs till när formatet har prövats.

### Fattade beslut
| Område | Beslut |
|---|---|
| Kommandoform | Som `gh`: `doris-cli <område> <åtgärd> [argument] [--flaggor]`. |
| Omfattning | `auth status`, `company list/view`, `year list`, `account list`, `ver list/view/new/correct`, `report trial-balance/ledger/statements`. |
| Agenter | `--json` på alla kommandon: exakt ett JSON-värde på stdout, fel som JSON. Exit-koder skiljer användningsfel, nekande från servern och anslutningsfel. |
| `--dry-run` | Kontrolleras på servern: `RecordVoucher` och `CorrectVoucher` får `dry_run`, servern gör allt och rullar tillbaka, och svaret visar vad som skulle ha hänt. |
| Autentisering | `DORIS_TOKEN` krävs. `DORIS_URL` krävs, utan standardvärde. `DORIS_COMPANY` är valfri. |
| Belopp | In: kronor med punkt eller komma. Ut i JSON: exakta decimalsträngar i kronor (`"1250.00"`), aldrig flyttal. |
| Felmeddelanden | Samma svenska texter som webben, från en gemensam tabell. |

## Crate och anslutning
- Ny crate `crates/cli` (`doris-cli`), med ett binärt program `doris-cli`.
  Den beror på `doris-proto` (klient), `tonic`, `tonic-web`, `hyper`,
  `hyper-util`, `hyper-tls`, `tokio`, `clap`, `serde`, `serde_json` och
  `jiff`.
  - `hyper-tls` är det enda nya beroendet. Det använder native-tls, alltså
    OpenSSL på Linux och Security.framework på macOS, som serverns
    reqwest.
- Transporten är gRPC-Web över HTTP/1.1 (`GrpcWebClientLayer`), samma som
  servertesterna och webbläsaren använder. Servern behöver alltså ingen
  ny port eller något nytt protokoll.
- Varje anrop skickar `authorization: Bearer $DORIS_TOKEN`. Token skrivs
  aldrig ut, loggas aldrig och tas aldrig med i ett felmeddelande.
- `DORIS_URL` är serverns adress, till exempel `https://doris.example.se`.
  Saknas den blir det exit-kod 3 med meddelandet "Ange serverns adress i
  DORIS_URL." `http://` godtas, men bara för `localhost` och `127.0.0.1`,
  så att en token inte skickas okrypterad över nätet.
- `make dist` bygger även `target/dist/doris-cli`.
- Underlag kan vara upp till 20 MiB per verifikation, så klienten avkodar
  svar på upp till 11 MiB, som webbens ledger-klient.

## Kommandon
Globala flaggor: `--json`, och `--company <orgnr eller id>` för kommandon
som gäller ett bolag.

| Kommando | RPC | Utdata |
|---|---|---|
| `auth status` | `GetStatus` | Vem token tillhör: namn och e-post. |
| `company list` | `ListCompanies` | De bolag token har tillgång till. |
| `company view` | `GetCompany` | Bolagets uppgifter och innevarande räkenskapsår. |
| `year list` | `ListFiscalYears` | Räkenskapsår med status (öppet/stängt). |
| `account list` | `ListAccounts` | Kontoplanen. |
| `ver list [--year]` | `ListVouchers` | Årets verifikationer, nyast först. |
| `ver view NR [--year]` | `ListVouchers` | En verifikation med rader och underlag. |
| `ver new …` | `RecordVoucher` | Den bokförda (eller testkörda) verifikationen. |
| `ver correct NR --date … [--year]` | `CorrectVoucher` | Rättelsens nummer. |
| `report trial-balance [--year]` | `GetTrialBalance` | Saldobalansen. |
| `report ledger KONTO [--year]` | `GetAccountLedger` | Huvudboken för ett konto. |
| `report statements [--year]` | `GetFinancialStatements` | Resultat- och balansräkning. |

- **Bolag:** `--company` tar organisationsnummer (med eller utan
  bindestreck) eller bolagets id. Utan flaggan gäller `DORIS_COMPANY`, och
  utan den väljs bolaget automatiskt om token har exakt ett. Annars blir
  det exit-kod 2 med en lista över bolagen. Bolaget slås upp med
  `ListCompanies`.
- **Räkenskapsår:** `--year` tar ett år (`2026`), som betyder det
  räkenskapsår som börjar det året, eller ett startdatum (`2025-07-01`).
  Utan flaggan gäller räkenskapsåret som dagens svenska datum ligger i.
  Året slås upp med `ListFiscalYears`.
- **`ver new`:**
  ```
  doris-cli ver new --date 2026-02-02 --text "Kontorsmaterial" \
      --debit 6110=800 --debit 2641=200 --credit 1930=1000 \
      --attach kvitto.pdf [--dry-run]
  ```
  - `--date` och `--text` krävs.
  - `--debit KONTO=BELOPP` och `--credit KONTO=BELOPP` får upprepas, och
    det behövs minst en rad.
  - Belopp anges i kronor med högst två decimaler, med punkt eller komma
    och utan tecken: `1250`, `1250.5`, `1250,50`.
  - `--attach FIL` får upprepas. Filnamnet är filens namn utan katalog.
    Gränserna (10 MiB per fil, 20 MiB per verifikation) kontrolleras innan
    något skickas.
  - `--input FIL|-` läser verifikationen som JSON i stället för flaggorna,
    i formen `{"date","text","lines":[{"account","debit","credit"}],
    "attachments":["sökväg"]}`, med belopp som strängar eller tal i
    kronor. Kombineras det med flaggor för datum, text eller rader blir
    det exit-kod 2.
- **`ver correct NR --date DATUM`:** rättar verifikation NR i räkenskapsåret
  (`--year`) med en rättelse daterad DATUM. `--dry-run` fungerar som för
  `new`.
- **Läsande kommandon:** `--dry-run` godtas men gör ingenting. Textläget
  säger att kommandot inte ändrar något, och JSON har `"dry_run": true`.

## `--dry-run` på servern
- `RecordVoucherRequest` får `bool dry_run = 6`, och `CorrectVoucherRequest`
  får `bool dry_run = 5`. Svaren får `bool dry_run` och, för
  `RecordVoucher`, en sammanfattning av underlagen
  (`repeated Attachment attachments`, fyllt både i riktiga och testkörda
  svar).
- Servern kör samma kod som för en riktig körning i samma `BEGIN
  IMMEDIATE`: domänbeslut, numrering, projektioner och underlag. Vid
  `dry_run` rullar den tillbaka i stället för att committa. Ledgern får
  därför en variant av sina skrivfunktioner som avslutar transaktionen med
  `rollback` (`doris_ledger::record_voucher_with_attachments(…, dry_run)`
  och `correct_voucher(…, dry_run)`).
- En testkörning sparar ingenting:
  - inget event;
  - ingen rad i `vouchers` eller `attachment_files`;
  - ingen behandlingshistorik.
  Den uppdaterar inte heller tokenens senast-använd-tid. Lagret `auth_gate`
  hoppar över `touch_api_token` när svaret är en testkörning; det enklaste
  sättet är att handlern sätter en extension på svaret.
- Felen är exakt desamma som vid en riktig körning.
- Webben skickar aldrig `dry_run`, och fältet har standardvärdet false.

## Utdata
- **Textläge** är svenska. Listor är kolumner justerade med mellanslag,
  och belopp skrivs som i webben (`1 250,00`). Datum är `ÅÅÅÅ-MM-DD`.
- **`--json`:** exakt ett JSON-värde på stdout, med fält i snake_case.
  - Belopp är decimalsträngar i kronor (`"1250.00"`), datum är
    `"ÅÅÅÅ-MM-DD"` och tider är RFC 3339.
  - Listor är arrayer av objekt.
  - Skrivande kommandon har alltid `"dry_run"`.
  - Formen för varje kommando beskrivs i `crates/cli/README.md`, med ett
    exempel per kommando.
- **Fel** går till stderr. Med `--json` skrivs i stället
  `{"error":{"code":"…","message":"…"}}` på stdout och ingenting annat.
  `code` är serverns felkod, eller en egen för klientfel: `usage`,
  `missing_token`, `missing_url`, `insecure_url`, `connection_failed`,
  `company_ambiguous`, `company_not_found`, `fiscal_year_not_found` och
  `voucher_not_found`. `message` är den svenska texten.
- **Exit-koder:**

  | Kod | Betyder | Exempel |
  |---|---|---|
  | 0 | Det gick bra | |
  | 1 | Servern eller bokföringen nekade | `voucher_unbalanced`, `fiscal_year_closed`, `missing_scope` |
  | 2 | Fel användning | Saknad flagga, felaktigt belopp, flera bolag utan `--company` |
  | 3 | Autentisering eller anslutning | Ingen `DORIS_TOKEN`, `not_signed_in`, servern svarar inte |

## Felmeddelanden
Tabellen från felkod till svensk text (`crates/web/src/errors.rs`,
`message`) flyttas till `doris-proto` som `doris_proto::messages::message(code)
-> &'static str`, eftersom båda klienterna redan beror på den. Webbens
`describe`/`describe_code` anropar den, och testerna i `errors.rs` flyttar
med. CLI:ns egna koder får sina texter i samma tabell.

## Tester
- **Enhetstester (cli):**
  - belopp, inklusive punkt och komma, fler än två decimaler, negativt
    belopp och tom sträng;
  - `KONTO=BELOPP`, och en verifikation från `--input` jämförd med samma
    verifikation från flaggorna;
  - `--year` som år och som datum;
  - kopplingen från felkod till exit-kod;
  - formatering av belopp i text och i JSON;
  - att `http://` till en annan värd än localhost nekas.
- **Integration (cli mot en riktig testserver):** CLI:ns körfunktion anropas
  med argument och en buffert för stdout och stderr, mot en `TestServer`
  med en token. Följande ska fungera:
  - `auth status`, `company list` och automatiskt valt bolag;
  - `ver new` i text och JSON, med underlag;
  - `ver new --dry-run`, som ger samma svar med `dry_run`, ett nummer, och
    ingenting sparat (`ver list` är tom efteråt);
  - `ver correct` med och utan `--dry-run`;
  - `ver view`, `report trial-balance` och `report ledger`;
  - obalans, som ger exit-kod 1 och `voucher_unbalanced` i JSON;
  - saknad `DORIS_TOKEN`, som ger 3;
  - en token utan `ledger:write` på `ver new`, som ger 1 och
    `missing_scope`;
  - fel argument, som ger 2.
- **Server:**
  - `RecordVoucher` och `CorrectVoucher` med `dry_run` ger samma svar som
    en riktig körning men inga events, verifikationer eller underlag;
  - samma fel som en riktig körning (obalans, stängt år);
  - ingen uppdatering av senast-använd-tid.
- **Webben:** testerna för felmeddelanden går igenom mot den flyttade
  tabellen.

## Utanför det här steget
Fakturor, kunder och leverantörer, lön, AGI och moms, och kommandon som
kräver passkey. En konfigurationsfil, och skalkomplettering
(`doris-cli completion`), kan läggas till senare.

## AGENTS.md
Layout: `crates/cli`. Ett stycke om doris-cli: miljövariablerna,
`--json`/`--dry-run`, exit-koderna, att `dry_run` rullar tillbaka på
servern, och att CLI:ns kommandon följer gh-formen och utökas område för
område.
