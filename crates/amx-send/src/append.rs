//! IMAP `APPEND` filing of composed MIME bytes (task 5, D2) — used only to file a sent message
//! into the Sent mailbox for accounts that don't file it server-side, and to file a draft into
//! the Drafts mailbox. Never used for reading; that stays `amx-store`'s job in the read lane.

use imap::types::Flag;
use imap::{Authenticator, ClientBuilder, Connection, Session};
use imap_proto::NameAttribute;

use amx_core::AmxError;

use crate::credentials::Credential;

/// The account's IMAP endpoint, discovered the same way as [`crate::SendingEndpoint`] but from
/// the mail account's own `Hostname`/`PortNumber` properties directly — no
/// `SendingAccountIdentifier` hop, since the IMAP account *is* the receiving account.
pub struct ImapEndpoint {
    pub hostname: String,
    pub port: u16,
}

/// The special-use mailbox to file into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecialUse {
    Sent,
    Drafts,
}

impl SpecialUse {
    fn attribute(self) -> NameAttribute<'static> {
        match self {
            Self::Sent => NameAttribute::Sent,
            Self::Drafts => NameAttribute::Drafts,
        }
    }

    /// Name-guess fallback for servers that don't advertise the RFC 6154 special-use attribute
    /// on a plain `LIST` — iCloud has been observed to name this mailbox `"Sent Messages"` rather
    /// than the generic `"Sent"` (sub-plan §"Risks").
    fn name_fallbacks(self) -> &'static [&'static str] {
        match self {
            Self::Sent => &["Sent Messages", "Sent"],
            Self::Drafts => &["Drafts"],
        }
    }
}

struct Xoauth2<'a> {
    user: &'a str,
    access_token: &'a str,
}

impl Authenticator for Xoauth2<'_> {
    type Response = String;

    fn process(&self, _challenge: &[u8]) -> Self::Response {
        format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.user, self.access_token
        )
    }
}

fn connect_and_authenticate(
    endpoint: &ImapEndpoint,
    from_email: &str,
    credential: &Credential,
) -> Result<Session<Connection>, AmxError> {
    let to_failure = |reason: String| AmxError::ImapAppendFailed {
        mailbox: "<connect>".to_string(),
        reason,
    };

    let client = ClientBuilder::new(endpoint.hostname.as_str(), endpoint.port)
        .connect()
        .map_err(|err| {
            to_failure(format!(
                "could not connect to {}:{}: {err}",
                endpoint.hostname, endpoint.port
            ))
        })?;

    match credential {
        Credential::OAuth2 { access_token } => {
            let auth = Xoauth2 {
                user: from_email,
                access_token,
            };
            client
                .authenticate("XOAUTH2", &auth)
                .map_err(|(err, _client)| {
                    to_failure(format!("XOAUTH2 authentication failed: {err}"))
                })
        }
        Credential::Password { secret } => client
            .login(from_email, secret)
            .map_err(|(err, _client)| to_failure(format!("LOGIN authentication failed: {err}"))),
    }
}

/// Resolves `special_use` to a selectable mailbox name: prefers the RFC 6154 special-use
/// attribute reported by `LIST`, falling back to a name guess when the server doesn't advertise
/// it on a plain `LIST` (observed live: not every server needs `LIST-EXTENDED` to report this,
/// but not every server reports it unprompted either).
fn resolve_mailbox_name(
    session: &mut Session<Connection>,
    special_use: SpecialUse,
) -> Result<String, AmxError> {
    let names = session
        .list(None, Some("*"))
        .map_err(|err| AmxError::ImapAppendFailed {
            mailbox: "<list>".to_string(),
            reason: format!("could not list mailboxes: {err}"),
        })?;

    let attribute = special_use.attribute();
    for name in names.iter() {
        if name.attributes().contains(&attribute) {
            return Ok(name.name().to_string());
        }
    }

    for candidate in special_use.name_fallbacks() {
        if names.iter().any(|name| name.name() == *candidate) {
            return Ok((*candidate).to_string());
        }
    }

    Err(AmxError::ImapAppendFailed {
        mailbox: format!("{special_use:?}"),
        reason: "no mailbox advertises this special-use attribute, and no name-guess fallback \
                 matched the server's mailbox list"
            .to_string(),
    })
}

/// Appends `raw_mime` to the account's `special_use` mailbox, marked `\Seen`.
///
/// A failed APPEND is always reported to the caller as its own error — it never triggers a
/// re-send, and a successful `smtp::submit` followed by a failed `append` must be surfaced by the
/// caller as a partial success, not silently swallowed (this is the parasxos #3 guard extended to
/// the filing step: a degraded outcome must never look identical to a full success).
pub fn append_to_special_use(
    endpoint: &ImapEndpoint,
    from_email: &str,
    credential: &Credential,
    special_use: SpecialUse,
    raw_mime: &[u8],
) -> Result<(), AmxError> {
    let mut session = connect_and_authenticate(endpoint, from_email, credential)?;
    let mailbox = resolve_mailbox_name(&mut session, special_use)?;

    session
        .append(&mailbox, raw_mime)
        .flag(Flag::Seen)
        .finish()
        .map_err(|err| AmxError::ImapAppendFailed {
            mailbox: mailbox.clone(),
            reason: err.to_string(),
        })?;

    let _ = session.logout();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_special_use_falls_back_to_icloud_naming_before_generic_sent() {
        assert_eq!(
            SpecialUse::Sent.name_fallbacks(),
            &["Sent Messages", "Sent"]
        );
    }

    #[test]
    fn xoauth2_response_matches_the_sasl_wire_format() {
        let auth = Xoauth2 {
            user: "alice@example.com",
            access_token: "token-123",
        };
        assert_eq!(
            auth.process(&[]),
            "user=alice@example.com\x01auth=Bearer token-123\x01\x01"
        );
    }
}
