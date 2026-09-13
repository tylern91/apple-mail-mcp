//! Transport wiring for `amxcli serve` (Phase 3 task 9): stdio and streamable HTTP. Binding any
//! non-loopback address requires a bearer token or the server refuses to start (umbrella §8) —
//! stricter than `StreamableHttpServerConfig`'s own `allowed_hosts` default, which guards against
//! DNS rebinding on the Host header rather than open network exposure.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};

use crate::server::AmxServer;

#[derive(Debug, Clone)]
pub struct HttpServeConfig {
    pub bind: SocketAddr,
    pub token: Option<String>,
}

pub fn require_token_for_non_loopback(bind: SocketAddr, token: Option<&str>) -> Result<(), String> {
    if !bind.ip().is_loopback() && token.is_none() {
        return Err(format!(
            "refusing to bind {bind}: non-loopback addresses require --token (umbrella §8)"
        ));
    }
    Ok(())
}

pub async fn serve_stdio(server: AmxServer) -> std::io::Result<()> {
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(std::io::Error::other)?;
    running.waiting().await.map_err(std::io::Error::other)?;
    Ok(())
}

async fn bearer_auth(
    State(token): State<Arc<str>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|presented| presented == token.as_ref());

    if authorized {
        next.run(request).await
    } else {
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .expect("static response body")
    }
}

pub async fn serve_http(server: AmxServer, config: HttpServeConfig) -> std::io::Result<()> {
    require_token_for_non_loopback(config.bind, config.token.as_deref())
        .map_err(std::io::Error::other)?;

    let session_manager = Arc::new(LocalSessionManager::default());
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        session_manager,
        StreamableHttpServerConfig::default(),
    );

    let mut router = axum::Router::new().fallback_service(service);
    if let Some(token) = config.token.clone() {
        router = router.layer(middleware::from_fn_with_state(
            Arc::<str>::from(token),
            bearer_auth,
        ));
    }

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    axum::serve(listener, router).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_bind_needs_no_token() {
        let bind: SocketAddr = "127.0.0.1:9800".parse().unwrap();
        assert!(require_token_for_non_loopback(bind, None).is_ok());
    }

    #[test]
    fn loopback_v6_bind_needs_no_token() {
        let bind: SocketAddr = "[::1]:9800".parse().unwrap();
        assert!(require_token_for_non_loopback(bind, None).is_ok());
    }

    #[test]
    fn non_loopback_bind_without_token_is_refused() {
        let bind: SocketAddr = "0.0.0.0:9800".parse().unwrap();
        let err = require_token_for_non_loopback(bind, None).unwrap_err();
        assert!(err.contains("non-loopback"));
    }

    #[test]
    fn non_loopback_bind_with_token_is_accepted() {
        let bind: SocketAddr = "0.0.0.0:9800".parse().unwrap();
        assert!(require_token_for_non_loopback(bind, Some("secret")).is_ok());
    }
}
