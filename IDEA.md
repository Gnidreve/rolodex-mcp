# Idee: Weitere Outbound-Kanäle neben E-Mail

> **Update:** Telegram ist inzwischen umgesetzt (Branch `telegram`,
> `src/telegram.rs` + `telegram_chat_id` in `config.toml`) — der
> "empfohlene erste Schritt" unten ist damit erledigt. Der Rest dieses
> Dokuments (ntfy.sh/Pushover als zweiter Kanal, alles andere) ist weiterhin
> offen.

Status: Brainstorming/Recherche für die restlichen Kanäle. Ziel: das bestehende
Prinzip von `sendmail-mcp` (pro Kontakt ein fest konfiguriertes Tool, reiner
Outbound-Versand, kein generisches "sende an beliebige Adresse"-Tool) auf
weitere Kanäle ausweiten, nicht nur E-Mail.

## Warum das architektonisch passt

Das Kernprinzip von `sendmail-mcp` ist kanal-agnostisch:

- Kontaktbuch (`config.toml`) definiert **wer** erreichbar ist.
- Für jeden Kontakt entsteht **ein eigenes Tool** (`send_<kanal>_to_<name>`).
- Der Agent sieht nie eine rohe Adresse/ID, nur den Tool-Namen.
- Zugangsdaten (SMTP-Credentials bisher) kommen separat aus ENV, nie aus dem
  Kontaktbuch.

Das lässt sich 1:1 auf andere Kanäle übertragen, solange der Kanal
**reinen One-Way-Outbound-Versand an eine vorher bekannte Adresse/ID**
unterstützt — also kein Channel, der ein Gespräch/Session-Handling braucht.

## Offene Design-Frage (bevor irgendwas gebaut wird)

Zwei Möglichkeiten, das Kontaktbuch zu erweitern:

1. **Ein Kanal pro Kontakt**, wie bisher — `config.toml` bekommt pro Kontakt
   ein `channel`-Feld (`email` | `telegram` | `discord` | ...) plus die
   kanal-spezifische Adresse. Tool bleibt `send_message_to_<name>`, das
   Backend routet intern zum richtigen Kanal.
2. **Mehrere Kanäle pro Kontakt** — ein Kontakt kann z.B. sowohl `email` als
   auch `telegram` hinterlegt haben, es entsteht dann pro Kontakt *und*
   Kanal ein eigenes Tool (`send_email_to_max`, `send_telegram_to_max`).

Variante 2 passt besser zum bisherigen "explizite Tool-Liste"-Prinzip
(Agent sieht klar, über welchen Kanal er etwas verschickt), bräuchte aber
eine Erweiterung des `config.toml`-Schemas von `[[to]]` (flach) zu
verschachtelten Kanal-Einträgen pro Kontakt.

## Kandidaten

### Sehr guter Fit (einfache API, kein OAuth, reiner Outbound-Send)

| Kanal | Mechanismus | Auth | Selbst hostbar? | Notiz |
|---|---|---|---|---|
| **Telegram** | Bot API, `sendMessage` | Bot-Token (via BotFather) | Nein (Telegram-Server) | Empfänger muss den Bot einmal gestartet haben, dann feste `chat_id`. Sehr stabile, gut dokumentierte API, kostenlos. |
| **Discord** | Incoming Webhook (pro Channel eine URL) | Webhook-URL selbst ist das Secret | Nein (Discord-Server) | Kein Bot-Setup nötig, einfacher POST-Request. Passt aber eher zu "Nachricht in einen Channel" als "an eine Person" — für 1:1 bräuchte es stattdessen einen echten Bot + DM-Kanal. |
| **Slack** | Incoming Webhook oder `chat.postMessage` (Bot-Token) | Webhook-URL bzw. Bot-Token | Nein (Slack-Server) | Wie Discord: Webhook = Channel, für DMs an eine Person braucht es die Bot-Token-Variante mit User-ID. |
| **Pushover** | Ein POST (`user`-Key + `api_token` + Nachricht) | API-Token + User-Key | Nein | Explizit für genau diesen Anwendungsfall gebaut ("sende mir eine Push-Notification"). Kleiner Jahrespreis pro Nutzer (einmalig, kein Abo). Sehr simpel zu integrieren. |
| **ntfy.sh** | POST auf `https://ntfy.sh/<topic>` (oder eigene Instanz) | Optional (Topic ist quasi das Secret, oder eigener Access-Token bei selbst gehosteter Instanz) | **Ja** | Open Source, kein Account nötig für den öffentlichen Dienst. Passt gut zum Coolify-Self-Hosting-Ansatz aus diesem Projekt. |
| **Gotify** | POST an eigene Gotify-Instanz mit App-Token | App-Token | **Ja** | Wie ntfy, aber man hostet den Server komplett selbst (z.B. auch via Coolify). Etwas mehr Betriebsaufwand als ntfy. |

