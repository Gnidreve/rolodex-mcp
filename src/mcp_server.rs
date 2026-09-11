use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler};
use serde_json::{json, Map, Value};

use crate::config::{Channel, ChannelTool};
use crate::smtp::{send_mail, SmtpConfig};
use crate::telegram::{send_telegram, TelegramConfig};

/// Ein MCP-Server, der pro konfiguriertem Kontakt UND Kanal genau EIN Tool
/// anbietet. Es gibt bewusst KEIN generisches "send_message(channel, to,
/// subject, body)"-Tool — weder die Adresse/Chat-ID noch der Kanal werden
/// dem Agenten als Parameter exponiert, beides steckt fest im Tool-Namen
/// selbst (z.B. "send_telegram_to_max_mustermann").
#[derive(Clone)]
pub struct SendMailServer {
    tools: Arc<Vec<ChannelTool>>,
    smtp: Option<Arc<SmtpConfig>>,
    telegram: Option<Arc<TelegramConfig>>,
}

impl SendMailServer {
    pub fn new(tools: Vec<ChannelTool>, smtp: Option<SmtpConfig>, telegram: Option<TelegramConfig>) -> Self {
        Self {
            tools: Arc::new(tools),
            smtp: smtp.map(Arc::new),
            telegram: telegram.map(Arc::new),
        }
    }

    fn find(&self, tool_name: &str) -> Option<&ChannelTool> {
        self.tools.iter().find(|t| t.tool_name == tool_name)
    }

    fn tool_input_schema() -> Arc<Map<String, Value>> {
        let schema = json!({
            "type": "object",
            "properties": {
                "subject": { "type": "string", "description": "Betreff/Titel der Nachricht" },
                "body": { "type": "string", "description": "Inhalt der Nachricht (Klartext)" }
            },
            "required": ["subject", "body"]
        });
        match schema {
            Value::Object(map) => Arc::new(map),
            _ => unreachable!("json!-Makro liefert hier immer ein Objekt"),
        }
    }
}

impl ServerHandler for SendMailServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "sendmail-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..Default::default()
            },
            instructions: Some(
                "Schickt Nachrichten an fest konfigurierte Empfänger, über fest konfigurierte \
                 Kanäle (E-Mail, Telegram). Jede Empfänger-Kanal-Kombination hat ein eigenes \
                 Tool (send_email_to_<name>, send_telegram_to_<name>) — es gibt kein \
                 generisches Tool mit freier Adresseingabe oder Kanalwahl. Wähle das Tool, \
                 dessen Name zu gewünschtem Empfänger UND Kanal passt, und gib subject + body an."
                    .into(),
            ),
            ..Default::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self
            .tools
            .iter()
            .map(|t| {
                let description = match t.channel {
                    Channel::Email => format!("Sende eine E-Mail an {}.", t.contact_name),
                    Channel::Telegram => format!("Sende eine Telegram-Nachricht an {}.", t.contact_name),
                };
                Tool::new(t.tool_name.clone(), description, Self::tool_input_schema())
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool = self
            .find(&request.name)
            .ok_or_else(|| McpError::invalid_params(format!("unbekanntes Tool: {}", request.name), None))?;

        let args = request.arguments.unwrap_or_default();
        let subject = args
            .get("subject")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::invalid_params("Parameter 'subject' fehlt", None))?;
        let body = args
            .get("body")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::invalid_params("Parameter 'body' fehlt", None))?;

        let result = match tool.channel {
            Channel::Email => {
                let smtp = self
                    .smtp
                    .as_ref()
                    .expect("Tool send_email_to_* existiert nur, wenn SmtpConfig geladen wurde");
                send_mail(smtp, &tool.contact_name, &tool.address, subject, body).await
            }
            Channel::Telegram => {
                let telegram = self
                    .telegram
                    .as_ref()
                    .expect("Tool send_telegram_to_* existiert nur, wenn TelegramConfig geladen wurde");
                send_telegram(telegram, &tool.address, subject, body).await
            }
        };

        match result {
            Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Nachricht an {} wurde verschickt.",
                tool.contact_name
            ))])),
            Err(err) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Versand an {} fehlgeschlagen: {err:#}",
                tool.contact_name
            ))])),
        }
    }
}
