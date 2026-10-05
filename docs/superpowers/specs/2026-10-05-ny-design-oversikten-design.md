# Doris – Ny design, del 2: översikten

## Kontext
Del 1 gav hela webbappen det nya skalet, men startsidan är fortfarande två kort ("Välkommen", "Aktivt företag"). Det här steget ersätter den med översikten i den godkända skissen ("A2" i canvasen https://claude.ai/artifact/FpdLiRfFggm3ZVSuM6nyUL): nyckeltal, Att göra, räkenskapsårets status, intäkter och kostnader per månad, obetalda leverantörsfakturor och senaste verifikationer, för det aktiva företaget och ett valt räkenskapsår.

Steget är bara frontend. Det tillkommer inga event, kommandon, projektioner, migrationer eller RPC:er.

### Fattade beslut
| Område | Beslut |
|---|---|
| Datakälla | Åtta befintliga anrop, skickade parallellt; all summering sker i webbklienten i rena funktioner. |
| Månadsdiagram | Räknas ur `ListVouchers`, som "Senaste verifikationer" ändå hämtar. En kommentar vid summeringen pekar ut `GetMonthlyTotals` i ledger som nästa steg när listan blir tung. |
| Att göra | Leverantörsfakturor, kundfakturor, lönekörningar och AGI. Verifikationer utan underlag är inte med. |
| Fel | Ett anrop som misslyckas fäller bara de kort som behöver det. |
| Diagramfärger | Två nya tokens i `input.css`, `--chart-1` (amber) och `--chart-2` (stone), med egna värden i mörkt läge. |
| "Inloggad som" | Kortet tas bort; namnet står i kontomenyn. |

## Data
Alla anrop gäller det aktiva företaget. `fy` är det valda räkenskapsåret, som standard det som innehåller i dag (`Company.fiscal_year_start`), annars det senaste.

| Anrop | Används till |
|---|---|
| `CompanyService.GetCompany` | Sidrubrikens underrad, standardåret |
| `LedgerService.ListFiscalYears` | Årsväljaren, årets status, föregående års status |
| `LedgerService.GetTrialBalance(fy)` | Nyckeltalen |
| `LedgerService.ListVouchers(fy)` | Månadsdiagrammet, antal verifikationer, senast bokfört, senaste verifikationer |
| `InvoicingService.ListSupplierInvoices` | Att göra, obetalda leverantörsfakturor |
| `InvoicingService.ListCustomerInvoices` | Att göra |
| `PayrollService.ListPayrollRuns` | Att göra |
| `PayrollService.ListAgiMonths` | Att göra |

De tre första och `ListVouchers` hämtas om när året byts; de fyra sista bara när företaget byts. Ett svar som kommer efter att företaget bytts kastas, som på övriga sidor.

## Beräkningar
Rena funktioner i en ny modul `crates/web/src/overview.rs`, testade utan webbläsare. Belopp är öre (`i64`).

### Nyckeltal
Ur saldobalansens rader, där en rads rörelse är `debit − credit`:

| Nyckeltal | Definition |
|---|---|
| Intäkter | −Σ rörelse för konto 3000–3999 |
| Kostnader | Σ rörelse för konto 4000–8989 |
| Resultat hittills | Intäkter − Kostnader (samma konton som "Årets resultat" i rapporterna, 3000–8989) |
| Kassa och bank | Σ (`opening` + rörelse) för konto 1900–1999 |

Stora tal visas i hela kronor ("304 330 kr"); ören avrundas inte bort i summeringen, bara i visningen (trunkering mot noll).

### Per månad
För varje verifikation i året och varje rad: månaden är verifikationens datum (`ÅÅÅÅ-MM`). Intäkter och kostnader summeras med samma kontointervall och tecken som nyckeltalen. Rättelser räknas som vanliga verifikationer, så en rättad verifikation och dess rättelse tar ut varandra. Resultatet är en rad per månad i räkenskapsåret, i ordning, även för månader utan verifikationer; ett brutet eller förlängt år ger så många månader som året har.

### Räkenskapsårets förlopp
`dag = (i dag − start) + 1`, begränsad till 1..=antal dagar; `kvar = antal dagar − dag`. För ett år som inte har börjat är dag 0, för ett som har slutat är kvar 0. Andelen är `dag / antal dagar`.

### Att göra
Varje regel ger högst en rad, i den här ordningen. `i dag` är webbläsarens datum, som på övriga sidor.

| Rad | Villkor | Text | Underrad | Knapp |
|---|---|---|---|---|
| Förfallna leverantörsfakturor | `status == "unpaid"` och `due_date < i dag` | "N leverantörsfaktura har förfallit" / "… fakturor har förfallit" | Summa, och leverantör och förfallodag för den äldsta | "Visa fakturorna" → `/supplier-invoices` |
| Leverantörsfakturor som förfaller snart | `unpaid` och `i dag <= due_date <= i dag + 30` | "N leverantörsfaktura förfaller inom 30 dagar" | Summa, och nästa förfallodag med leverantör | "Visa fakturorna" → `/supplier-invoices` |
| Förfallna kundfakturor | `unpaid` och `due_date < i dag` | "N kundfaktura har förfallit" | Summa, och kund och förfallodag för den äldsta | "Visa fakturorna" → `/customer-invoices` |
| Lönekörningar att bokföra | `Finalized` och `pay_date <= i dag` | "N lönekörning kan bokföras" | Text och utbetalningsdag för den äldsta | "Öppna körningen" → `/payroll-runs/{id}` (den äldsta) |
| Öppna lönekörningar | `Open` | "N lönekörning är inte färdigställd" | Text och utbetalningsdag för den med närmast utbetalningsdag | "Öppna körningen" → `/payroll-runs/{id}` |
| AGI | `NotSubmitted` eller `Changed`, för perioder vars månad har slutat | "Arbetsgivardeklarationen för N månad är inte inlämnad" | Perioderna, t.ex. "2026-08, 2026-09" (högst tre, sedan "och N till") | "Visa deklarationerna" → `/agi` |

Förfallet (leverantör och kund) ritas med den röda ikonrundeln i skissen; övriga med den neutrala. Varje rad har ikon och text, så färgen bär ingen egen betydelse. Rubriken "Att göra" har en etikett med antalet rader. Utan rader visas "Inget att göra just nu."

Singular och plural: "1 leverantörsfaktura har förfallit", "2 leverantörsfakturor har förfallit"; motsvarande för övriga.

## Sidan
`crates/web/src/pages/home.rs` skrivs om. Layouten är den i skissen, uppifrån:

1. **`PageHeader`**: företagets namn som `<h1>`; underrad "organisationsnummer · juridisk form · bokföringsmetod". Till höger årsväljaren (`FiscalYearSelect`) och knapparna Ny lönekörning, Ny leverantörsfaktura, Ny kundfaktura (kontur) och Ny verifikation (primär, plus-ikon).
2. **Nyckeltal**: fyra kort i `grid` med `repeat(auto-fit, minmax(min(220px, 100%), 1fr))`: Resultat hittills i år ("Efter finansiella poster"), Intäkter ("Konto 3000–3999"), Kostnader ("Konto 4000–8989"), Kassa och bank ("Saldo {i dag eller årets sista dag}").
3. **Rad**: Att göra (2 delar) och Räkenskapsåret (1 del): etikett Öppet/Stängt, förloppsmätare (`role="progressbar"`), "Dag N av M", "K dagar kvar", period, antal verifikationer, senast bokfört, föregående års status, länk "Visa räkenskapsår".
4. **Rad**: Intäkter och kostnader per månad (2 delar) och Obetalda leverantörsfakturor (1 del): antal och summa, de fyra med tidigast förfallodag, etiketten "Förfallen" där det gäller, länk "Alla leverantörsfakturor".
5. **Senaste verifikationer**: de fem med högst nummer, kolumnerna Nr, Datum, Text, Belopp, länk "Alla verifikationer".

Raderna är `flex flex-wrap` med `flex: 2 1 480px` och `flex: 1 1 280px`, så att korten staplas på smal skärm.

### Diagrammet
Staplar byggda av `div`:ar, två per månad (intäkter `--chart-1`, kostnader `--chart-2`), 12px breda med 2px mellanrum och 4px rundade toppar, förankrade i baslinjen. Skalan går från 0 till närmaste "jämna" tal över det största värdet (1, 2 eller 5 gånger en tiopotens), med tre stödlinjer och etiketter i tusental kronor. Negativa månadsvärden (en månad där rättelser överväger) ritas som 0. Månadsnamnen står under; innevarande månad är markerad. En teckenförklaring står i kortets rubrik. `figure` har `role="img"` och en `aria-label` som säger vad diagrammet visar; tabellvyn är Rapporter-sidan, som kortet länkar till ("Visa resultaträkningen"). Inget hovringsläge.

Färgerna (samma som i skissen):

| Token | Ljust | Mörkt |
|---|---|---|
| `--chart-1` | `oklch(0.555 0.163 48.998)` | `oklch(0.769 0.188 70.08)` |
| `--chart-2` | `oklch(0.268 0.007 34.298)` | `oklch(0.553 0.013 58.071)` |

Förloppsmätarens fyllning använder `--chart-1`, eftersom `--primary` är för mörk mot bakgrunden i mörkt läge.

### Tillstånd
| Läge | Visas |
|---|---|
| Inget aktivt företag | `PageHeader` "Översikt" och kortet "Aktivt företag" med "Du har inga företag än." och länken "Lägg till företag", som i dag |
| Laddar | Sidrubriken direkt; varje kort visar sin rubrik och "Laddar…" tills dess data finns |
| Ett anrop misslyckas | De kort som behöver svaret visar felets svenska text (`describe`) i stället för innehåll; övriga visas som vanligt |
| Tomt år | Nyckeltal 0 kr, diagram utan staplar, "Inga verifikationer än." i Senaste verifikationer, "Inga obetalda leverantörsfakturor." |
| Stängt år | Etiketten "Stängt"; knapparna i sidrubriken är kvar (de gäller inte det valda året) |

## Tillgänglighet
- Ett `<h1>` (företagets namn); korten har `<h2>`.
- Nyckeltalens rubriker är `<h2>` så att varje tal har ett namn för skärmläsare.
- Förloppsmätaren har `aria-valuenow`, `aria-valuemin`, `aria-valuemax` och en `aria-label`.
- Att göra-knapparna är länkar med knapputseende (`LinkButton`).

## Test
TDD: varje del börjar med ett fallande test.

### Enhet (`overview.rs`)
| Funktion | Fall |
|---|---|
| Nyckeltal | Intäkter, kostnader och resultat ur blandade rader; gränskontona 2999/3000, 3999/4000, 8989/8990; kassa och bank med ingående saldo; tom lista |
| Per månad | En rad per månad i ett kalenderår; brutet år (juli–juni); förlängt år (18 månader); en rättelse tar ut sin verifikation; månader utan verifikationer är 0 |
| Förlopp | Första dagen, sista dagen, mitt i, före start, efter slut, skottår |
| Att göra | Varje regel för sig med datum på båda sidor om gränsen (förfaller i dag är inte förfallen; dag 30 är med, dag 31 inte); betalda och makulerade räknas inte; ordningen; singular och plural; AGI för innevarande månad räknas inte |
| Skala | 0 → en tom skala utan division med noll; 241 000 → 250 000; 1 → 1 |
| Hela kronor | Negativa belopp trunkeras mot noll; gruppering med hårt blanksteg |

### E2E (`e2e/tests/overview.spec.ts`)
| Test | Kontrollerar |
|---|---|
| Tomt företag | Rubriken är företagets namn; nyckeltalen visar 0 kr; "Inget att göra just nu."; "Inga verifikationer än." |
| Efter en bokförd försäljning | Intäkter, resultat och kassa visar beloppet; verifikationen står i Senaste verifikationer; antal verifikationer är 1 |
| En förfallen leverantörsfaktura | Raden finns i Att göra med rätt antal och summa; knappen leder till `/supplier-invoices`; fakturan står i Obetalda leverantörsfakturor med "Förfallen" |
| Byte av år | Med två år: väljaren byter nyckeltal och verifikationer |
| Ett anrop misslyckas | Med `ListVouchers` avbrutet visar diagrammet och Senaste verifikationer felet, medan nyckeltalen visas |
| 390 px | Sidan rullar inte i sidled, i ljust och mörkt läge |

`fixtures.ts`: `register` väntar på kontomenyn (`summary` med namnet) i stället för "Inloggad som". `companies.spec.ts` som i dag läser kortet "Aktivt företag" för ett företag skrivs om mot sidrubriken; fallet utan företag är oförändrat. `design.spec.ts` byter "Översikt" mot företagets namn där den kontrollerar startsidans `h1`.

### Övrigt
`cargo test --workspace`, båda clippy-kommandona och hela Playwright-sviten går igenom. Sidan stäms av mot skissen i ljust och mörkt läge på 1280 och 390 px med `/verify`. Wasm-storleken jämförs med före.

## Ordning
Ett commit per steg, varje steg grönt:

1. `overview.rs`: nyckeltal, hela kronor, förlopp.
2. `overview.rs`: per månad och skalan.
3. `overview.rs`: Att göra-reglerna.
4. Sidan: sidrubrik, nyckeltal, räkenskapsåret, tillstånden utan företag och vid fel; `fixtures.ts` och de befintliga specarna.
5. Sidan: Att göra, obetalda leverantörsfakturor, senaste verifikationer.
6. Sidan: diagrammet och färgtokens.
7. `AGENTS.md`, avstämning mot skissen.

## Utanför det här steget
- Nya RPC:er (`GetOverview`, `GetMonthlyTotals`).
- Verifikationer utan underlag i Att göra, "Senaste händelser" och momsrelaterade punkter.
- Hovringsläge och tabellvy i diagrammet.
- Jämförelse med föregående år i nyckeltalen.
- Del 3 (sök, filter, "Visa fler" och behandlingshistorik på verifikationssidan).
