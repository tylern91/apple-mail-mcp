//! Google OAuth2 — loopback-redirect + PKCE consent flow, and refresh-token-based renewal.
//!
//! Apple publishes no third-party OAuth endpoint for IMAP/SMTP (F5) — this module exists only for
//! Google accounts. iCloud/Exchange/generic IMAP use [`crate::credentials::Provider::AppSpecificPassword`]
//! instead, never this flow.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use amx_core::AmxError;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://mail.google.com/";

/// Google Cloud OAuth **Desktop app** client credentials. Never committed — read from the
/// environment only.
pub struct GoogleOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

impl GoogleOAuthConfig {
    /// Reads `AMX_GOOGLE_CLIENT_ID` / `AMX_GOOGLE_CLIENT_SECRET` from the environment.
    pub fn from_env() -> Result<Self, AmxError> {
        let client_id =
            std::env::var("AMX_GOOGLE_CLIENT_ID").map_err(|_| AmxError::OAuthFlowFailed {
                reason:
                    "AMX_GOOGLE_CLIENT_ID is not set — create a Google Cloud OAuth Desktop app \
                     client and export its client ID"
                        .to_string(),
            })?;
        let client_secret =
            std::env::var("AMX_GOOGLE_CLIENT_SECRET").map_err(|_| AmxError::OAuthFlowFailed {
                reason:
                    "AMX_GOOGLE_CLIENT_SECRET is not set — export the Desktop app client's secret"
                        .to_string(),
            })?;
        Ok(Self {
            client_id,
            client_secret,
        })
    }
}

struct Pkce {
    verifier: String,
    challenge: String,
}

fn generate_pkce() -> Pkce {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(digest);
    Pkce {
        verifier,
        challenge,
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

fn parse_token_response(response: reqwest::blocking::Response) -> Result<TokenResponse, AmxError> {
    let status = response.status();
    let body = response.text().map_err(|err| AmxError::OAuthFlowFailed {
        reason: format!("could not read Google's token response body: {err}"),
    })?;
    if !status.is_success() {
        let reason = serde_json::from_str::<ErrorResponse>(&body)
            .map(|err| match err.error_description {
                Some(desc) => format!("{} ({desc})", err.error),
                None => err.error,
            })
            .unwrap_or(body);
        return Err(AmxError::OAuthFlowFailed {
            reason: format!("Google rejected the token request ({status}): {reason}"),
        });
    }
    serde_json::from_str(&body).map_err(|err| AmxError::OAuthFlowFailed {
        reason: format!("could not parse Google's token response: {err}"),
    })
}

/// Runs the interactive consent flow: opens a loopback listener, prints the consent URL for the
/// user to open, waits for the redirect carrying the authorization code, and exchanges it for a
/// refresh token (persisted by the caller via [`crate::credentials::CredentialBroker`]).
pub fn run_consent_flow(config: &GoogleOAuthConfig) -> Result<String, AmxError> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|err| AmxError::OAuthFlowFailed {
        reason: format!("could not open a loopback listener for the OAuth redirect: {err}"),
    })?;
    let port = listener
        .local_addr()
        .map_err(|err| AmxError::OAuthFlowFailed {
            reason: format!("could not read the loopback listener's port: {err}"),
        })?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let pkce = generate_pkce();

    let auth_url = format!(
        "{AUTH_ENDPOINT}?client_id={client_id}&redirect_uri={redirect_uri}&response_type=code\
         &scope={scope}&code_challenge={challenge}&code_challenge_method=S256\
         &access_type=offline&prompt=consent",
        client_id = config.client_id,
        redirect_uri = urlencoding_component(&redirect_uri),
        scope = urlencoding_component(SCOPE),
        challenge = pkce.challenge,
    );
    println!("Open this URL to authorize apple-mail-mcp:\n\n{auth_url}\n");

    let code = accept_redirect(&listener)?;
    exchange_code_for_refresh_token(config, &code, &redirect_uri, &pkce.verifier)
}

fn urlencoding_component(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn accept_redirect(listener: &TcpListener) -> Result<String, AmxError> {
    let (stream, _) = listener.accept().map_err(|err| AmxError::OAuthFlowFailed {
        reason: format!("did not receive the OAuth redirect: {err}"),
    })?;
    let mut reader =
        BufReader::new(
            stream
                .try_clone()
                .map_err(|err| AmxError::OAuthFlowFailed {
                    reason: format!("could not clone the redirect connection: {err}"),
                })?,
        );
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|err| AmxError::OAuthFlowFailed {
            reason: format!("could not read the OAuth redirect request: {err}"),
        })?;

    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| AmxError::OAuthFlowFailed {
            reason: format!("malformed OAuth redirect request line: {request_line:?}"),
        })?;
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let code = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("code="))
        .ok_or_else(|| AmxError::OAuthFlowFailed {
            reason: format!("OAuth redirect carried no `code` parameter: {path:?}"),
        })?
        .to_string();

    let mut stream = stream;
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n\
          apple-mail-mcp: authorization received, you can close this tab.",
    );
    Ok(code)
}

fn exchange_code_for_refresh_token(
    config: &GoogleOAuthConfig,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<String, AmxError> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", code_verifier),
        ])
        .send()
        .map_err(|err| AmxError::OAuthFlowFailed {
            reason: format!("could not reach Google's token endpoint: {err}"),
        })?;
    let token = parse_token_response(response)?;
    token
        .refresh_token
        .ok_or_else(|| AmxError::OAuthFlowFailed {
            reason:
                "Google's token response carried no refresh_token — retry with `prompt=consent`"
                    .to_string(),
        })
}

/// Exchanges a stored refresh token for a fresh access token.
pub fn refresh_access_token(
    config: &GoogleOAuthConfig,
    refresh_token: &str,
) -> Result<String, AmxError> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .map_err(|err| AmxError::OAuthFlowFailed {
            reason: format!("could not reach Google's token endpoint: {err}"),
        })?;
    let token = parse_token_response(response)?;
    Ok(token.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_the_sha256_of_the_verifier() {
        let pkce = generate_pkce();
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.verifier.as_bytes()));
        assert_eq!(pkce.challenge, expected);
        assert!(!pkce.verifier.is_empty());
    }

    #[test]
    fn pkce_verifiers_are_not_reused() {
        let a = generate_pkce();
        let b = generate_pkce();
        assert_ne!(a.verifier, b.verifier);
    }

    #[test]
    fn url_encoding_escapes_reserved_characters() {
        let encoded = urlencoding_component("http://127.0.0.1:9/ mail.google.com/");
        assert!(!encoded.contains(':'));
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains(' '));
    }
}
