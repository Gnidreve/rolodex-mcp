use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::json;

/// Bot-Token kommt aus ENV, nie aus config.toml - config.toml bleibt
/// ausschließlich das Kontaktbuch (siehe config.rs).
#[derive(Debug, Clone)]
pub struct TelegramConfig {
    pub bot_token: String,
}

impl TelegramConfig {
    /// `None`, wenn TELEGRAM_BOT_TOKEN nicht gesetzt ist - der Aufrufer
    /// entscheidet anhand des Kontaktbuchs, ob das ein Fehler ist (nämlich
    /// genau dann, wenn mindestens ein Kontakt telegram_chat_id nutzt).
    pub fn from_env() -> Option<Self> {
        non_empty(std::env::var("TELEGRAM_BOT_TOKEN").ok()).map(|bot_token| Self { bot_token })
    }

    /// Prüft den Bot-Token einmal gegen die Telegram-API (`getMe`), ohne
    /// eine Nachricht zu verschicken. Für den Startup-Check gedacht, analog
    /// zu SmtpConfig::test_connection.
    pub async fn test_connection(&self) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{}/getMe", self.bot_token);
        let response = reqwest::get(&url)
            .await
            .context("Telegram-Verbindungstest fehlgeschlagen (Netzwerk)")?;
        let status = response.status();
        let body: TelegramApiResponse<serde_json::Value> = response
            .json()
            .await
            .context("Telegram-Antwort auf getMe nicht lesbar")?;

        if !status.is_success() || !body.ok {
            bail!(
                "Telegram-Verbindungstest fehlgeschlagen: {}",
                body.description.unwrap_or_else(|| format!("HTTP {status}"))
            );
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct TelegramApiResponse<T> {
    ok: bool,
    #[serde(default)]
    description: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    result: Option<T>,
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub async fn send_telegram(cfg: &TelegramConfig, chat_id: &str, subject: &str, body: &str) -> Result<()> {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", cfg.bot_token);
    // Telegram kennt keinen separaten Betreff - subject wird als fette
    // erste Zeile vor den eigentlichen Text gesetzt.
    let text = format!("*{}*\n\n{}", escape_markdown(subject), escape_markdown(body));

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .json(&json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "MarkdownV2",
        }))
        .send()
        .await
        .context("Telegram-Versand fehlgeschlagen (Netzwerk)")?;

    let status = response.status();
    let body: TelegramApiResponse<serde_json::Value> =
        response.json().await.context("Telegram-Antwort nicht lesbar")?;

    if !status.is_success() || !body.ok {
        bail!(
            "Telegram-Versand fehlgeschlagen: {}",
            body.description.unwrap_or_else(|| format!("HTTP {status}"))
        );
    }
    Ok(())
}

/// MarkdownV2 verlangt, dass eine feste Menge an Sonderzeichen escaped wird,
/// sonst lehnt die Telegram-API die Nachricht komplett ab.
/// https://core.telegram.org/bots/api#markdownv2-style
fn escape_markdown(text: &str) -> String {
    const SPECIAL: &[char] = &[
        '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ];
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if SPECIAL.contains(&ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markdown_special_chars() {
        assert_eq!(escape_markdown("Hallo (Welt)!"), r"Hallo \(Welt\)\!");
        assert_eq!(escape_markdown("100% fertig."), r"100% fertig\.");
    }
}
