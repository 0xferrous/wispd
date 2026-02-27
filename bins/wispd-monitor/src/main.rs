use anyhow::Result;
use futures_util::StreamExt;
use tokio::signal;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use wisp_monitor::{
    NotificationMessage, become_monitor, parse_notification_message, rules_all_notifications,
};
use zbus::MessageStream;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("wispd_monitor=info".parse()?))
        .init();

    let conn = zbus::Connection::session().await?;
    become_monitor(&conn, rules_all_notifications()).await?;

    info!("wispd-monitor attached to session bus without owning org.freedesktop.Notifications");
    info!("monitoring Notify/CloseNotification calls and NotificationClosed/ActionInvoked signals");

    let mut stream = MessageStream::from(&conn);
    let mut shutdown = Box::pin(signal::ctrl_c());

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                info!("received Ctrl+C; exiting");
                break;
            }
            maybe_msg = stream.next() => {
                let Some(msg) = maybe_msg else {
                    warn!("dbus message stream ended");
                    break;
                };

                let Ok(msg) = msg else {
                    warn!(error = %msg.unwrap_err(), "failed to decode dbus message");
                    continue;
                };

                match parse_notification_message(&msg) {
                    Ok(Some(NotificationMessage::Notify(call))) => {
                        info!(
                            kind = "Notify",
                            app_name = %call.app_name,
                            replaces_id = call.replaces_id,
                            summary = %call.summary,
                            body = %call.body,
                            action_pairs = call.actions.len() / 2,
                            expire_timeout = call.expire_timeout,
                        );
                    }
                    Ok(Some(NotificationMessage::CloseNotification { id })) => {
                        info!(kind = "CloseNotification", id);
                    }
                    Ok(Some(NotificationMessage::NotificationClosed { id, reason })) => {
                        info!(kind = "NotificationClosed", id, reason);
                    }
                    Ok(Some(NotificationMessage::ActionInvoked { id, action_key })) => {
                        info!(kind = "ActionInvoked", id, action_key = %action_key);
                    }
                    Ok(None) => {}
                    Err(err) => warn!(?err, "failed to parse notifications message"),
                }
            }
        }
    }

    Ok(())
}
