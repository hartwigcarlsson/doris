# Doris – Ny design, del 1: skalet och alla vyer

## Kontext
Webbappen har vuxit till 24 vyer, men skalet är kvar från de första stegen: femton länkar på två rader i sidhuvudet, en smal centrerad kolumn, och rubrikrader som varje sida skriver för hand. En ny design är framtagen och godkänd som skiss ("A2" och "Verifikationer" i canvasen https://claude.ai/artifact/FpdLiRfFggm3ZVSuM6nyUL). Det här steget inför den designen i hela `crates/web` så att ingen vy är kvar i det gamla utseendet.

Steget är bara frontend. Det tillkommer inga event, kommandon, projektioner, migrationer eller RPC:er, och ingen vy får nya funktioner.

### Uppdelning
| Del | Innehåll | Status |
|---|---|---|
| 1 | Skalet, gemensamma komponenter och alla befintliga vyer | Den här specen |
| 2 | Översikten: nyckeltal, Att göra, månadsdiagram m.m. (kräver nya frågor i servern) | Egen spec |
| 3 | Nytt på verifikationssidan: sök, filter, "Visa fler", behandlingshistorik | Egen spec |

Efter del 1 är startsidan fortfarande dagens två kort ("Välkommen", "Aktivt företag"), i det nya skalet.

### Fattade beslut
| Område | Beslut |
|---|---|
| Meny | En rad i sidhuvudet med grupperade, utfällbara menyer (A2). |
| Utfällbara menyer | `<details>`/`<summary>` med gemensamt `name`, så att webbläsaren håller en öppen i taget. |
| Ikoner | Lucide, inlinade som SVG, bara i de utfällda menyerna och där de redan finns. Menyraden är text. |
| Bredd | `main` och sidhuvudet är `max-w-6xl` (72rem). `data-wide` tas bort. |
| Formulär | Kortet behåller presetets bredd (högst 352 px) och ligger till vänster under sidrubriken. |
| Inloggning och registrering | Förblir centrerade kort; de saknar meny och sidrubrik. |
| Smal skärm | Menyraden radbryts under logotyp och företagsväljare. Ingen hamburgermeny. |
| Preset | Värden från shadcn-preset `b1Gdz9bFY` gäller fortfarande; inga nya tokens i `input.css`. |

## Sidhuvudet
En rad, `min-h-12`, i den här ordningen:

1. **Doris** – länk till `/`.
2. **Företagsväljaren** – `ActiveCompanySelect`, oförändrad, minst 150 px bred.
3. **Huvudmenyn** (`<nav aria-label="Huvudmeny">`), visas när ett företag är aktivt:

| Menyrad | Innehåll |
|---|---|
| Översikt | länk till `/` |
| Bokföring ▾ | Verifikationer `/vouchers` (receipt-text), Saldobalans `/trial-balance` (scale), Rapporter `/financial-statements` (chart-column), Kontoplan `/accounts` (list-tree), Räkenskapsår `/fiscal-years` (calendar-range) |
| Inköp ▾ | Leverantörsfakturor `/supplier-invoices` (file-text), Leverantörer `/suppliers` (building-2) |
| Försäljning ▾ | Kundfakturor `/customer-invoices` (file-text), Kunder `/customers` (contact) |
| Lön ▾ | Lönekörningar `/payroll-runs` (banknote), Anställda `/employees` (users), Arbetsgivardeklaration `/agi` (landmark) |

Försäljning och Arbetsgivardeklaration kom till när grenen rebasades på main, som då hade fått kundfakturor och AGI. Innan dess var Kunder en egen länk i menyraden.

4. **Användarmenyn** längst till höger (`ml-auto`): en rund markering med initialerna ur visningsnamnet, namnet och en pil. Innehåll: Företag `/companies` (building), Passkeys `/settings/passkeys` (key-round), Inbjudningar `/admin/invitations` (mail-plus, bara för administratörer), en avdelare och Logga ut (log-out).

Utloggad visas bara "Doris". Inloggad utan aktivt företag visas Doris, företagsväljaren och användarmenyn.

### Aktuell sida
Länken eller menyn som innehåller den aktuella sökvägen markeras med `bg-muted`, `font-medium` och förgrundsfärg; övriga är `text-muted-foreground`. Den aktuella länken har `aria-current="page"` (leptos_router sätter det på `<A>`). En undersida räknas till sin lista: `/vouchers/new` markerar Bokföring, `/trial-balance/:account` markerar Bokföring, `/supplier-invoices/new` markerar Inköp, `/payroll-runs/:id` markerar Lön, `/opening-balances` markerar Bokföring, `/companies/*` ingen menyrad.

### Utfällbara menyer
En komponent, `NavMenu`, i en ny fil `crates/web/src/nav.rs` som också tar över `Header` från `app.rs`:

- `<details name="doris-nav">` med `<summary>` som knapp (presetets ghost-knapp, `h-7 px-2`, med chevron-down) och en `<ul>` som panel: `absolute`, `top-8`, `min-w-46`, `rounded-lg bg-popover p-1 ring-1 ring-foreground/10 shadow-md`. Användarmenyns panel är högerställd.
- Varje val är en `<A>` med ikon (`size-3.5 text-muted-foreground`) och text, `h-7 px-2 rounded-sm hover:bg-muted`.
- Det gemensamma `name` gör att webbläsaren stänger den öppna menyn när en annan öppnas.
- En lyssnare på `document` stänger alla `details[name="doris-nav"]` vid klick utanför en öppen meny, vid Escape och när sökvägen ändras (en `Effect` på `use_location().pathname`). Det är den enda egna logiken.
- `summary` får `list-none` och `[&::-webkit-details-marker]:hidden` så att webbläsarens triangel inte syns.

`--popover` finns redan i `input.css`.

## Sidlayout
- `main`: `mx-auto w-full max-w-6xl px-4 py-10`. `has-[[data-wide]]` och alla `data-wide` tas bort.
- Varje vy är ett `grid gap-6` med en `PageHeader` först.
- Inloggning och registrering lägger sitt kort i `mx-auto w-full max-w-[22rem]`.

## Komponenter i `ui.rs`
Klasslistorna kopieras från presetets genererade komponenter, som för de befintliga.

| Komponent | Vad den är |
|---|---|
| `PageHeader` | `title`, valfri `description`, valfria `children` som åtgärder till höger. `<h1 class="text-sm font-medium">`, underrad `text-muted-foreground`, raden `flex flex-wrap items-end justify-between gap-4`. |
| `Badge` | shadcn badge: `h-5 rounded-full px-2 text-[0.625rem] font-medium`. Varianter `Secondary` (standard), `Outline` och `Destructive` (`bg-destructive/10 text-destructive`). |
| `Variant::Outline` | Ny knappvariant: `border-border hover:bg-muted dark:bg-input/30`. |
| `LinkButton` | En `<A>` med knappens klasser, för "Ny …"-åtgärder. Tar `variant` och valfri `icon`. |
| `Card` | Befintlig. Rubriken blir `<h2>`, eftersom sidans `<h1>` nu ligger i `PageHeader`. Ny prop `narrow` som sätter `max-w-[22rem]` (352 px, presetets formulärbredd). |
| `TableCard` | Ett kort (`rounded-lg bg-card ring-1 ring-foreground/10`) runt `Table`. En filterrad ovanför tabellen läggs till i del 3, när den först behövs. |
| Ikoner | En `Icon`-komponent med ett enum för de Lucide-ikoner appen använder (de tretton i menyn, plus `Plus`, `ChevronDown`, `ChevronRight`, `Paperclip`). `PaperclipIcon` ersätts av den. Banorna hämtas ur lucide-static 1.52.0. |

## Vyerna
Ingen vy får ny funktion; all text som finns i dag behålls om inget annat sägs.

### Listor
`PageHeader` med rubrik och "Ny …"/"Lägg till …" som primär `LinkButton` med plus, där sidan har en sådan länk i dag. Val av räkenskapsår och andra filter ligger till höger i `PageHeader`. Tabellen ligger i `TableCard`.

| Vy | Särskilt |
|---|---|
| Verifikationer | Som skissen "Verifikationer" utan sök, filter, "Visa fler" och behandlingshistorik (del 3). Status blir `Badge`. Radens nummer är en knapp med chevron. Den utfällda raden har `bg-muted/50`, konteringen som en liten tabell med Konto, Debet, Kredit och Summa, och underlag till höger. "Rätta" är kvar som ghost-knapp. |
| Kunder, Leverantörer | Inaktiv part visas med `Badge` (Outline). Formuläret för ny/ändra ligger kvar på sidan, i ett `Card`. |
| Leverantörsfakturor | Status (Obetald, Betald, Makulerad, Förfallen) som `Badge`; Förfallen är `Destructive`. |
| Lönekörningar | Öppen, Färdigställd, Bokförd som `Badge`. |
| Anställda | Inaktiv som `Badge` (Outline). |
| Kontoplan | Tabell i `TableCard`. |
| Företag | Tabell eller lista i `TableCard`, "Lägg till företag" som primär `LinkButton`. |
| Räkenskapsår | Öppet/Stängt som `Badge`. Stäng och öppna igen är kvar som i dag. |
| Passkeys, Inbjudningar | Listan i `TableCard`, formuläret i ett smalt `Card` under. |

### Rapporter
| Vy | Särskilt |
|---|---|
| Saldobalans | `PageHeader` med val av räkenskapsår till höger, tabellen i `TableCard`. Summarader `font-medium`. |
| Huvudbok (`/trial-balance/:account`) | `PageHeader` med kontots nummer och namn som rubrik och en länk tillbaka till saldobalansen i underraden. |
| Resultat- och balansräkning | `PageHeader` "Rapporter"; de två uppställningarna i var sitt `TableCard`, sida vid sida från `lg`. |

### Formulär
| Vy | Särskilt |
|---|---|
| Nytt företag, Företagssidan, Ingående balanser | `PageHeader`, sedan smalt `Card` till vänster. Ingående balansers radredigerare får full bredd i ett vanligt `Card`. |
| Ny verifikation, Ny leverantörsfaktura, Lönekörning | `PageHeader`, sedan ett `Card` i full bredd med radredigeraren. Knapparna längst ner i kortet. |
| Inloggning, Registrering | Centrerat smalt `Card`, som i dag. |

### Startsidan
Dagens två kort, med `PageHeader` "Översikt" och korten smala och vänsterställda. Ersätts i del 2.

## Felhantering
Oförändrad. `ErrorAlert` ligger direkt under `PageHeader` på varje sida.

## Tillgänglighet
- `<summary>` är fokuserbar och öppnas med Enter och mellanslag av webbläsaren; Escape stänger via lyssnaren.
- Ikoner är `aria-hidden`; varje menyval har text.
- Sidan har ett `<h1>` (i `PageHeader`); kort har `<h2>`.
- Status visas alltid som text i etiketten, aldrig bara som färg.

## Test
TDD: varje steg börjar med ett fallande test.

### E2E (`e2e/tests`)
`design.spec.ts` får nya tester och det befintliga headertestet skrivs om:

| Test | Kontrollerar |
|---|---|
| Sidhuvudet är en rad | På 1280 px har `banner` höjden 48–49 px för en administratör med företag. |
| Menyerna | Bokföring, Inköp, Lön och användarmenyn öppnas med klick och visar exakt länkarna i tabellen ovan; att öppna en stänger den förra; Escape, klick utanför och navigering stänger. |
| Aktuell sida | På `/vouchers` har Verifikationer `aria-current="page"` och Bokföring är markerad. |
| Inget spiller ut | På 1280 och 390 px: företagsväljaren är minst 150 px och ingen `nav` har `scrollWidth > clientWidth`. |
| Formulär | Det befintliga måttestet på `/register` är kvar. Nytt: på `/companies/new` är kortet högst 352 px brett och dess vänsterkant ligger i linje med sidrubrikens. |
| En rubrik per sida | Varje inloggad vy har exakt ett `h1`. |

`fixtures.ts` får `openMenu(page, name)` och `goTo(page, menu, link)`. Tester som i dag klickar på en länk i sidhuvudet går via dem. Tester som letar efter grå statustext hittar samma text i etiketten och ska inte behöva ändras.

### Enhet
`nav.rs` får en ren funktion som avgör vilken menyrad en sökväg hör till, med ett test per rad i avsnittet "Aktuell sida".

### Övrigt
`cargo test --workspace`, `cargo clippy --workspace -- -D warnings` och `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings` går igenom. Wasm-storleken efter `make dist` jämförs med före och noteras i PR:en.

### Avstämning mot skissen
Efter sista steget körs appen (`/verify`) och Översikt (skalet), Verifikationer och ett formulär jämförs med canvasen i ljust och mörkt läge, på 1280 och 390 px.

## Ordning
Ett commit per steg, varje steg grönt:

1. Ikoner och komponenter i `ui.rs` (`Icon`, `Badge`, `Variant::Outline`, `LinkButton`, `PageHeader`, `TableCard`, `Card` med `<h2>` och `narrow`).
2. `nav.rs`: sidhuvudet med menyer, `main` i ny bredd, `fixtures.ts`-hjälparna.
3. Verifikationer och Ny verifikation.
4. Saldobalans, Huvudbok, Rapporter, Kontoplan, Räkenskapsår, Ingående balanser.
5. Leverantörsfakturor, Ny leverantörsfaktura, Leverantörer, Kunder.
6. Lönekörningar, Lönekörning, Anställda.
7. Företag, Nytt företag, Företagssidan, Passkeys, Inbjudningar, Startsidan, Inloggning, Registrering.
8. `data-wide` bort, `AGENTS.md` uppdaterad (avsnitten Frontend och Style), avstämning mot skissen.

## Utanför det här steget
- Översiktens innehåll (del 2) och verifikationssidans nya funktioner (del 3).
- Globalt val av räkenskapsår i sidhuvudet.
- Hamburgermeny eller annan särskild mobilmeny.
- Nya färger, diagramtokens eller andra ändringar i `input.css`.
