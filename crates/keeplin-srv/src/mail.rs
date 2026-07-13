//! Delegated email delivery (issue #49).
//!
//! keeplin-srv is **not a mail client**: it never speaks SMTP and holds no
//! provider SDK. When an email flow (verification, password reset) needs a
//! message delivered, the server POSTs a small JSON payload to the operator's
//! own mail service — the **mail webhook** — which composes and sends the
//! actual email with whatever provider it likes:
//!
//! ```json
//! {
//!   "kind": "verify_email" | "password_reset",
//!   "to": "user@example.com",
//!   "display_name": "User",
//!   "token": "<single-use token to embed in the link>",
//!   "expires_at": "2026-07-13T00:00:00Z"
//! }
//! ```
//!
//! `MAIL_WEBHOOK_TOKEN`, when set, is sent as a bearer so the webhook can
//! authenticate the server. Without `MAIL_WEBHOOK_URL` the mailer is disabled
//! and the flows answer `501` — an explicit deferral, never silent mail loss.

use chrono::{DateTime, Utc};

/// The two email flows the server can ask the webhook to deliver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailKind {
    VerifyEmail,
    PasswordReset,
}

impl MailKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MailKind::VerifyEmail => "verify_email",
            MailKind::PasswordReset => "password_reset",
        }
    }
}

/// Posts email-flow payloads to the operator's mail webhook. Cheap to clone.
#[derive(Clone)]
pub struct Mailer {
    webhook_url: Option<String>,
    webhook_token: Option<String>,
    http: reqwest::Client,
}

impl Mailer {
    pub fn new(webhook_url: Option<String>, webhook_token: Option<String>) -> Self {
        Self {
            webhook_url,
            webhook_token,
            http: reqwest::Client::new(),
        }
    }

    /// Whether delivery is configured. Flows check this up front and answer
    /// `501` when it is not, so the deferral is visible to callers.
    pub fn enabled(&self) -> bool {
        self.webhook_url.is_some()
    }

    /// Deliver one flow message. An unreachable/erroring webhook is reported as
    /// an error so the caller can surface a `500` rather than pretend the mail
    /// went out (the token would otherwise be stranded).
    pub async fn send(
        &self,
        kind: MailKind,
        to: &str,
        display_name: &str,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), String> {
        let Some(url) = &self.webhook_url else {
            return Err("mail webhook not configured".into());
        };
        let mut req = self.http.post(url).json(&serde_json::json!({
            "kind": kind.as_str(),
            "to": to,
            "display_name": display_name,
            "token": token,
            "expires_at": expires_at,
        }));
        if let Some(bearer) = &self.webhook_token {
            req = req.bearer_auth(bearer);
        }
        match req.send().await {
            Ok(resp) if resp.status().is_success() => Ok(()),
            Ok(resp) => Err(format!("mail webhook answered {}", resp.status())),
            Err(e) => Err(format!("mail webhook unreachable: {e}")),
        }
    }
}
