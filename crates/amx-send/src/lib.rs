#![cfg(target_os = "macos")]

mod credentials;
mod oauth;
mod smtp;

pub use credentials::{Credential, CredentialBroker, Provider};
pub use oauth::{GoogleOAuthConfig, refresh_access_token, run_consent_flow};
pub use smtp::{SendingEndpoint, submit};
