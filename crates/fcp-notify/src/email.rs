//! The email channel: the notice as a message over SMTP
//! (`docs/E911_BLOCK_2.md` §8.1).

use crate::NotificationChannel;
use async_trait::async_trait;
use fcp_core::e911::notify::{ContactKind, EmergencyNotice, NotifyContact};
use lettre::message::{Mailbox, MessageBuilder};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use std::time::Duration;

/// How this deployment sends email.
#[derive(Debug, Clone)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    /// `starttls` (the default), `tls` for an implicit TLS port, or
    /// `none` — which only a relay on the same host should ever use.
    pub encryption: String,
    pub username: Option<String>,
    pub password: Option<String>,
    /// The address the notice comes from.
    pub from: String,
    pub timeout: Duration,
}

/// Sends the notice by email.
pub struct EmailChannel {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl EmailChannel {
    pub fn new(settings: &SmtpSettings) -> anyhow::Result<Self> {
        let from: Mailbox = settings
            .from
            .parse()
            .map_err(|e| anyhow::anyhow!("notifications.smtp.from is not an address: {e}"))?;
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&settings.host)
            .port(settings.port)
            .timeout(Some(settings.timeout));
        builder = match settings.encryption.as_str() {
            "none" => builder.tls(Tls::None),
            "tls" | "implicit" | "smtps" => {
                builder.tls(Tls::Wrapper(TlsParameters::new(settings.host.clone())?))
            }
            // The default: a plain connection upgraded with STARTTLS,
            // required rather than opportunistic — an emergency notice is
            // not sent in the clear because a server forgot to offer it.
            _ => builder.tls(Tls::Required(TlsParameters::new(settings.host.clone())?)),
        };
        if let Some(username) = settings.username.clone().filter(|u| !u.is_empty()) {
            builder = builder.credentials(Credentials::new(
                username,
                settings.password.clone().unwrap_or_default(),
            ));
        }
        Ok(Self {
            transport: builder.build(),
            from,
        })
    }
}

#[async_trait]
impl NotificationChannel for EmailChannel {
    fn kind(&self) -> ContactKind {
        ContactKind::Email
    }

    async fn deliver(
        &self,
        contact: &NotifyContact,
        notice: &EmergencyNotice,
    ) -> anyhow::Result<()> {
        let to: Mailbox = contact
            .target
            .parse()
            .map_err(|e| anyhow::anyhow!("{} is not an address: {e}", contact.target))?;
        let message = MessageBuilder::new()
            .from(self.from.clone())
            .to(to)
            .subject(notice.subject())
            // An emergency notice is read now or not at all.
            .header(lettre::message::header::ContentType::TEXT_PLAIN)
            .body(notice.text())?;
        self.transport.send(message).await?;
        Ok(())
    }
}
