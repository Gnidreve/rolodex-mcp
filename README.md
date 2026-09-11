# sendmail-mcp

Ein MCP-Server, der für jeden Kontakt aus `config.toml` **und Kanal**
(E-Mail, Telegram) ein eigenes Tool erzeugt (`send_to_<name>_via_email`,
`send_to_<name>_via_telegram`). Der Agent sieht nur Namen und Kanal über den
Tool-Namen, nie die tatsächliche Adresse/Chat-ID — es gibt kein generisches
Tool mit freier Adress- oder Kanalwahl. Name zuerst, Kanal als Suffix: so
bleiben die Tools eines Kontakts auch in alphabetisch sortierenden Clients
nebeneinander. Zusätzlich trägt jedes Tool einen menschenlesbaren `title`
(z.B. "Max Mustermann — Telegram") für Clients, die das MCP-`title`-Feld
anzeigen.

## Architektur

- `config.toml` (per Volume gemountet) = **nur** Kontaktbuch. Ein Kontakt
  kann mehrere Kanäle gleichzeitig haben — dann entstehen mehrere Tools:
  ```toml
  [[to]]
  name = "Max Mustermann"
  email = "max@example.com"
  telegram_chat_id = "123456789"
  ```
  Siehe `config.example.toml` für alle Details (Gruppen-Chat-IDs, wie man
  seine Telegram-Chat-ID herausfindet, etc.).
- `.env` (per `env_file`) = Zugangsdaten pro Kanal: SMTP (Host, Port,
  Encryption, Credentials, Absenderadresse) und/oder `TELEGRAM_BOT_TOKEN`.
  Siehe `.env.example`. Ein Kanal ist nur Pflicht, wenn ihn mindestens ein
  Kontakt in `config.toml` nutzt — reines Telegram braucht kein SMTP und
  umgekehrt.
- Beim Start wird die Kontaktliste geparst. Zwei Namen, die auf denselben
  Tool-Namen ("Slug") abbilden würden, führen zu einem **harten
  Startabbruch** mit klarer Fehlermeldung — kein Fuzzy-Matching, keine
  stille Kollisionsauflösung. Ein Kontakt ganz ohne Kanal ist ebenfalls ein
  Startabbruch.
- Jeder benötigte Kanal wird beim Start einmal real geprüft (SMTP-Verbindung
  bzw. Telegram `getMe`) — schlägt das fehl, startet der Server gar nicht
  erst, mit der echten Fehlerursache im Log, statt erst beim ersten
  Sendeversuch aufzufallen.
- Jedes Tool erwartet `subject` (string) und `body` (string, Klartext) —
  bei Telegram wird `subject` als fette erste Zeile vor `body` gesetzt,
  da der Kanal keinen separaten Betreff kennt.
- Transport: Streamable HTTP direkt auf `/` (nicht `/mcp`):
  - `GET /` — ungeschützter Healthcheck, liefert `{"status":"ok"}`. Kein
    Bearer-Token nötig, damit Docker/Coolify ihn erreichen können.
  - `POST /` — der eigentliche MCP-Endpunkt. Erfordert den Header
    `Authorization: Bearer <MCP_BEARER_TOKEN>` (siehe `src/auth.rs`), sonst
    `401 Unauthorized`. Der Wert kommt aus der Pflicht-ENV-Variable
    `MCP_BEARER_TOKEN` (siehe `.env.example`) — zum Rotieren dort ändern
    und neu deployen.
  - Der Server läuft mit `stateful_mode: false` (kein Session-Handling,
    kein `Mcp-Session-Id`), weil GET auf derselben Route bereits für den
    Healthcheck reserviert ist — rmcp würde GET sonst für
    SSE-Session-Resumption brauchen.
- Der Prozess selbst terminiert kein TLS — in Produktion Reverse Proxy
  davorsetzen (bei Coolify übernimmt das dessen Traefik-Instanz).

## Logging

Standardmäßig (ohne `RUST_LOG` gesetzt) läuft der Server auf `INFO`-Level,
nicht auf `ERROR` (was `tracing_subscriber`'s Default wäre) — sonst wären
weder die Startup-Logs noch die Request-Logs sichtbar. Bei jedem Request
gegen `/` loggt eine `tower-http`-`TraceLayer` Methode, Pfad, Statuscode und
Latenz; abgelehnte Bearer-Token loggen zusätzlich den Grund (fehlender vs.
falscher Header — der Token-Wert selbst landet nie im Log). Für mehr Detail
(inkl. `rmcp`s eigenem Request/Response-Tracing) `RUST_LOG=debug` setzen.

## Lokal bauen & prüfen

```bash
cargo check
cargo test        # prüft u.a. die Umlaut-Slugifizierung und Kollisionserkennung
cargo run
```

`CONFIG_PATH` (Default `/app/config.toml`) und die `SMTP_*`-Variablen
müssen gesetzt sein, z.B.:

```bash
cp config.example.toml config.toml
cp .env.example .env
# .env ausfüllen
export $(cat .env | xargs)
export CONFIG_PATH=./config.toml
cargo run
```

## Docker

```bash
cp config.example.toml config.toml   # anpassen
cp .env.example .env                 # SMTP-Zugangsdaten eintragen
docker compose up --build
```

Der Server lauscht dann auf `http://<host>:8080/` (Streamable HTTP, MCP via
POST mit Bearer-Token; GET liefert den Healthcheck). Ein eingebauter
Docker-Healthcheck (`curl -f http://localhost:8080/`) ist in
`docker-compose.yml` hinterlegt.

## Erweiterungsideen (bewusst nicht gebaut, minimal-invasiv gehalten)

- Hot-Reload der `config.toml` ohne Neustart.
- Mehrere Empfänger pro Aufruf / CC.
- Direkte TLS-Terminierung im Rust-Prozess statt Reverse Proxy.
- `SMTP_ACCEPT_INVALID_CERTS` für selbstsignierte interne Mailserver.
- Weitere Kanäle (Discord, Slack, ntfy.sh, SMS, ...) — siehe `IDEA.md` für
  eine Bewertung, welche Kandidaten zum "ein Tool pro Kontakt+Kanal,
  Agent sieht nie Rohdaten"-Prinzip passen.
