use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// Rohformat der config.toml: nur das Kontaktbuch, sonst nichts.
/// Zugangsdaten (SMTP, Telegram-Bot-Token) kommen bewusst NICHT hier rein,
/// sondern aus ENV — Trennung von "wer darf angeschrieben werden" und
/// "womit wird verschickt".
#[derive(Debug, Clone, Deserialize)]
struct RawConfig {
    #[serde(rename = "to", default)]
    to: Vec<ToEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct ToEntry {
    name: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    telegram_chat_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Email,
    Telegram,
}

impl Channel {
    fn slug(self) -> &'static str {
        match self {
            Channel::Email => "email",
            Channel::Telegram => "telegram",
        }
    }

    /// Für den menschenlesbaren Tool-Titel (nicht den Tool-Namen selbst).
    fn display_name(self) -> &'static str {
        match self {
            Channel::Email => "E-Mail",
            Channel::Telegram => "Telegram",
        }
    }
}

/// Ein Kontakt kann mehrere Kanäle gleichzeitig haben (z.B. E-Mail UND
/// Telegram). Pro Kontakt UND Kanal entsteht ein eigenes, konkretes Tool -
/// kein generisches "sende Nachricht", weil ein Kontakt sonst mehrdeutig
/// über mehr als einen Kanal ansprechbar wäre, ohne dass der Agent das im
/// Tool-Namen sehen könnte.
#[derive(Debug, Clone)]
pub struct ChannelTool {
    pub contact_name: String,
    pub channel: Channel,
    /// E-Mail-Adresse bzw. Telegram-Chat-ID, je nach `channel`.
    pub address: String,
    /// z.B. "send_to_max_mustermann_via_telegram" — der MCP-Tool-Name.
    /// Name zuerst, Kanal als Suffix: so bleiben die Tools eines Kontakts
    /// auch in alphabetisch sortierenden Clients nebeneinander, statt nach
    /// Kanal in getrennte Blöcke zu zerfallen.
    pub tool_name: String,
    /// z.B. "Max Mustermann — Telegram" - für Clients, die MCPs `title`-Feld
    /// zusätzlich zum `name` anzeigen.
    pub title: String,
}

