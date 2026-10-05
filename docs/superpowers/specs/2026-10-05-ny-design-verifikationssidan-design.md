# Doris – Ny design, del 3: sök, filter och behandlingshistorik på verifikationssidan

## Kontext
Del 1 gav verifikationssidan det nya utseendet. Skissen ("Verifikationer" i canvasen https://claude.ai/artifact/FpdLiRfFggm3ZVSuM6nyUL) har fyra saker till som det här steget inför: sök, två filter, senaste först med "Visa fler", och behandlingshistorik i den utfällda raden. Kontonamnen i konteringen blir också länkar till huvudboken.

Sök, filter och "Visa fler" görs i webbklienten: sidan hämtar redan hela årets verifikationer. Behandlingshistoriken kräver att `ListVouchers` skickar två uppgifter som projektionen redan har. Det tillkommer inga event, kommandon, projektioner eller migrationer.

### Fattade beslut
| Område | Beslut |
|---|---|
| Sök och filter | I webbklienten, i rena funktioner i en ny modul `crates/web/src/voucher_search.rs`. |
| Ordning | Senaste först (högst nummer överst). |
| "Visa fler" | 50 rader åt gången. |
| Behandlingshistorik | `Voucher` i protot får `recorded_at` och `recorded_by_name`; servern läser dem ur `vouchers` och `users`. |
| Persondata | Bara visningsnamnet skickas, aldrig e-postadressen. Inget av det loggas. |
| Adressfältet | Sök och filter läggs inte i adressen; räkenskapsåret ligger kvar som i dag. |

## Servern
### Proto
`proto/doris/ledger/v1/ledger.proto`, `message Voucher`:

```proto
  // When the voucher was recorded (RFC 3339, UTC) and by whom (the user's
  // display name). Set by ListVouchers; empty elsewhere.
  string recorded_at = 8;
  string recorded_by_name = 9;
```

### Läsningen
`crates/ledger/src/domain.rs`: `Voucher` får `pub recorded: Option<Recorded>`, där `Recorded { at: String, by: String }`. Domänens eget tillstånd (det som `evolve` bygger) sätter `None`; uppgiften hör till projektionen.

`crates/ledger/src/queries.rs`, `list_vouchers`: huvudfrågan läser också `recorded_at` och visningsnamnet:

```sql
SELECT v.number, v.date, v.text, v.corrects, v.corrected_by, v.recorded_at,
       COALESCE(u.display_name, '')
FROM vouchers v LEFT JOIN users u ON u.user_id = v.recorded_by
WHERE v.company_id = ? AND v.fiscal_year_start = ? ORDER BY v.number
```

`LEFT JOIN` och `COALESCE`, så att en verifikation alltid listas även om användaren saknas i `users`; namnet är då tomt och klienten visar bara tidpunkten. `users` är identitetens projektion, i samma databas; ledger läser den, som `doris_company::get_company` redan läser medlemskap. Ingen främmande nyckel läggs till.

`crates/server/src/ledger.rs`, `voucher_message`: fyller de två fälten ur `recorded`, tomma strängar när den är `None`.

Åtkomsten är oförändrad: `list_vouchers` kontrollerar medlemskap först, och namnet som visas är en medlems (eller tidigare medlems) visningsnamn, som medlemmarna redan ser på företagssidan.

## Webbklienten
### Sök och filter (`voucher_search.rs`)
```rust
pub struct Filter { pub query: String, pub missing_attachment: bool, pub corrections: bool }
pub fn matches(voucher: &lpb::Voucher, accounts: &[lpb::Account], filter: &Filter) -> bool
pub fn shown(total: usize, limit: usize) -> usize
```

En verifikation visas om den uppfyller alla aktiva villkor:

| Villkor | Regel |
|---|---|
| Sök | Söksträngen delas vid blanksteg i ord. Varje ord ska finnas i minst ett av: numret, texten, ett kontonummer på en rad, ett kontonamn på en rad, eller ett belopp. Jämförelsen bortser från skiftläge. Tom söksträng matchar allt. |
| Belopp | Ett ord som bara består av siffror, blanksteg-fria tusental och ett valfritt decimaltecken (`,` eller `.`) jämförs mot verifikationens summa och varje rads belopp, skrivna utan tusentalsavgränsare: "1250" och "1250,00" matchar 1 250,00. "125" matchar inte 1 250,00 som belopp, men kan matcha ett nummer eller en text. |
| Nummer | Ett ord av bara siffror matchar numret om det är lika med det ("21" matchar ver 21, inte 210 eller 121). |
| Kontonummer | Ett ord av bara siffror matchar ett kontonummer om kontot börjar med ordet ("19" matchar 1930). |
| Saknar underlag | `attachments` är tom. |
| Rättelser | `corrects != 0` eller `corrected_by != 0`. |

Kontonamn slås upp i kontoplanen som sidan redan hämtar.

### Sidan (`pages/vouchers.rs`)
- Ovanför tabellen, i kortet: sökfältet (`type="search"`, etiketten "Sök bland verifikationer" för skärmläsare, platshållaren "Sök nummer, text, konto eller belopp") och kryssrutorna "Saknar underlag" och "Rättelser".
- Listan sorteras med högst nummer först.
- De 50 första som matchar visas. Under tabellen: "Visar N av M" där M är antalet som matchar, och knappen "Visa fler" när N < M. Sök, filter, byte av år och byte av företag återställer till 50.
- Utan träffar: "Inga verifikationer matchar." Ett år utan verifikationer visar "Inga verifikationer under räkenskapsåret."
- `TableCard` får den filterrad som del 1 sköt upp: en valfri `toolbar` ovanför tabellen.
- Den utfällda raden: under underlagen, rubriken "Behandlingshistorik" och raden "Bokförd 2026-10-02 14:12 av Anna Lind" (webbläsarens tidszon, utan sekunder). Utan namn: "Bokförd 2026-10-02 14:12". Utan tidpunkt (äldre svar) visas inget.
- Kontot på varje konteringsrad är en länk till `/trial-balance/{konto}?fy={år}`.

Tidpunkten formateras av en ren funktion som tar tidszonens förskjutning i minuter, så att den går att testa; sidan läser förskjutningen ur `js_sys::Date`.

## Felhantering
Oförändrad. Sök och filter kan inte misslyckas; de arbetar på det som redan är hämtat.

## Tillgänglighet
- Sökfältet och kryssrutorna har etiketter.
- "Visar N av M" är `role="status"`, så att en skärmläsare hör att listan ändrades.
- Raden för behandlingshistorik är vanlig text under en `<h2>`, som "Underlag".

## Test
TDD: varje del börjar med ett fallande test.

### Ledger (`crates/ledger/tests`)
| Test | Kontrollerar |
|---|---|
| `list_vouchers` ger vem och när | Efter en bokföring är `recorded.at` händelsens tidpunkt och `recorded.by` användarens visningsnamn |
| Två medlemmar | En verifikation som en annan medlem bokfört visar den medlemmens namn |
| Användaren saknas | En `recorded_by` utan rad i `users` ger tomt namn, och verifikationen listas ändå |
| Ombyggnad | Projektionen ombyggd ur `read_all` ger samma `recorded` (det befintliga ombyggnadstestet utökas) |

### Server (`crates/server/tests`)
`ListVouchers` över gRPC-Web ger `recorded_at` och `recorded_by_name`; e-postadressen finns inte i svaret.

### Enhet (`voucher_search.rs`, `format.rs`)
| Funktion | Fall |
|---|---|
| `matches`, sök | Tom sträng; text utan hänsyn till skiftläge; flera ord ska alla matcha; nummer exakt (21 mot 21, 210, 121); kontonummer som prefix; kontonamn; belopp med och utan decimaler och med punkt; "125" matchar inte 1 250,00; ett konto som saknas i kontoplanen |
| `matches`, filter | Saknar underlag; rättelser (både den rättade och rättelsen); båda samtidigt; tillsammans med sök |
| `shown` | Färre än gränsen, lika med, fler |
| Tidpunkt | UTC till lokal tid över midnatt och över årsskifte, positiv och negativ förskjutning; ogiltig sträng ger tom text |

### E2E (`e2e/tests/ledger.spec.ts` eller en ny `voucher_search.spec.ts`)
| Test | Kontrollerar |
|---|---|
| Sök | Med tre verifikationer: sökning på ett ord i texten, på ett konto och på ett belopp visar rätt rader; rensat fält visar alla |
| Filter | "Saknar underlag" visar bara verifikationen utan underlag; "Rättelser" visar den rättade och rättelsen |
| Ordning | Högst nummer står överst |
| Visa fler | Med 51 verifikationer visas 50 och "Visar 50 av 51"; "Visa fler" visar den sista och knappen försvinner |
| Behandlingshistorik | Den utfällda raden visar "Bokförd" med dagens datum och den inloggades namn |
| Kontolänk | Kontot i konteringen leder till huvudboken för kontot och året |

De 51 verifikationerna bokförs genom gRPC-Web direkt i testet, inte genom formuläret, för att testet ska bli snabbt.

### Övrigt
`cargo test --workspace` (med `crates/ledger/tests/stress.rs`), båda clippy-kommandona och hela Playwright-sviten går igenom. Sidan stäms av mot skissen med `/verify`.

## Ordning
Ett commit per steg, varje steg grönt:

1. Ledger och proto: `recorded` i `list_vouchers`, fälten i protot och i serverns svar.
2. `voucher_search.rs`: `matches` och `shown`.
3. Tidpunktens formatering.
4. Sidan: senaste först, sök, filter, "Visa fler".
5. Sidan: behandlingshistorik och kontolänkar.
6. `AGENTS.md`, avstämning mot skissen.

## Utanför det här steget
- Sökning över flera räkenskapsår och sökning i servern.
- Historik för underlag som lagts till i efterhand, och för rättelser utöver det som redan visas som status.
- Sök och filter i adressfältet.
- Rättningen av samma krasch-mönster på saldobalansen som hittades i del 2 (`trial_balance.rs`); den görs för sig.
