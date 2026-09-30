# Doris – Steg 3: Aktivt företag

## Kontext
En användare som sköter bokföringen åt flera företag arbetar med ett av dem åt gången. Hen ska kunna välja det **aktiva företaget** i en rullista högst upp i menyn. Allt som byggs framöver, som verifikationer, kontoplan och rapporter, gäller det aktiva företaget.

### Fattade beslut
| Område | Beslut |
|---|---|
| Lagring | Valet sparas i webbläsarens `localStorage` och gäller den webbläsaren. Alla flikar delar samma aktiva företag. Inga ändringar görs på servern. |
| Nyckel | `doris.active_company.{användar-id}`, så att två personer som delar webbläsare får var sitt val. Värdet är företagets UUID och ingen personuppgift. |
| Automatiskt val | Det sparade företaget blir aktivt om användaren fortfarande är medlem. Annars blir det första företaget i listan aktivt, i den ordning `ListCompanies` ger (bokstavsordning). Utan företag är inget företag aktivt. |
| Nytt företag | Ett företag som användaren just har lagt till blir aktivt direkt. |
| Säkerhet | Valet i webbläsaren ger aldrig åtkomst till något. Varje anrop som gäller ett företag skickar `company_id`, och servern kontrollerar medlemskapet som förut (`company_not_found` för den som inte är medlem). |

## Frontend
- **Delat tillstånd:** `app.rs` får ett tillstånd som heter `Companies` och som sätts som context bredvid `Session`. Det innehåller listan över användarens företag (`cpb::CompanySummary`) och id:t för det aktiva företaget.
  - Listan hämtas med `ListCompanies` när en användare är inloggad, och hämtas igen efter inloggning.
  - När en användare loggar ut töms tillståndet.
  - Tillståndet har en funktion för att läsa in listan igen, som används när ett företag läggs till.
- **Val av aktivt företag:** en ren funktion, `resolve_active(stored: Option<&str>, companies: &[CompanySummary]) -> Option<String>`, avgör vilket företag som är aktivt. Den enhetstestas.
- **Lagring:** läsning och skrivning av `localStorage` görs via `web-sys` (feature `Storage`). Alla fel ignoreras, till exempel i privat läge. Valet gäller då bara den aktuella sidvisningen.
- **Rullistan:** den ligger i sidhuvudet direkt efter "Doris" och visas bara för en inloggad användare.
  - Den är en inbyggd `<select>` med presetets native-select-klasser och en tillgänglig etikett, "Aktivt företag". Etiketten är visuellt dold men finns för skärmläsare och tester.
  - Varje alternativ visar företagets namn och har klasserna `SELECT_OPTION`.
  - Ett byte sparas direkt.
  - Utan företag visas länken "Lägg till företag" (till `/companies/new`) i stället för rullistan.
- **Nytt företag:** när `CreateCompany` lyckas läses listan in igen och det nya företaget blir aktivt, innan sidan navigerar till företagssidan.
- **Startsidan:** den visar ett kort med rubriken "Aktivt företag", med namn, organisationsnummer och en länk till `/companies/{id}`. Utan företag visar kortet texten "Du har inga företag än." och en länk till `/companies/new`.

## Tester
- **Enhetstester för `resolve_active`:**
  - Ett sparat id som finns i listan blir aktivt.
  - Ett sparat id som saknas i listan ger det första företaget.
  - Utan sparat id blir det första företaget aktivt.
  - En tom lista ger inget aktivt företag.
- **Playwright:**
  - Två företag. Det senast tillagda är aktivt. Användaren byter till det andra i rullistan, laddar om sidan, och det valda är fortfarande aktivt, både i rullistan och på startsidan.
  - Utan företag visas länken "Lägg till företag" i sidhuvudet och ingen rullista.

## Utanför omfattningen
Val av aktivt företag på servern eller i URL:en, delning av valet mellan enheter, samt bokföringsfunktioner som använder det aktiva företaget. Sådana kommer i senare steg.
