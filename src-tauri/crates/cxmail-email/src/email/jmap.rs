//! JMAP (JSON Meta Application Protocol) transport layer.
//!
//! Alternative to IMAP for providers that support it (e.g., Fastmail).
//! Implements the same operations as imap.rs behind a common trait.
//!
//! RFC 8620 (JMAP Core) + RFC 8621 (JMAP Mail)

use crate::error::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct JmapClient {
    base_url: String,
    auth_token: String,
    account_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct JmapSession {
    accounts: std::collections::HashMap<String, JmapAccount>,
    api_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct JmapAccount {
    name: String,
    #[serde(rename = "isPersonal")]
    is_personal: bool,
}

impl JmapClient {
    /// Connect to a JMAP server via the well-known endpoint.
    pub async fn connect(server: &str, auth_token: &str) -> Result<Self, AppError> {
        let session_url = format!("https://{}/.well-known/jmap", server);

        let client = reqwest::Client::new();
        let resp = client
            .get(&session_url)
            .bearer_auth(auth_token)
            .send()
            .await
            .map_err(|e| AppError::General(format!("JMAP session request failed: {}", e)))?;

        let session: JmapSession = resp
            .json()
            .await
            .map_err(|e| AppError::General(format!("JMAP session parse failed: {}", e)))?;

        let account_id = session
            .accounts
            .iter()
            .find(|(_, a)| a.is_personal)
            .map(|(id, _)| id.clone())
            .ok_or_else(|| AppError::General("No personal JMAP account found".to_string()))?;

        Ok(Self {
            base_url: session.api_url,
            auth_token: auth_token.to_string(),
            account_id,
        })
    }

    /// List mailboxes (equivalent to IMAP LIST).
    pub async fn list_mailboxes(&self) -> Result<Vec<JmapMailbox>, AppError> {
        let request = serde_json::json!({
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            "methodCalls": [
                ["Mailbox/get", {
                    "accountId": self.account_id,
                }, "0"]
            ]
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(&self.base_url)
            .bearer_auth(&self.auth_token)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::General(format!("JMAP request failed: {}", e)))?;

        let body: serde_json::Value = resp.json().await
            .map_err(|e| AppError::General(format!("JMAP response parse failed: {}", e)))?;

        let list = body["methodResponses"][0][1]["list"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        let mailboxes: Vec<JmapMailbox> = list
            .into_iter()
            .filter_map(|m| serde_json::from_value(m).ok())
            .collect();

        Ok(mailboxes)
    }

    /// Query emails in a mailbox.
    pub async fn query_emails(
        &self,
        mailbox_id: &str,
        limit: u32,
    ) -> Result<Vec<String>, AppError> {
        let request = serde_json::json!({
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            "methodCalls": [
                ["Email/query", {
                    "accountId": self.account_id,
                    "filter": { "inMailbox": mailbox_id },
                    "sort": [{ "property": "receivedAt", "isAscending": false }],
                    "limit": limit,
                }, "0"]
            ]
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(&self.base_url)
            .bearer_auth(&self.auth_token)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::General(format!("JMAP query failed: {}", e)))?;

        let body: serde_json::Value = resp.json().await
            .map_err(|e| AppError::General(format!("JMAP response parse failed: {}", e)))?;

        let ids: Vec<String> = body["methodResponses"][0][1]["ids"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        Ok(ids)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmapMailbox {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    #[serde(rename = "totalEmails")]
    pub total_emails: Option<u64>,
    #[serde(rename = "unreadEmails")]
    pub unread_emails: Option<u64>,
}
