use std::io::{self, BufRead};

use anyhow::Result;
use tokio::{signal, sync::mpsc};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use wisp_source::{SourceConfig, WispSource};
use wisp_types::CloseReason;

#[derive(Debug, Clone, PartialEq, Eq)]
enum DebugCommand {
    Help,
    List,
    Close(u32),
    Action { id: u32, key: String },
    Quit,
}

fn parse_command(line: &str) -> Result<Option<DebugCommand>, String> {
    let mut parts = line.split_whitespace();
    let Some(cmd) = parts.next() else {
        return Ok(None);
    };

    match cmd {
        "help" => Ok(Some(DebugCommand::Help)),
        "list" => Ok(Some(DebugCommand::List)),
        "quit" | "exit" => Ok(Some(DebugCommand::Quit)),
        "close" => {
            let id = parts
                .next()
                .ok_or_else(|| "usage: close <id>".to_string())?
                .parse::<u32>()
                .map_err(|_| "id must be a positive integer".to_string())?;
            Ok(Some(DebugCommand::Close(id)))
        }
        "action" => {
            let id = parts
                .next()
                .ok_or_else(|| "usage: action <id> <action-key>".to_string())?
                .parse::<u32>()
                .map_err(|_| "id must be a positive integer".to_string())?;
            let key = parts
                .next()
                .ok_or_else(|| "usage: action <id> <action-key>".to_string())?
                .to_string();
            Ok(Some(DebugCommand::Action { id, key }))
        }
        _ => Err("unknown command; use: help, list, close, action, quit".to_string()),
    }
}

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
    info!("commands: help | list | close <id> | action <id> <action-key> | quit");

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<DebugCommand>();
    tokio::task::spawn_blocking(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => match parse_command(&line) {
                    Ok(Some(cmd)) => {
                        if cmd_tx.send(cmd.clone()).is_err() {
                            break;
                        }
                        if cmd == DebugCommand::Quit {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(err) => eprintln!("{err}"),
                },
                Err(err) => {
                    eprintln!("failed to read stdin: {err}");
                    break;
                }
            }
        }
    });

    let mut shutdown = Box::pin(signal::ctrl_c());
    loop {
        tokio::select! {
            maybe_event = events.recv() => {
                let Some(event) = maybe_event else {
                    warn!("event stream ended");
                    break;
                };
                info!(?event, "notification event");
            }
            maybe_cmd = cmd_rx.recv() => {
                let Some(cmd) = maybe_cmd else {
                    warn!("command stream ended");
                    break;
                };

                match cmd {
                    DebugCommand::Help => {
                        info!("commands: help | list | close <id> | action <id> <action-key> | quit");
                    }
                    DebugCommand::List => {
                        let snapshot = source.snapshot().await;
                        info!(count = snapshot.len(), "current notifications");
                        for (id, n) in snapshot {
                            info!(id, app = %n.app_name, summary = %n.summary, "notification");
                        }
                    }
                    DebugCommand::Close(id) => {
                        let closed = source.close(id, CloseReason::ClosedByCall).await?;
                        info!(id, closed, "close command handled");
                    }
                    DebugCommand::Action { id, key } => {
                        let invoked = source.invoke_action(id, &key).await?;
                        info!(id, action_key = %key, invoked, "action command handled");
                    }
                    DebugCommand::Quit => {
                        info!("quitting");
                        break;
                    }
                }
            }
            _ = &mut shutdown => {
                info!("received Ctrl+C");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_returns_none() {
        assert_eq!(parse_command("   "), Ok(None));
    }

    #[test]
    fn parse_close_command() {
        assert_eq!(parse_command("close 42"), Ok(Some(DebugCommand::Close(42))));
    }

    #[test]
    fn parse_action_command() {
        assert_eq!(
            parse_command("action 7 open"),
            Ok(Some(DebugCommand::Action {
                id: 7,
                key: "open".to_string()
            }))
        );
    }
}
