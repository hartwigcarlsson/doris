# Doris – Steg 7: Underlag till verifikationer

## Kontext
En verifikation har i dag datum, text och konteringsrader, men inget underlag. BFL 5 kap. 6–7 § kräver att varje affärshändelse har en verifikation som innehåller eller hänvisar till underlaget, till exempel ett kvitto eller en faktura. 7 kap. kräver att underlaget, som är räkenskapsinformation, bevaras i 7 år i läsbar form. Det här steget låter användaren bifoga PDF:er och bilder till en verifikation, både när den bokförs och i efterhand. Filerna lagras i SQLite-filen, så systemet fortfarande består av en binär och en databasfil.

### Fattade beslut
| Område | Beslut |
|---|---|
| Filtyper | PDF, JPEG och PNG. Servern avgör typen från filens första byte och litar inte på klientens uppgift. Andra typer avvisas. |
| Tidpunkt | Filerna kan väljas i formuläret Ny verifikation och bokförs då i samma transaktion som verifikationen. Fler kan läggas till i efterhand från grundboken. |
| Stängt år | Underlag får läggas till på verifikationer i stängda år. Ett underlag ändrar inga belopp, och händelsen loggas med vem och när. Det är det enda som ett stängt år tar emot. |
| Varaktighet | Ett underlag tas aldrig bort och får aldrig ett nytt namn. Filtabellen är append-only och skyddas med triggers. |
| Lagring | Byten ligger som BLOB i `attachment_files`, med SHA-256 som nyckel, så samma fil lagras bara en gång. Kopplingen till verifikationen är ett event. |
| Storlek | Högst 10 MiB (10 485 760 byte) per fil och högst 20 MiB i ett anrop. Tomma filer avvisas. |
| Transport | gRPC-Web, som allt annat. Byten skickas i `bytes`-fält. Nedladdningen öppnas som en Blob-URL i webbläsaren. |
| Åtkomst | Alla medlemmar i företaget. En fil kan bara läsas via en verifikation i det egna företaget, aldrig enbart med hashen. |
| Krav | En verifikation behöver inget underlag. Rättelser och "Årets resultat" är sina egna underlag. Ingen varning visas. |

## Domän (`crates/ledger/src/domain.rs`, ren och utan I/O)

### Värdeobjekt
- `ContentType`: `Pdf`, `Jpeg` eller `Png`. `as_mime()` ger `application/pdf`, `image/jpeg` eller `image/png`.
- `sniff(data: &[u8]) -> Result<ContentType, DomainError>`: `%PDF-` ger PDF, `FF D8 FF` ger JPEG och `89 50 4E 47 0D 0A 1A 0A` ger PNG. Allt annat ger `UnsupportedAttachmentType`.
- `AttachmentName::parse(raw)` trimmar namnet. Det ger `InvalidAttachmentName` om namnet är tomt, är längre än 255 tecken eller innehåller `/`, `\` eller kontrolltecken.
- `Attachment { sha256: String, file_name: AttachmentName, content_type: ContentType, size: u64 }`. `sha256` är 64 hex-tecken med gemener.
- `Attachment::new(file_name, data, sha256)` validerar namnet, att filen inte är tom (`EmptyAttachment`), att storleken är högst `MAX_ATTACHMENT_SIZE` (`AttachmentTooLarge`) och filtypen. Hashen räknas av anroparen (lib.rs, med `sha2`), så att domänen förblir fri från kryptoberoenden.

### Event
`LedgerEvent` får en ny variant. Befintliga event ändras inte, så `schema_version` förblir 1.
```rust
/// A file backing up a voucher (BFL 5 kap. 6–7 §). Never removed.
AttachmentAdded { voucher: u32, attachment: Attachment },
```

### Tillstånd
`Voucher` får `attachments: Vec<Attachment>` i den ordning de lades till. `Ledger::apply` lägger till underlaget på verifikationen.

### Bokföring med underlag
`RecordVoucher` och `domain::record_voucher` ändras inte. Lib-lagret bokför verifikationen och lägger sedan till varje underlag med `add_attachment`, i samma `BEGIN IMMEDIATE`-transaktion. Varje underlag blir ett eget `AttachmentAdded` direkt efter `VoucherRecorded`. Om något underlag avvisas, till exempel samma fil två gånger (`DuplicateAttachment`), rullas hela bokföringen tillbaka. `correct_voucher` tar inga underlag.

### `add_attachment(ledger, voucher, attachment)`
- Om verifikationen saknas blir det `VoucherNotFound`.
- Om hashen redan finns på verifikationen blir det `DuplicateAttachment`.
- Ett stängt år kontrolleras **inte**.
- Funktionen returnerar `AttachmentAdded`.

## Lagring och transaktioner

### Migration `migrations/0008_attachments.sql`
```sql
-- Underlag (räkenskapsinformation, BFL 7 kap.). Primary data, like events:
-- not a projection, never updated or deleted.
CREATE TABLE attachment_files (
    sha256 TEXT    PRIMARY KEY,
    size   INTEGER NOT NULL,
    data   BLOB    NOT NULL
);
CREATE TRIGGER attachment_files_no_update BEFORE UPDATE ON attachment_files
BEGIN SELECT RAISE(ABORT, 'attachment files are append-only'); END;
CREATE TRIGGER attachment_files_no_delete BEFORE DELETE ON attachment_files
BEGIN SELECT RAISE(ABORT, 'attachment files are append-only'); END;

