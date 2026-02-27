use std::{
    env,
    io::Read,
    net::{TcpStream, ToSocketAddrs},
    sync::mpsc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use ssh2::Session;
use tokio::{net, signal, time};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use wisp_monitor::{
    NotificationMessage, become_monitor, parse_notification_message, rules_notify_only,
};
use zbus::MessageStream;

#[derive(Debug, Clone)]
struct ForwardConfig {
    ssh_host: String,
    ssh_port: u16,
    ssh_user: String,
    ssh_password: String,
    remote_notify_send: String,
    startup_wait_secs: u64,
    startup_poll_interval_ms: u64,
}

impl ForwardConfig {
    fn from_env() -> Result<Self> {
        let ssh_host =
            env::var("WISPD_FORWARD_SSH_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());

        let ssh_port = env::var("WISPD_FORWARD_SSH_PORT")
            .ok()
            .map(|s| s.parse::<u16>())
            .transpose()
            .context("WISPD_FORWARD_SSH_PORT must be a valid u16")?
            .unwrap_or(2222);

        let ssh_user = env::var("WISPD_FORWARD_SSH_USER").unwrap_or_else(|_| "wisp".to_string());
        let ssh_password =
            env::var("WISPD_FORWARD_SSH_PASSWORD").unwrap_or_else(|_| "wisp".to_string());
        let remote_notify_send =
            env::var("WISPD_FORWARD_NOTIFY_SEND").unwrap_or_else(|_| "notify-send".to_string());

        let startup_wait_secs = env::var("WISPD_FORWARD_SSH_STARTUP_WAIT_SECS")
            .ok()
            .map(|s| s.parse::<u64>())
            .transpose()
            .context("WISPD_FORWARD_SSH_STARTUP_WAIT_SECS must be a valid u64")?
            .unwrap_or(60);

        let startup_poll_interval_ms = env::var("WISPD_FORWARD_SSH_STARTUP_POLL_MS")
            .ok()
            .map(|s| s.parse::<u64>())
            .transpose()
            .context("WISPD_FORWARD_SSH_STARTUP_POLL_MS must be a valid u64")?
            .unwrap_or(500);

        Ok(Self {
            ssh_host,
            ssh_port,
            ssh_user,
            ssh_password,
            remote_notify_send,
            startup_wait_secs,
            startup_poll_interval_ms,
        })
    }
}

#[derive(Debug, Clone)]
struct ForwardPayload {
    app_name: String,
    summary: String,
    body: String,
    expire_timeout: i32,
    urgency: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("wispd_forward=info".parse()?))
        .init();

    let cfg = ForwardConfig::from_env()?;
    info!(
        ssh_host = %cfg.ssh_host,
        ssh_port = cfg.ssh_port,
        ssh_user = %cfg.ssh_user,
        startup_wait_secs = cfg.startup_wait_secs,
        "starting notification forwarder"
    );

    wait_for_ssh_startup(&cfg).await?;

    let (tx, rx) = mpsc::channel::<ForwardPayload>();
    let worker_cfg = cfg.clone();
    let worker = std::thread::spawn(move || run_forward_worker(worker_cfg, rx));

    let conn = zbus::Connection::session().await?;
    become_monitor(&conn, rules_notify_only()).await?;

    info!("attached to session bus; forwarding Notify calls to VM");

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
                    warn!("dbus stream ended");
                    break;
                };

                let Ok(msg) = msg else {
                    warn!(error = %msg.unwrap_err(), "failed to decode dbus message");
                    continue;
                };

                let Ok(parsed) = parse_notification_message(&msg) else {
                    warn!("failed to parse monitored message");
                    continue;
                };

                let Some(NotificationMessage::Notify(call)) = parsed else {
                    continue;
                };

                let urgency = call
                    .hints
                    .get("urgency")
                    .and_then(|v| u8::try_from(v).ok())
                    .map(|u| match u {
                        0 => "low",
                        2 => "critical",
                        _ => "normal",
                    })
                    .unwrap_or("normal")
                    .to_string();

                let payload = ForwardPayload {
                    app_name: call.app_name,
                    summary: call.summary,
                    body: call.body,
                    expire_timeout: call.expire_timeout,
                    urgency,
                };

                if let Err(err) = tx.send(payload) {
                    warn!(?err, "forward worker channel closed");
                    break;
                }
            }
        }
    }

    drop(tx);
    let _ = worker.join();

    Ok(())
}

