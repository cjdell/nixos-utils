mod handlers;
mod html;
mod oidc;
mod podman;
mod state;

use std::collections::HashMap;
use std::sync::Arc;

use state::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();

    let config = state::Config::from_env();
    tracing::info!(
        listen = %config.listen_addr,
        auth = %config.auth_enabled(),
        base = %config.base_url,
        "starting container-ui"
    );

    let http = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("build http client");

    let oidc = config
        .oidc
        .as_ref()
        .map(|c| Arc::new(oidc::Oidc::new(c.clone())));

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("bind listen address");

    let app_state = AppState {
        config: config.clone(),
        http,
        podman: podman::Podman::new(&config),
        oidc,
        sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        pending: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
    };

    axum::serve(listener, handlers::router(app_state))
        .await
        .expect("http server failed");
}
