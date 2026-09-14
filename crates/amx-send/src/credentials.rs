//! Credential storage — our own Keychain namespace via `keyring`, never Mail.app's.
//!
//! Two arms behind one interface (D1): Google accounts authenticate with OAuth2/XOAUTH2, iCloud
//! and generic IMAP accounts with an app-specific password. `lettre`/`imap` both accept either
//! mechanism at the same call site — the caller picks a [`Provider`] from the account's kind, not
//! from a value inside [`Credential`] itself.

use amx_core::AmxError;

use crate::oauth::GoogleOAuthConfig;

const KEYRING_SERVICE: &str = "apple-mail-mcp";
const REFRESH_TOKEN_SUFFIX: &str = "#oauth-refresh-token";

/// Which authentication mechanism an account's provider requires (D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// Google — full OAuth2 client (XOAUTH2), the only arm with a public third-party endpoint.
    Google,
    /// iCloud, Exchange, and generic IMAP — an app-specific (or account) password. Apple's
    /// `ATOKEN2` OAuth scheme is private; there is no public flow to reach it (F5).
    AppSpecificPassword,
}

/// A credential ready to authenticate an SMTP/IMAP session.
///
/// `Debug` is hand-written to print `<redacted>` in place of the secret — this, not a tracing
/// redaction filter, is what satisfies the umbrella's secrets-never-logged requirement here,
/// because this workspace has no tracing layer to filter (F6).
pub enum Credential {
    OAuth2 { access_token: String },
    Password { secret: String },
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Credential::OAuth2 { .. } => write!(f, "Credential::OAuth2(<redacted>)"),
            Credential::Password { .. } => write!(f, "Credential::Password(<redacted>)"),
        }
    }
}

fn entry(account: &str) -> Result<keyring::Entry, AmxError> {
    keyring::Entry::new(KEYRING_SERVICE, account).map_err(|err| AmxError::OAuthFlowFailed {
        reason: format!("could not open a Keychain entry for {account}: {err}"),
    })
}

pub struct CredentialBroker;

impl CredentialBroker {
    /// Resolves the stored credential for `email` under the mechanism its `provider` requires.
    pub fn for_account(email: &str, provider: Provider) -> Result<Credential, AmxError> {
        match provider {
            Provider::Google => {
                let refresh_token = Self::refresh_token(email)?;
                let config = GoogleOAuthConfig::from_env()?;
                let access_token = crate::oauth::refresh_access_token(&config, &refresh_token)?;
                Ok(Credential::OAuth2 { access_token })
            }
            Provider::AppSpecificPassword => {
                let secret = Self::password(email)?;
                Ok(Credential::Password { secret })
            }
        }
    }

    /// Stores an app-specific (or account) password for `email`.
    pub fn store_password(email: &str, secret: &str) -> Result<(), AmxError> {
        entry(email)?
            .set_password(secret)
            .map_err(|err| AmxError::OAuthFlowFailed {
                reason: format!("could not store the password for {email}: {err}"),
            })
    }

    /// Stores a Google OAuth2 refresh token for `email`, under a distinct Keychain account so it
    /// never collides with an app-specific password entry for the same address.
    pub fn store_refresh_token(email: &str, refresh_token: &str) -> Result<(), AmxError> {
        entry(&format!("{email}{REFRESH_TOKEN_SUFFIX}"))?
            .set_password(refresh_token)
            .map_err(|err| AmxError::OAuthFlowFailed {
                reason: format!("could not store the refresh token for {email}: {err}"),
            })
    }

    fn password(email: &str) -> Result<String, AmxError> {
        entry(email)?.get_password().map_err(|err| match err {
            keyring::Error::NoEntry => AmxError::CredentialUnavailable {
                email: email.to_string(),
            },
            other => AmxError::OAuthFlowFailed {
                reason: format!("could not read the password for {email}: {other}"),
            },
        })
    }

    fn refresh_token(email: &str) -> Result<String, AmxError> {
        entry(&format!("{email}{REFRESH_TOKEN_SUFFIX}"))?
            .get_password()
            .map_err(|err| match err {
                keyring::Error::NoEntry => AmxError::CredentialUnavailable {
                    email: email.to_string(),
                },
                other => AmxError::OAuthFlowFailed {
                    reason: format!("could not read the refresh token for {email}: {other}"),
                },
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_the_secret() {
        let oauth = Credential::OAuth2 {
            access_token: "ya29.super-secret-token".to_string(),
        };
        let password = Credential::Password {
            secret: "hunter2-app-specific".to_string(),
        };
        assert!(!format!("{oauth:?}").contains("super-secret-token"));
        assert!(!format!("{password:?}").contains("hunter2-app-specific"));
        assert!(format!("{oauth:?}").contains("<redacted>"));
        assert!(format!("{password:?}").contains("<redacted>"));
    }
}
