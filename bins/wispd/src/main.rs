use anyhow::Result;
use tokio::signal;
use tracing::info;
use wisp_source::{SourceConfig, WispSource};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cfg = SourceConfig::default();
    let (source, mut events, _dbus) = WispSource::start_dbus(cfg).await?;
    info!(capabilities = ?source.capabilities(), "wispd started");

    let event_task = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            info!(?event, "notification event");
        }
    });

    info!("press Ctrl+C to stop");
    signal::ctrl_c().await?;
    event_task.abort();

    Ok(())
}
