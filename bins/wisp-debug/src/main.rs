use anyhow::Result;
use tokio::signal;
use tracing::info;
use tracing_subscriber::EnvFilter;
use wisp_source::{SourceConfig, WispSource};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("wisp_debug=info".parse()?))
        .init();

    let cfg = SourceConfig::default();
    let (source, mut events, _dbus) = WispSource::start_dbus(cfg.clone()).await?;

    info!(
        dbus_name = %cfg.dbus_name,
        dbus_path = %cfg.dbus_path,
        capabilities = ?source.capabilities(),
        "wisp-debug listening for notifications"
    );
    info!("send one with: notify-send 'hello from notify-send'");

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