async fn wait_for_ssh_startup(cfg: &ForwardConfig) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(cfg.startup_wait_secs);
    let addr = format!("{}:{}", cfg.ssh_host, cfg.ssh_port);

    loop {
        match net::TcpStream::connect(addr.as_str()).await {
            Ok(_) => {
                info!(address = %addr, "ssh endpoint is reachable");
                return Ok(());
            }
            Err(err) => {
                if Instant::now() >= deadline {
                    anyhow::bail!("ssh endpoint {addr} not reachable within timeout: {err}");
                }
                time::sleep(Duration::from_millis(cfg.startup_poll_interval_ms)).await;
            }
        }
    }
}

fn run_forward_worker(cfg: ForwardConfig, rx: mpsc::Receiver<ForwardPayload>) {
    let mut session: Option<Session> = None;

    for payload in rx {
        if let Err(err) = forward_with_reconnect(&cfg, &mut session, &payload) {
            warn!(?err, app = %payload.app_name, summary = %payload.summary, "failed to forward notification");
        } else {
            info!(app_name = %payload.app_name, summary = %payload.summary, "forwarded notification");
        }
    }
}

fn forward_with_reconnect(
    cfg: &ForwardConfig,
    session: &mut Option<Session>,
    payload: &ForwardPayload,
) -> Result<()> {
    if session.is_none() {
        *session = Some(connect_session(cfg)?);
    }

    let first_try = session
        .as_mut()
        .context("ssh session unexpectedly absent")
        .and_then(|s| exec_notify(s, cfg, payload));

    if first_try.is_ok() {
        return Ok(());
    }

    warn!("ssh session failed; reconnecting and retrying once");
    *session = Some(connect_session(cfg)?);

    let s = session
        .as_mut()
        .context("ssh session unexpectedly absent after reconnect")?;
    exec_notify(s, cfg, payload)
}

fn connect_session(cfg: &ForwardConfig) -> Result<Session> {
    let addr = (cfg.ssh_host.as_str(), cfg.ssh_port)
        .to_socket_addrs()
        .context("failed to resolve ssh host")?
        .next()
        .context("no resolved ssh address")?;

    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .with_context(|| format!("failed to connect to {}:{}", cfg.ssh_host, cfg.ssh_port))?;
    tcp.set_read_timeout(Some(Duration::from_secs(5))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(5))).ok();

    let mut session = Session::new().context("failed to create ssh session")?;
    session.set_tcp_stream(tcp);
    session.handshake().context("ssh handshake failed")?;

    session
        .userauth_password(&cfg.ssh_user, &cfg.ssh_password)
        .with_context(|| format!("ssh password auth failed for {}", cfg.ssh_user))?;

    if !session.authenticated() {
        anyhow::bail!("ssh authentication failed");
    }

    Ok(session)
}

fn exec_notify(session: &mut Session, cfg: &ForwardConfig, payload: &ForwardPayload) -> Result<()> {
    let mut channel = session
        .channel_session()
        .context("failed to open ssh channel")?;

    let cmd = build_remote_notify_command(cfg, payload);
    channel
        .exec(&cmd)
        .with_context(|| format!("failed to exec remote command: {cmd}"))?;

    let mut stdout = String::new();
    let mut stderr = String::new();
    let _ = channel.read_to_string(&mut stdout);
    let _ = channel.stderr().read_to_string(&mut stderr);

    channel
        .wait_close()
        .context("failed waiting for ssh channel close")?;
    let status = channel
        .exit_status()
        .context("failed to read ssh channel exit status")?;

    if status != 0 {
        anyhow::bail!(
            "remote notify-send failed with status {status}, stderr: {}, stdout: {}",
            stderr.trim(),
            stdout.trim()
        );
    }

    Ok(())
}

fn build_remote_notify_command(cfg: &ForwardConfig, payload: &ForwardPayload) -> String {
    let mut cmd = format!(
        "{} -a {} -u {}",
        sh_quote(&cfg.remote_notify_send),
        sh_quote(&payload.app_name),
        sh_quote(&payload.urgency)
    );

    if payload.expire_timeout >= 0 {
        cmd.push_str(&format!(" -t {}", payload.expire_timeout));
    }

    cmd.push(' ');
    cmd.push_str(&sh_quote(&payload.summary));

    if !payload.body.is_empty() {
        cmd.push(' ');
        cmd.push_str(&sh_quote(&payload.body));
    }

    cmd
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}
