#![cfg(target_os = "macos")]
//! `send_message` / `create_draft` (Phase 5 task 6) — compose in-process (`amx-compose`), submit
//! or file over the account's own SMTP/IMAP endpoint (`amx-send`). macOS-only: `amx-send` itself
//! is `#[cfg(target_os = "macos")]`-gated, so this module is too, following the `tools::mutate`
//! precedent in the (unmerged) mutate-lane branch.
//!
//! The D2 Gmail-exception (skip Sent-folder `APPEND` for Gmail, which files it server-side) is
//! decided here, not in `amx-send` — that crate stays decoupled from any provider-specific
//! branching (task 5's Results section). The signal is the resolved IMAP/SMTP hostname
//! (`imap.gmail.com`/`smtp.gmail.com`), since `AccountKind` has no distinct Gmail variant: Gmail's
//! Mail.app account type is the same `com.apple.account.IMAP` as any other IMAP provider.

use amx_compose::{Draft, EmailAddress, compose};
use amx_core::AmxError;
use amx_send::{
    CredentialBroker, ImapEndpoint, Provider, SendingEndpoint, SpecialUse, append_to_special_use,
    submit,
};
use amx_store::account::AccountResolver;

use crate::schema::tools::{
    CreateDraftRequest, CreateDraftResponse, SendMessageRequest, SendMessageResponse,
};

fn is_gmail_hostname(hostname: &str) -> bool {
    hostname.to_ascii_lowercase().ends_with("gmail.com")
}

fn provider_for_hostname(hostname: &str) -> Provider {
    if is_gmail_hostname(hostname) {
        Provider::Google
    } else {
        Provider::AppSpecificPassword
    }
}

/// Resolves `from` (an email address) to its owning account's identifier — the UUID
/// `sending_settings`/`receiving_settings` key on, not the address itself.
fn resolve_identifier(account_resolver: &AccountResolver, from: &str) -> Result<String, AmxError> {
    account_resolver
        .resolve_by_address(from)?
        .into_iter()
        .next()
        .map(|account| account.identifier)
        .ok_or_else(|| AmxError::AccountFilterUnmatched {
            requested: from.to_string(),
            available: 0,
            suggestions: Vec::new(),
        })
}

fn build_draft(
    from: &str,
    to: &[String],
    cc: &[String],
    bcc: &[String],
    subject: &str,
    text_body: &Option<String>,
    html_body: &Option<String>,
) -> Draft {
    Draft {
        from: Some(EmailAddress::new(from.to_string())),
        to: to.iter().cloned().map(EmailAddress::new).collect(),
        cc: cc.iter().cloned().map(EmailAddress::new).collect(),
        bcc: bcc.iter().cloned().map(EmailAddress::new).collect(),
        subject: subject.to_string(),
        text_body: text_body.clone(),
        html_body: html_body.clone(),
        ..Draft::default()
    }
}

/// Submits `request` over SMTP, then — unless `from` resolves to a Gmail account (D2) — files a
/// copy into the Sent mailbox via IMAP `APPEND`.
///
/// A failed submission is a hard error. A failed *filing* after a successful submission is not:
/// the mail already left, so `response.filing_error` reports it as a partial success rather than
/// looking identical to a full success (the parasxos #3 guard, extended to the filing step —
/// task 5's Results section).
pub fn run_send_message(
    account_resolver: &AccountResolver,
    request: &SendMessageRequest,
) -> Result<SendMessageResponse, AmxError> {
    let draft = build_draft(
        &request.from,
        &request.to,
        &request.cc,
        &request.bcc,
        &request.subject,
        &request.text_body,
        &request.html_body,
    );
    let composed = compose(&draft)?;

    let identifier = resolve_identifier(account_resolver, &request.from)?;
    let sending = account_resolver
        .sending_settings(&identifier)?
        .ok_or_else(|| AmxError::SendingSettingsUnresolvable {
            identifier: identifier.clone(),
            reason: "account resolved but has no sending settings".to_string(),
        })?;

    let is_gmail = is_gmail_hostname(&sending.hostname);
    let credential =
        CredentialBroker::for_account(&request.from, provider_for_hostname(&sending.hostname))?;

    let endpoint = SendingEndpoint {
        hostname: sending.hostname,
        port: sending.port,
        ssl_enabled: sending.ssl_enabled,
    };
    submit(
        &endpoint,
        &request.from,
        &request.to,
        &request.cc,
        &request.bcc,
        &credential,
        &composed.raw,
    )?;

    let (filed_to_sent, filing_error) = if is_gmail {
        (false, None)
    } else {
        match file_to_sent(
            account_resolver,
            &identifier,
            &request.from,
            &credential,
            &composed.raw,
        ) {
            Ok(()) => (true, None),
            Err(err) => (false, Some(err.to_string())),
        }
    };

    Ok(SendMessageResponse {
        message_id: composed.message_id,
        filed_to_sent,
        filing_error,
    })
}

