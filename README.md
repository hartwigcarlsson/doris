# Doris

Doris är ett bokföringsprogram för svenska företag, byggt för att följa
[Bokföringslagen (SFS 1999:1078)](https://www.riksdagen.se/sv/dokument-och-lagar/dokument/svensk-forfattningssamling/bokforingslag-19991078_sfs-1999-1078/).

## Kom igång

Med Docker (image på `ghcr.io/hartwigcarlsson/doris`):

```sh
docker compose up --build
```

Öppna <http://localhost:3000>. Den första användaren som registrerar sig blir
admin; därefter krävs en inbjudan från en admin.

## AI-agenter (MCP)

Doris är en MCP-server på `/mcp`. Skapa en API-token under API-tokens i
kontomenyn, med så få behörigheter som möjligt, och anslut till exempel
Claude Code:

    claude mcp add --transport http doris https://doris.example.se/mcp \
      --header "Authorization: Bearer doris_…"

Verktygen är desamma som doris-cli:s kommandon. Connectors i claude.ai
och Claude Desktop kräver OAuth och stöds inte än.

## Utveckling

Krav: Rust med target `wasm32-unknown-unknown`, [Trunk](https://trunk-rs.github.io/trunk/),
`protoc` och systemets OpenSSL (`brew install openssl@3` på macOS,
`libssl-dev` på Debian/Ubuntu). Node behövs bara för e2e-testerna.

```sh
make dev       # server på :3000 + trunk serve på :8080 (öppna http://localhost:8080)
make test      # cargo test --workspace
make e2e       # Playwright mot debug-servern
make dist      # target/dist/doris (frontend inbäddad) + frontend som tarball
make e2e-dist  # Playwright mot release-binären
```

Använd `localhost`, inte `127.0.0.1`: WebAuthn och `Secure`-cookies kräver det
över vanlig http.

## Konfiguration

Miljövariabler (eller motsvarande CLI-flaggor, se `doris --help`):

| Variabel | Standard | Beskrivning |
|---|---|---|
| `DORIS_DATABASE` | `sqlite://doris.db` | SQLite-databasens URL |
| `DORIS_LISTEN` | `127.0.0.1:3000` | Adress att lyssna på (`0.0.0.0:3000` i container) |
| `DORIS_RP_ID` | `localhost` | WebAuthn RP-id: domänen, utan schema och port |
| `DORIS_RP_ORIGIN` | `http://localhost:3000` | Origin som sidan faktiskt laddas från |
| `DORIS_CORS_ORIGINS` | - | Andra origins som får anropa API:t, kommaseparerade |
| `DORIS_SERVE_FRONTEND` | `true` | Servera den inbäddade frontenden |

## Arkitektur

| Lager | Val |
|---|---|
| Backend | Rust, tonic + tonic-web (gRPC-Web) |
| Frontend | Leptos CSR (WASM), Trunk, Tailwind v4 |
| Lagring | SQLite via sqlx |
| Modell | Event sourcing med projektioner för läsning |

```
proto/              .proto-filer (doris.<område>.v1)
migrations/         sqlx-migreringar
crates/eventstore   append-only-händelselogg
crates/identity     användare, passkeys, inbjudningar, sessioner
crates/proto        genererad gRPC-kod
crates/server       binären: gRPC-tjänster + inbäddad frontend
crates/web          Leptos-appen
e2e/                Playwright-tester
```

Händelseloggen är append-only på databasnivå, och händelser sparas som JSON så
att de förblir läsbara under hela arkiveringstiden.

## Licens

[MIT](LICENSE)