-- Projection of AttachmentAdded. Rebuildable from events.
CREATE TABLE voucher_attachments (
    company_id        TEXT    NOT NULL,
    fiscal_year_start TEXT    NOT NULL,
    number            INTEGER NOT NULL,
    position          INTEGER NOT NULL,
    sha256            TEXT    NOT NULL REFERENCES attachment_files (sha256),
    file_name         TEXT    NOT NULL,
    content_type      TEXT    NOT NULL,
    size              INTEGER NOT NULL,
    added_at          TEXT    NOT NULL,
    added_by          TEXT    NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start, number, position),
    UNIQUE (company_id, fiscal_year_start, number, sha256),
    FOREIGN KEY (company_id, fiscal_year_start, number)
        REFERENCES vouchers (company_id, fiscal_year_start, number)
);
```

### Skrivflöde (`crates/ledger/src/lib.rs`)
- `NewAttachment { file_name: String, data: Vec<u8> }` är indata från API:t. `add_attachment_in` räknar SHA-256, anropar `Attachment::new` och `domain::add_attachment`, gör `INSERT OR IGNORE INTO attachment_files` och sedan `append`.
- `record_voucher_with_attachments(pool, company_id, actor, cmd, attachments, today)` kör `record_voucher_in` och därefter `add_attachment_in` per underlag i samma `BEGIN IMMEDIATE`. Om något misslyckas rullas allt tillbaka, och då blir varken någon fil eller något nummer kvar. `record_voucher` finns kvar oförändrad.
- `add_attachment(pool, company_id, actor, fiscal_year_start, number, NewAttachment, today) -> Result<Attachment>` följer samma mönster som `correct_voucher`: `member_company`, `fiscal_year_at` (ett år som saknas ger `VoucherNotFound`), `load_ledger`, `domain::add_attachment`, insert av filen och `append`.
- Projektionen (`projections.rs`) lägger in en rad i `voucher_attachments` för varje `AttachmentAdded`, med `position` = antalet befintliga rader + 1. `added_at` och `added_by` kommer från eventets metadata.
- `rebuild_projections` tömmer och bygger om `voucher_attachments`. `attachment_files` rörs inte.

### Läsningar (`queries.rs`)
- `list_vouchers` tar med varje verifikations underlag (metadata, inte bytes).
- `get_attachment(conn, company_id, actor, fiscal_year_start, number, sha256) -> Result<(Attachment, Vec<u8>)>` kontrollerar medlemskap och gör en join från `voucher_attachments` till `attachment_files` på hela nyckeln. Om ingen rad finns blir det `AttachmentNotFound`.

## API (`proto/doris/ledger/v1/ledger.proto`, `LedgerService`)
```proto
message NewAttachment {
  string file_name = 1;
  bytes data = 2;
}

message Attachment {
  string id = 1;           // sha256, hex
  string file_name = 2;
  string content_type = 3; // application/pdf, image/jpeg, image/png
  uint64 size = 4;
}

// RecordVoucherRequest: repeated NewAttachment attachments = 5;
// Voucher:              repeated Attachment attachments = 7;

rpc AddAttachment(AddAttachmentRequest) returns (AddAttachmentResponse);
message AddAttachmentRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
  uint32 number = 3;
  NewAttachment attachment = 4;
}
message AddAttachmentResponse { Attachment attachment = 1; }