fn file_to_sent(
    account_resolver: &AccountResolver,
    identifier: &str,
    from: &str,
    credential: &amx_send::Credential,
    raw_mime: &[u8],
) -> Result<(), AmxError> {
    let receiving = account_resolver
        .receiving_settings(identifier)?
        .ok_or_else(|| AmxError::ReceivingSettingsUnresolvable {
            identifier: identifier.to_string(),
            reason: "account resolved but has no receiving (IMAP) settings".to_string(),
        })?;
    let imap_endpoint = ImapEndpoint {
        hostname: receiving.hostname,
        port: receiving.port,
    };
    append_to_special_use(&imap_endpoint, from, credential, SpecialUse::Sent, raw_mime)
}

/// Composes `request` and files it into the account's Drafts mailbox via IMAP `APPEND` — no SMTP
/// submission, per D3 (`create_draft` never sends).
pub fn run_create_draft(
    account_resolver: &AccountResolver,
    request: &CreateDraftRequest,
) -> Result<CreateDraftResponse, AmxError> {
    let draft = build_draft(
        &request.from,
        &request.to,
        &request.cc,
        &request.bcc,
        &request.subject,
        &request.text_body,
        &request.html_body,
    );
    let composed = compose(&draft)?;

    let identifier = resolve_identifier(account_resolver, &request.from)?;
    let receiving = account_resolver
        .receiving_settings(&identifier)?
        .ok_or_else(|| AmxError::ReceivingSettingsUnresolvable {
            identifier: identifier.clone(),
            reason: "account resolved but has no receiving (IMAP) settings".to_string(),
        })?;

    let credential =
        CredentialBroker::for_account(&request.from, provider_for_hostname(&receiving.hostname))?;
    let imap_endpoint = ImapEndpoint {
        hostname: receiving.hostname,
        port: receiving.port,
    };
    append_to_special_use(
        &imap_endpoint,
        &request.from,
        &credential,
        SpecialUse::Drafts,
        &composed.raw,
    )?;

    Ok(CreateDraftResponse {
        message_id: composed.message_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    #[test]
    fn gmail_hostnames_are_recognised_case_insensitively() {
        assert!(is_gmail_hostname("imap.gmail.com"));
        assert!(is_gmail_hostname("SMTP.GMAIL.COM"));
        assert!(!is_gmail_hostname("imap.mail.me.com"));
    }

    #[test]
    fn gmail_hostname_selects_the_google_provider() {
        assert_eq!(provider_for_hostname("smtp.gmail.com"), Provider::Google);
        assert_eq!(
            provider_for_hostname("smtp.mail.me.com"),
            Provider::AppSpecificPassword
        );
    }

    fn seed_accounts_db(path: &std::path::Path) {
        Connection::open(path)
            .unwrap()
            .execute_batch(
                r#"
                CREATE TABLE ZACCOUNTTYPE (Z_PK INTEGER PRIMARY KEY, ZIDENTIFIER TEXT);
                CREATE TABLE ZACCOUNT (
                    Z_PK INTEGER PRIMARY KEY, ZIDENTIFIER TEXT, ZACCOUNTDESCRIPTION TEXT,
                    ZUSERNAME TEXT, ZACCOUNTTYPE INTEGER
                );
                CREATE TABLE ZACCOUNTPROPERTY (Z_PK INTEGER PRIMARY KEY, ZOWNER INTEGER, ZKEY TEXT);

                INSERT INTO ZACCOUNTTYPE VALUES (1, 'com.apple.account.IMAP');
                INSERT INTO ZACCOUNT VALUES (10, 'ACCOUNT-UUID', NULL, 'alice@example.com', 1);
                "#,
            )
            .unwrap();
    }

    #[test]
    fn resolve_identifier_finds_the_owning_account() {
        let file = NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();
        let identifier = resolve_identifier(&resolver, "Alice@Example.com").unwrap();
        assert_eq!(identifier, "ACCOUNT-UUID");
    }

    #[test]
    fn resolve_identifier_errors_loudly_on_no_match() {
        let file = NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();
        let err = resolve_identifier(&resolver, "nobody@example.com").unwrap_err();
        assert!(matches!(err, AmxError::AccountFilterUnmatched { .. }));
    }
}
