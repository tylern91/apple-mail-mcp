//! SMTP submission of composed MIME bytes (task 4) — the auth mechanism is selected from the
//! [`Credential`] variant (D1): `XOAUTH2` for Google, `PLAIN` for iCloud/Exchange/generic IMAP's
//! app-specific password. Both arms go through the same call site; there is no branch anywhere
//! else in this workspace that picks a different mechanism.

use lettre::address::Envelope;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{Address, SmtpTransport, Transport};

use amx_core::AmxError;

use crate::credentials::Credential;

/// The account's outbound SMTP endpoint, as discovered by
/// `amx_store::AccountResolver::sending_settings`. Duplicated here rather than depending on
/// `amx-store` directly — `amx-send` only ever needs these three primitives, not the store's
/// SQLite access.
pub struct SendingEndpoint {
    pub hostname: String,
    pub port: u16,
    pub ssl_enabled: bool,
}

fn to_address(email: &str) -> Result<Address, AmxError> {
    email.parse().map_err(|err| AmxError::ComposeInvalid {
        reason: format!("{email:?} is not a valid RFC 5321 address: {err}"),
    })
}

fn build_envelope(
    from_email: &str,
    to: &[String],
    cc: &[String],
    bcc: &[String],
) -> Result<Envelope, AmxError> {
    let mut recipients = Vec::with_capacity(to.len() + cc.len() + bcc.len());
    for email in to.iter().chain(cc).chain(bcc) {
        recipients.push(to_address(email)?);
    }
    if recipients.is_empty() {
        return Err(AmxError::ComposeInvalid {
            reason: "no recipients (to/cc/bcc all empty) — nothing to submit".to_string(),
        });
    }
    Envelope::new(Some(to_address(from_email)?), recipients).map_err(|err| {
        AmxError::ComposeInvalid {
            reason: format!("invalid SMTP envelope: {err}"),
        }
    })
}

/// Selects the SASL mechanism and secret for `credential` — `XOAUTH2` carries the access token as
/// its secret, `PLAIN` carries the app-specific password. Never a mismatch between the two: this
/// is the only place either arm is chosen from.
fn mechanism_and_secret(credential: &Credential) -> (Mechanism, String) {
    match credential {
        Credential::OAuth2 { access_token } => (Mechanism::Xoauth2, access_token.clone()),
        Credential::Password { secret } => (Mechanism::Plain, secret.clone()),
    }
}

/// Submits `raw_mime` (from [`amx_compose::compose`]) to `endpoint`, authenticating as
/// `from_email` with `credential`.
pub fn submit(
    endpoint: &SendingEndpoint,
    from_email: &str,
    to: &[String],
    cc: &[String],
    bcc: &[String],
    credential: &Credential,
    raw_mime: &[u8],
) -> Result<(), AmxError> {
    let envelope = build_envelope(from_email, to, cc, bcc)?;
    let (mechanism, secret) = mechanism_and_secret(credential);
    let credentials = Credentials::new(from_email.to_string(), secret);

    let builder = if endpoint.ssl_enabled {
        SmtpTransport::starttls_relay(&endpoint.hostname).map_err(|err| {
            AmxError::SmtpSubmissionFailed {
                hostname: endpoint.hostname.clone(),
                port: endpoint.port,
                reason: format!("could not establish STARTTLS: {err}"),
            }
        })?
    } else {
        SmtpTransport::builder_dangerous(&endpoint.hostname)
    };

    let mailer = builder
        .port(endpoint.port)
        .credentials(credentials)
        .authentication(vec![mechanism])
        .build();

    mailer
        .send_raw(&envelope, raw_mime)
        .map_err(|err| AmxError::SmtpSubmissionFailed {
            hostname: endpoint.hostname.clone(),
            port: endpoint.port,
            reason: err.to_string(),
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_rejects_an_invalid_recipient_address() {
        let err = build_envelope(
            "alice@example.com",
            &["not-an-address".to_string()],
            &[],
            &[],
        )
        .expect_err("malformed address should be rejected");
        assert!(matches!(err, AmxError::ComposeInvalid { .. }));
    }

    #[test]
    fn envelope_rejects_no_recipients() {
        let err = build_envelope("alice@example.com", &[], &[], &[])
            .expect_err("empty recipient set should be rejected");
        assert!(matches!(err, AmxError::ComposeInvalid { .. }));
    }

    #[test]
    fn envelope_folds_to_cc_and_bcc_together() {
        let envelope = build_envelope(
            "alice@example.com",
            &["bob@example.com".to_string()],
            &["carol@example.com".to_string()],
            &["dave@example.com".to_string()],
        )
        .expect("valid addresses should build an envelope");
        assert_eq!(envelope.to().len(), 3);
    }

    #[test]
    fn oauth2_credential_selects_xoauth2() {
        let credential = Credential::OAuth2 {
            access_token: "token-123".to_string(),
        };
        let (mechanism, secret) = mechanism_and_secret(&credential);
        assert_eq!(mechanism, Mechanism::Xoauth2);
        assert_eq!(secret, "token-123");
    }

    #[test]
    fn password_credential_selects_plain() {
        let credential = Credential::Password {
            secret: "app-specific-pw".to_string(),
        };
        let (mechanism, secret) = mechanism_and_secret(&credential);
        assert_eq!(mechanism, Mechanism::Plain);
        assert_eq!(secret, "app-specific-pw");
    }
}
