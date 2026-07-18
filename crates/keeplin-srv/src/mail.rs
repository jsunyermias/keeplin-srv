// md:Overview
use chrono::{DateTime, Utc};

// md:MailKind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailKind {
    VerifyEmail,
    PasswordReset,
}

// md:impl MailKind
impl MailKind {
    // md:impl MailKind > fn as_str
    pub fn as_str(self) -> &'static str {
        match self {
            MailKind::VerifyEmail => "verify_email",
            MailKind::PasswordReset => "password_reset",
        }
    }
}

// md:Mailer
#[derive(Clone)]
pub struct Mailer {
    webhook_url: Option<String>,
    webhook_token: Option<String>,
    http: reqwest::Client,
}

// md:impl Mailer
impl Mailer {
    // md:impl Mailer > fn new
    pub fn new(webhook_url: Option<String>, webhook_token: Option<String>) -> Self {
        Self {
            webhook_url,
            webhook_token,
            http: reqwest::Client::new(),
        }
    }

    // md:impl Mailer > fn enabled
    pub fn enabled(&self) -> bool {
        self.webhook_url.is_some()
    }

    // md:impl Mailer > fn send
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
