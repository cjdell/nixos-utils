mod api;
mod metrics;

use std::time::Duration;
use api::AppState;

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();

    let listen = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:8092".into());
    let static_dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "web/dist".into());
    let sample_interval_ms = env_u64("SAMPLE_INTERVAL_MS", 1000);
    let fast_retention = env_u64("FAST_RETENTION_SECS", 3600) as usize;
    let slow_interval = env_u64("SLOW_INTERVAL_SECS", 10).max(1);
    let slow_retention = env_u64("SLOW_RETENTION_SECS", 86400) as usize;

    let sampler = metrics::Sampler::new(
        Duration::from_millis(sample_interval_ms),
        fast_retention,
        slow_interval,
        slow_retention,
    );
    let s = sampler.clone();
    tokio::spawn(async move { s.run().await });

    let (hostname, kernel) = metrics::host_info();
    tracing::info!(
        %listen,
        %hostname,
        %kernel,
        sample_interval_ms,
        fast_retention,
        slow_interval,
        slow_retention,
        static_dir,
        "starting health-backend"
    );

    let state = AppState {
        sampler,
        static_dir: static_dir.into(),
        hostname,
        kernel,
    };

    let listener = tokio::net::TcpListener::bind(&listen).await?;
    axum::serve(listener, api::router(state)).await?;
    Ok(())
}
