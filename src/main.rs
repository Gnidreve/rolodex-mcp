mod auth;
mod config;
mod logging;
mod mcp_server;
mod smtp;
mod telegram;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use axum::middleware;
use axum::routing::{get, post_service};
use axum::Json;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::json;
use tracing_subscriber::EnvFilter;

use crate::config::{load_contacts, Channel};
use crate::mcp_server::SendMailServer;
use crate::smtp::SmtpConfig;
use crate::telegram::TelegramConfig;

/// `[2026-09-11 07:52:45]` statt tracing_subscribers Default
/// (`2026-09-11T07:52:45.251010Z`) - besser lesbar in Coolifys Log-Viewer.
struct BracketedUtcTime;

impl tracing_subscriber::fmt::time::FormatTime for BracketedUtcTime {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "[{}]", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Ohne RUST_LOG würde tracing_subscriber defaultmäßig nur ERROR loggen —
    // damit wären auch die info!()-Zeilen unten (Kontakte geladen, Requests)
    // unsichtbar. rmcp=warn unterdrückt rmcps eigenes internes Session-/
    // Response-Logging (Debug-Dump jeder Antwort) im Normalbetrieb - bei
    // Bedarf zieht RUST_LOG=debug (o.ä.) das wieder hoch.
    tracing_subscriber::fmt()
        .with_timer(BracketedUtcTime)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,rmcp=warn")))
        .init();

    let config_path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| "/app/config.toml".into());
    let tools = load_contacts(&PathBuf::from(&config_path))
        .with_context(|| format!("Kontaktliste konnte nicht geladen werden: {config_path}"))?;

    tracing::info!(count = tools.len(), "Tools geladen");
    for t in &tools {
        tracing::info!(tool = %t.tool_name, name = %t.contact_name, "Tool registriert");
    }

    // SMTP/Telegram werden nur geladen (und ihre Pflicht-ENV-Variablen nur
    // verlangt), wenn das Kontaktbuch den jeweiligen Kanal tatsächlich
    // nutzt - eine reine Telegram-Config soll keinen SMTP_HOST brauchen
    // und umgekehrt.
    let uses_email = tools.iter().any(|t| t.channel == Channel::Email);
    let uses_telegram = tools.iter().any(|t| t.channel == Channel::Telegram);

    let smtp = if uses_email {
        let smtp = SmtpConfig::from_env().context("SMTP-Konfiguration unvollständig (siehe ENV-Variablen)")?;
        tracing::info!("Prüfe SMTP-Verbindung...");
        smtp.test_connection()
            .await
            .context("SMTP-Verbindung fehlgeschlagen - Server startet nicht")?;
        tracing::info!("SMTP-Verbindung OK");
        Some(smtp)
    } else {
        None
    };

    let telegram = if uses_telegram {
        let telegram = TelegramConfig::from_env()
            .context("config.toml enthält telegram_chat_id-Kontakte, aber TELEGRAM_BOT_TOKEN ist nicht gesetzt")?;
        tracing::info!("Prüfe Telegram-Verbindung...");
        telegram
            .test_connection()
            .await
            .context("Telegram-Verbindung fehlgeschlagen - Server startet nicht")?;
        tracing::info!("Telegram-Verbindung OK");
        Some(telegram)
    } else {
        None
    };

    let bearer_token = std::env::var("MCP_BEARER_TOKEN")
        .context("Pflicht-ENV-Variable MCP_BEARER_TOKEN ist nicht gesetzt")?;
    if !bearer_token.is_ascii() {
        // HTTP-Header-Werte müssen ASCII sein. Mit einem nicht-ASCII-Token
        // setzen viele HTTP-Clients den Authorization-Header gar nicht erst
        // (statt eines Fehlers) - der Server lehnt dann jeden Request mit
        // "Header fehlt" ab, was wie ein Client-Bug aussieht, aber ein
        // ungültiger Token war. Lieber hier hart und sofort abbrechen.
        bail!("MCP_BEARER_TOKEN enthält nicht-ASCII-Zeichen - HTTP-Header dürfen nur ASCII sein");
    }

    let server = SendMailServer::new(tools, smtp, telegram);

    // Streamable HTTP ist der von rmcp empfohlene HTTP-Transport (ersetzt das
    // alte zweigeteilte HTTP+SSE-Schema). Der Prozess selbst spricht nur
    // HTTP — TLS/HTTPS wird über einen vorgeschalteten Reverse Proxy
    // (Traefik/Caddy/nginx) terminiert, wie bei Docker-Compose-Deployments üblich.
    //
    // stateful_mode: false, weil wir GET auf derselben Route als Healthcheck
    // brauchen. Bei stateful_mode: true reserviert rmcp GET für die
    // SSE-Session-Resumption; da wir GET selbst bedienen, würde das nie
    // greifen und rmcp bräuchte ohnehin ein Session-Handling, das dieser
    // einfache, zustandslose Sendmail-Server nicht braucht.
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig {
            stateful_mode: false,
            ..Default::default()
        },
    );

    // Alles auf "/": GET liefert einen ungeschützten Healthcheck, POST ist
    // der eigentliche MCP-Endpunkt und verlangt den Bearer-Token.
    let health = get(|| async { Json(json!({ "status": "ok" })) });
    let mcp_route =
        post_service(service).layer(middleware::from_fn_with_state(bearer_token, auth::require_bearer_token));

    let app = axum::Router::new()
        .route("/", health.merge(mcp_route))
        .layer(middleware::from_fn(logging::log_requests));

    let bind_addr: SocketAddr = std::env::var("MCP_BIND")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .context("MCP_BIND ist keine gültige Socket-Adresse")?;

    tracing::info!(%bind_addr, "sendmail-mcp startet");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}