### Mittlerer Fit (funktioniert, aber mehr Aufwand/Bürokratie)

| Kanal | Mechanismus | Auth | Notiz |
|---|---|---|---|
| **SMS via Twilio/Vonage/MessageBird** | REST-API, eine Nachricht pro Call | API-Key + Account-ID | Erreicht jeden mit Handynummer, kein App-Zwang beim Empfänger. Kostet pro Nachricht, Twilio-Nummer muss gemietet werden. |
| **Matrix** | Client-Server-API, `PUT /rooms/{roomId}/send/...` | Access-Token eines Bot-Accounts | Selbst hostbar (eigener Homeserver), aber pro Kontakt braucht es einen gemeinsamen Room + dessen Room-ID vorab. Setup-Aufwand höher als Telegram. |
| **Signal** | Kein offizielles Bot-API — nur über `signal-cli` (inoffiziell) bzw. dessen REST-Wrapper (`signal-cli-rest-api`, Docker-Image) | Telefonnummer-Registrierung des Bot-Accounts | Attraktiv für privacy-fokussierte Nutzer, aber technisch der komplizierteste Kandidat hier (eigener Sidecar-Container, Geräte-Verifizierung). |
| **WhatsApp Business Cloud API** | Meta Graph API | Meta Business-Account + Verifizierung, Access-Token | Erreicht extrem viele Leute, aber: Meta-Business-Verifizierung nötig, außerhalb eines 24h-Antwortfensters müssen vorab genehmigte Nachrichten-Templates verwendet werden — für spontane 1:1-Nachrichten unpraktisch. |
| **Microsoft Teams** | Power-Automate-HTTP-Trigger (Incoming Webhooks werden von Microsoft abgekündigt) | Workflow-URL | Im Umbruch (Microsoft deprecatet die klassischen Webhooks), API-Fläche aktuell nicht stabil genug für eine Empfehlung. |

### Kein guter Fit für dieses Muster

- **Mastodon / Bluesky**: posten öffentlich (Broadcast), nicht direkt an eine
  Person adressiert — passt nicht zum "ein Tool pro Kontakt"-Modell.
- Alles mit **OAuth-User-Flow** (z.B. Nachrichten im Namen eines Nutzers
  über dessen eigenen Google/Microsoft-Account): würde eine
  Redirect-basierte Autorisierung pro Kontakt brauchen — passt nicht zum
  bisherigen "einmal ENV setzen, fertig"-Betriebsmodell.

## Generischer Fallback-Kanal: Webhook

Unabhängig von einzelnen Plattformen wäre ein generischer
`send_webhook_to_<name>`-Kanal denkbar: pro Kontakt eine feste URL + Payload
(JSON), die z.B. an n8n, Zapier, IFTTT, Home Assistant oder irgendeinen
eigenen Endpunkt geht. Deckt implizit sehr viele Plattformen ab, ohne dass
für jede einzeln Code geschrieben werden muss — auf Kosten davon, dass die
Konfiguration dann kanal-spezifisches Wissen beim Nutzer voraussetzt
(welche URL/welches Payload-Format der Zielservice erwartet).

## Empfehlung für einen ersten Schritt (falls das umgesetzt wird)

1. **Telegram** zuerst — bester Kompromiss aus Reichweite, einfacher API,
   keine Kosten, kein Server-Betrieb nötig zusätzlich zu unserem eigenen.
2. **ntfy.sh oder Pushover** als zweiter Kanal — deckt den "einfache
   Push-Benachrichtigung an mich selbst"-Fall ab, minimal-invasiv zu bauen.
3. Alles andere (Discord/Slack-Webhooks, SMS, Matrix, Signal, WhatsApp) erst
   bei konkretem Bedarf, da jeweils eigene Trade-offs (Broadcast- statt
   DM-Semantik, Kosten, Setup-Aufwand, API-Stabilität).

## Nicht Teil dieser Idee

- Kein Empfang/Antworten auf Nachrichten (bleibt reiner Outbound-Versand,
  wie bei E-Mail auch).
- Keine generischen "sende an beliebige Adresse"-Tools — das
  Sicherheitsprinzip (Agent kennt nur Namen, nie rohe Adressen/IDs) gilt
  für jeden neuen Kanal genauso wie für E-Mail.