pub fn load_contacts(path: &Path) -> Result<Vec<ChannelTool>> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("konnte config nicht lesen: {}", path.display()))?;
    let parsed: RawConfig = toml::from_str(&raw)
        .with_context(|| format!("konnte config nicht parsen: {}", path.display()))?;

    if parsed.to.is_empty() {
        bail!("config.toml enthält keinen einzigen [[to]]-Eintrag — es gäbe kein einziges Tool zum Versenden");
    }

    let mut seen_slugs: HashMap<String, String> = HashMap::new();
    let mut tools = Vec::new();

    for entry in parsed.to {
        let name = entry.name.trim().to_string();
        if name.is_empty() {
            bail!("ein [[to]]-Eintrag hat einen leeren name");
        }

        let email = non_empty(entry.email);
        let telegram_chat_id = non_empty(entry.telegram_chat_id);
        if email.is_none() && telegram_chat_id.is_none() {
            bail!("Kontakt '{name}' hat keinen einzigen Kanal konfiguriert (email/telegram_chat_id)");
        }

        let slug = slugify(&name);
        if let Some(existing_name) = seen_slugs.insert(slug.clone(), name.clone()) {
            bail!(
                "Namenskollision in config.toml: '{existing_name}' und '{name}' ergeben \
                 denselben Slug '{slug}' und damit mehrdeutige Tool-Namen. Bitte einen der \
                 beiden Namen eindeutig machen (z.B. Nachname ergänzen) — der Server startet \
                 bewusst nicht mit mehrdeutigen Tool-Namen."
            );
        }

        if let Some(email) = email {
            if !email.contains('@') {
                bail!("Kontakt '{name}' hat keine gültige email ('{email}')");
            }
            tools.push(ChannelTool {
                contact_name: name.clone(),
                channel: Channel::Email,
                address: email,
                tool_name: format!("send_to_{slug}_via_{}", Channel::Email.slug()),
                title: format!("{name} — {}", Channel::Email.display_name()),
            });
        }

        if let Some(chat_id) = telegram_chat_id {
            if chat_id.parse::<i64>().is_err() {
                bail!(
                    "Kontakt '{name}' hat eine ungültige telegram_chat_id ('{chat_id}') - \
                     muss eine Zahl sein (Gruppen/Supergruppen haben negative IDs)"
                );
            }
            tools.push(ChannelTool {
                contact_name: name.clone(),
                channel: Channel::Telegram,
                address: chat_id,
                tool_name: format!("send_to_{slug}_via_{}", Channel::Telegram.slug()),
                title: format!("{name} — {}", Channel::Telegram.display_name()),
            });
        }
    }

    Ok(tools)
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Erzeugt aus einem Anzeigenamen einen stabilen, tool-tauglichen Slug.
/// Deutsche Umlaute werden transliteriert, alles andere auf [a-z0-9_] reduziert.
fn slugify(name: &str) -> String {
    let normalized: String = name
        .chars()
        .map(|c| match c {
            'ä' => "ae".to_string(),
            'ö' => "oe".to_string(),
            'ü' => "ue".to_string(),
            'Ä' => "Ae".to_string(),
            'Ö' => "Oe".to_string(),
            'Ü' => "Ue".to_string(),
            'ß' => "ss".to_string(),
            other => other.to_string(),
        })
        .collect();

    let mut out = String::new();
    let mut last_was_sep = true; // verhindert führenden Unterstrich
    for ch in normalized.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('_');
            last_was_sep = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("contact");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_umlauts() {
        assert_eq!(slugify("Jürgen Müller"), "juergen_mueller");
        assert_eq!(slugify("Max Mustermann"), "max_mustermann");
        assert_eq!(slugify("  Anna-Lena  "), "anna_lena");
    }

    fn write_config(contents: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sendmail-mcp-test-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn rand_suffix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64
    }

    #[test]
    fn detects_slug_collision() {
        let path = write_config(
            r#"
[[to]]
name = "Jürgen Schmidt"
email = "j1@example.com"

[[to]]
name = "Jürgen  Schmidt"
email = "j2@example.com"
"#,
        );
        let err = load_contacts(&path).unwrap_err();
        assert!(err.to_string().contains("Namenskollision"));
    }

    #[test]
    fn contact_needs_at_least_one_channel() {
        let path = write_config(
            r#"
[[to]]
name = "Ohne Kanal"
"#,
        );
        let err = load_contacts(&path).unwrap_err();
        assert!(err.to_string().contains("keinen einzigen Kanal"));
    }

    #[test]
    fn rejects_non_numeric_telegram_chat_id() {
        let path = write_config(
            r#"
[[to]]
name = "Max Mustermann"
telegram_chat_id = "@max_mustermann"
"#,
        );
        let err = load_contacts(&path).unwrap_err();
        assert!(err.to_string().contains("ungültige telegram_chat_id"));
    }

    #[test]
    fn contact_with_both_channels_gets_two_tools() {
        let path = write_config(
            r#"
[[to]]
name = "Max Mustermann"
email = "max@example.com"
telegram_chat_id = "123456789"
"#,
        );
        let tools = load_contacts(&path).unwrap();
        assert_eq!(tools.len(), 2);
        assert!(tools.iter().any(|t| t.tool_name == "send_to_max_mustermann_via_email"));
        assert!(tools.iter().any(|t| t.tool_name == "send_to_max_mustermann_via_telegram"));
        assert!(tools.iter().any(|t| t.title == "Max Mustermann — E-Mail"));
        assert!(tools.iter().any(|t| t.title == "Max Mustermann — Telegram"));
    }
}
