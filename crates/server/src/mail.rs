//! Outgoing email: notifications are sent to people's inboxes through the company's own SMTP
//! server. Settings and the "already sent" mark live in the credential store, not the journal.

use buildkiosk_store::creds::SmtpSettings;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials as SmtpCreds;
use lettre::{Message, SmtpTransport, Transport};

/// One message to send.
pub struct Mail {
    /// The notification this is about (0 for a test message).
    pub id: u64,
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// Checks the settings a form gave; returns the clean settings.
pub fn check(s: SmtpSettings) -> Result<SmtpSettings, String> {
    let host = s.host.trim().to_string();
    if host.is_empty() || host.len() > 253 || host.chars().any(|c| c.is_whitespace() || c == '/') {
        return Err("Enter the mail server's host name.".into());
    }
    if s.port == 0 {
        return Err("Enter the mail server's port.".into());
    }
    let security = match s.security.as_str() {
        "tls" | "none" => s.security.clone(),
        _ => "starttls".to_string(),
    };
    if s.from.parse::<Mailbox>().is_err() {
        return Err("The sender isn't a valid address. Use a form like noreply@example.com.".into());
    }
    let base_url = s.base_url.trim().trim_end_matches('/').to_string();
    if !base_url.is_empty() && !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
        return Err("The address of this server must start with https:// or http://.".into());
    }
    Ok(SmtpSettings { host, security, base_url, from: s.from.trim().to_string(), ..s })
}

fn transport(s: &SmtpSettings) -> Result<SmtpTransport, String> {
    let builder = match s.security.as_str() {
        "tls" => SmtpTransport::relay(&s.host).map_err(|e| e.to_string())?,
        "none" => SmtpTransport::builder_dangerous(&s.host),
        _ => SmtpTransport::starttls_relay(&s.host).map_err(|e| e.to_string())?,
    };
    let mut b = builder.port(s.port).timeout(Some(std::time::Duration::from_secs(20)));
    if !s.username.is_empty() {
        b = b.credentials(SmtpCreds::new(s.username.clone(), s.password.clone()));
    }
    Ok(b.build())
}

/// Sends one message (blocking; call from a blocking thread).
pub fn send(s: &SmtpSettings, mail: &Mail) -> Result<(), String> {
    let from: Mailbox = s.from.parse().map_err(|_| "the sender address is not valid".to_string())?;
    let to: Mailbox = mail.to.parse().map_err(|_| format!("{} is not a valid address", mail.to))?;
    let msg = Message::builder().from(from).to(to).subject(mail.subject.clone()).body(mail.body.clone()).map_err(|e| e.to_string())?;
    transport(s)?.send(&msg).map(|_| ()).map_err(|e| e.to_string())
}