rpc GetAttachment(GetAttachmentRequest) returns (GetAttachmentResponse);
message GetAttachmentRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
  uint32 number = 3;
  string id = 4;
}
message GetAttachmentResponse {
  Attachment attachment = 1;
  bytes data = 2;
}
```

### Server (`crates/server`)
- `LedgerServiceServer::new(..)` får `.max_decoding_message_size(21 MiB)` och `.max_encoding_message_size(11 MiB)`. De andra tjänsterna behåller tonics standard (4 MiB).
- `ledger.rs` kontrollerar att summan av `data` i `RecordVoucher` är högst 20 MiB, annars `attachment_too_large`.
- Nya koder: `unsupported_attachment_type`, `invalid_attachment_name`, `empty_attachment`, `attachment_too_large` och `duplicate_attachment` mappas till `InvalidArgument`. `attachment_not_found` mappas till `NotFound`.
- Filnamn loggas aldrig, eftersom de kan innehålla personuppgifter.

## Frontend

### `/vouchers/new`, Ny verifikation
- Fältet "Underlag" ligger under Text. Det är en synlig, nativ filväljare (komponenten `FileInput` i `ui.rs`): `<input type="file" multiple accept="application/pdf,image/jpeg,image/png">`. På mobilen erbjuder det kameran.
- De valda filerna listas med namn och storlek, var och en med knappen "Ta bort". Filer som väljs vid flera tillfällen läggs till i listan.
- Innan filerna skickas kontrollerar klienten 10 MiB per fil och 20 MiB totalt. Felet visas med samma text som koden `attachment_too_large`.
- Byten läses med `File.arrayBuffer()` och skickas i `RecordVoucherRequest.attachments`.

### `/vouchers`, Verifikationer (grundboken)
- En ny smal kolumn visar lucide-ikonen `paperclip` (inlinad SVG) och antalet underlag. Den är tom för verifikationer utan underlag.
- När en rad expanderas visas rubriken "Underlag" under konteringsraderna, med en lista av knappar i formen "kvitto.pdf (1 kB)". Det är knappar och inte länkar, eftersom de saknar `href`. Storleken anges i kB, eller i MB med en decimal från 1 MB.
- Ett klick anropar `GetAttachment`. Svaret blir en `Blob` med `content_type` från servern och öppnas via `URL.createObjectURL` med `window.open(url, "_blank")`. URL:en släpps med `revokeObjectURL` efter en kort fördröjning.
- I den expanderade vyn finns också filväljaren "Lägg till underlag till ver N", som anropar `AddAttachment` en gång per vald fil och sedan läser om listan. Den visas även för stängda år.

### Övrigt
- `web-sys` får features för `File`, `FileList`, `HtmlInputElement`, `Blob`, `BlobPropertyBag` och `Url`.
- `src/errors.rs` får en svensk text per ny kod.
  - `unsupported_attachment_type`: "Underlaget måste vara en PDF, JPEG eller PNG."
  - `invalid_attachment_name`: "Filnamnet är ogiltigt."
  - `empty_attachment`: "Filen är tom."
  - `attachment_too_large`: "Underlaget är för stort (högst 10 MB per fil och 20 MB totalt)."
  - `duplicate_attachment`: "Underlaget finns redan på verifikationen."
  - `attachment_not_found`: "Underlaget hittades inte."

## Tester
Varje beteende utvecklas med TDD: rött, grönt, refaktorering och commit.

### Domän (`crates/ledger/tests/domain.rs`)
- `sniff` känner igen PDF, JPEG och PNG och avvisar GIF, text och tom indata.
- `Attachment::new`: en fil på exakt 10 MiB går igenom och en byte till ger `AttachmentTooLarge`. En tom fil ger `EmptyAttachment`. Namnregler: tomt namn, 256 tecken, `/`, `\` och `\n`.
- `add_attachment`: en verifikation som saknas ger `VoucherNotFound`, och en hash som redan finns ger `DuplicateAttachment`. Givet ett stängt år lyckas det.
- `evolve` ger `Voucher.attachments` i ordningen de lades till.

### Lagring (`crates/ledger/tests/store.rs`)
- Samma fil på två verifikationer ger en rad i `attachment_files` och två i `voucher_attachments`.
- `UPDATE` och `DELETE` på `attachment_files` avvisas.
- En bokföring med två underlag ger `VoucherRecorded` följt av två `AttachmentAdded`.
- En bokföring som inte balanserar men har underlag, en som har en GIF eller en som har samma fil två gånger lämnar både `events` och `attachment_files` tomma.
- `rebuild_projections` från `read_all` ger samma `voucher_attachments`.
- `get_attachment` med rätt hash men ett annat företags verifikation ger `AttachmentNotFound`. Den som inte är medlem får `Error::NotFound`, som för andra läsningar.
- `add_attachment` i ett stängt år lyckas.
- `crates/ledger/tests/stress.rs` går fortfarande igenom.

### Server (gRPC-Web-integrationstester)
- `RecordVoucher` med en PDF på 10 MiB går igenom. 10 MiB + 1 byte ger `attachment_too_large`.
- Två filer på 10 MiB i samma anrop går igenom. Två på 10 MiB plus en fil på 1 byte (20 MiB + 1, under gränsen på 21 MiB) ger `attachment_too_large`.
- `GetAttachment` returnerar exakt samma bytes och `application/pdf`.
- En GIF ger `unsupported_attachment_type`.

### E2E (`e2e/`)
- Bokför en verifikation med en liten PDF via fältet "Underlag". Raden i grundboken visar gemet och "1".
- Expandera raden och klicka på filnamnet. En ny sida öppnas med en `blob:`-URL.
- "Lägg till underlag" på en befintlig verifikation, även efter att året har stängts.
- En GIF ger "Underlaget måste vara en PDF, JPEG eller PNG."

### Avslutning
- `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
- `make dist` håller sig inom `WASM_BUDGET`.
- AGENTS.md uppdateras med de nya koderna, `attachment_files` som append-only primärdata och regeln om underlag i stängda år.

## Utanför omfattningen
- Inkorg för kvitton som ännu inte är kopplade till en verifikation
- OCR och förifyllning av verifikationen
- Krav på eller varning för verifikationer utan underlag
- Att byta namn på eller ta bort underlag
- Miniatyrbilder
- Underlag i SIE-exporten
- Andra filtyper (HEIC, XML-/Peppol-fakturor, e-post)
