use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use rand::{RngExt, prelude::IndexedRandom, seq::SliceRandom};
use tracing::info;
use tracing_subscriber::EnvFilter;
use zbus::zvariant::{OwnedValue, Str};

const DBUS_NAME: &str = "org.freedesktop.Notifications";
const DBUS_PATH: &str = "/org/freedesktop/Notifications";
const DBUS_IFACE: &str = "org.freedesktop.Notifications";

const ADJECTIVES: &[&str] = &[
    "Amber", "Brisk", "Curious", "Distant", "Electric", "Feral", "Gentle", "Hidden", "Icy",
    "Jagged", "Kind", "Lunar", "Mellow", "Noisy", "Oblique", "Patient", "Quick", "Restless",
    "Solar", "Tender", "Uneven", "Velvet", "Wired", "Young",
];

const NOUNS: &[&str] = &[
    "Badger", "Comet", "Drift", "Engine", "Forest", "Garden", "Harbor", "Island", "Junction",
    "Kernel", "Lantern", "Meadow", "Needle", "Orbit", "Parade", "Quartz", "Rocket", "Signal",
    "Tunnel", "Umbra", "Valley", "Window", "Yarrow", "Zephyr",
];

const BODY_LINES: &[&str] = &[
    "Background sync finished ahead of schedule.",
    "A quiet process requested your attention.",
    "The queue has reshuffled itself again.",
    "Someone somewhere pressed the interesting button.",
    "This is a synthetic message for popup testing.",
    "The system insists everything is probably fine.",
    "One more event drifted in from the session bus.",
    "Rendering should now exercise spacing and wrapping.",
    "A harmless experiment is currently in progress.",
    "Tiny goblins updated the internal counters.",
];

const ACTION_LABELS: &[(&str, &str)] = &[
    ("default", "Open"),
    ("view", "View"),
    ("reply", "Reply"),
    ("archive", "Archive"),
    ("dismiss", "Dismiss"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Toggle {
    Random,
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Config {
    count: usize,
    interval_ms: u64,
    replace_id: Option<u32>,
    persistent_only: bool,
    actions: Toggle,
    icons: Toggle,
    loop_forever: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            count: 1,
            interval_ms: 0,
            replace_id: None,
            persistent_only: false,
            actions: Toggle::Random,
            icons: Toggle::Random,
            loop_forever: false,
        }
    }
}

fn parse_args() -> Result<Config> {
    let mut cfg = Config::default();
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-n" | "--count" => {
                let value = args.next().context("missing value for --count")?;
                cfg.count = value
                    .parse()
                    .context("--count must be a positive integer")?;
                if cfg.count == 0 {
                    bail!("--count must be greater than zero");
                }
            }
            "-i" | "--interval-ms" => {
                let value = args.next().context("missing value for --interval-ms")?;
                cfg.interval_ms = value
                    .parse()
                    .context("--interval-ms must be a non-negative integer")?;
            }
            "--replace-id" => {
                let value = args.next().context("missing value for --replace-id")?;
                cfg.replace_id = Some(
                    value
                        .parse()
                        .context("--replace-id must be a non-negative integer")?,
                );
            }
            "--persistent-only" => {
                cfg.persistent_only = true;
            }
            "--actions-always" => {
                cfg.actions = Toggle::Always;
            }
            "--actions-never" => {
                cfg.actions = Toggle::Never;
            }
            "--icons-always" => {
                cfg.icons = Toggle::Always;
            }
            "--icons-never" => {
                cfg.icons = Toggle::Never;
            }
            "--loop" => {
                cfg.loop_forever = true;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    Ok(cfg)
}

fn print_help() {
    println!(
        "wisp-random\n\nUSAGE:\n  wisp-random [OPTIONS]\n\nOPTIONS:\n  -n, --count N         Number of notifications to send (default: 1)\n  -i, --interval-ms MS  Delay between notifications in milliseconds (default: 0)\n      --replace-id ID   Reuse the same replaces_id for every notification\n      --persistent-only Force timeout = -1 for every notification\n      --actions-always  Always include action buttons\n      --actions-never   Never include action buttons\n      --icons-always    Always include an icon when one can be found\n      --icons-never     Never include an icon\n      --loop            Send notifications forever\n  -h, --help            Show this help\n"
    );
}

fn random_summary<R: RngExt + ?Sized>(rng: &mut R) -> String {
    format!(
        "{} {}",
        ADJECTIVES.choose(rng).expect("adjectives not empty"),
        NOUNS.choose(rng).expect("nouns not empty")
    )
}

fn random_body<R: RngExt + ?Sized>(rng: &mut R) -> String {
    let line_count = rng.random_range(1..=3);
    (0..line_count)
        .map(|_| *BODY_LINES.choose(rng).expect("body lines not empty"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn random_actions<R: RngExt + ?Sized>(rng: &mut R, toggle: Toggle) -> Vec<String> {
    let include = match toggle {
        Toggle::Always => true,
        Toggle::Never => false,
        Toggle::Random => !rng.random_bool(0.5),
    };

    if !include {
        return Vec::new();
    }

    let action_count = rng.random_range(1..=2);
    let mut pool = ACTION_LABELS.to_vec();
    pool.shuffle(rng);

    let mut actions = Vec::with_capacity(action_count * 2);
    for (key, label) in pool.into_iter().take(action_count) {
        actions.push(key.to_string());
        actions.push(label.to_string());
    }
    actions
}

fn discover_icon_files() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/run/current-system/sw/share/icons"),
        PathBuf::from("/run/current-system/sw/share/pixmaps"),
        PathBuf::from("/usr/share/icons"),
        PathBuf::from("/usr/share/pixmaps"),
    ];

    if let Some(data_dirs) = env::var_os("XDG_DATA_DIRS") {
        for dir in env::split_paths(&data_dirs) {
            roots.push(dir.join("icons"));
            roots.push(dir.join("pixmaps"));
        }
    }

    let mut found = Vec::new();
    for root in roots {
        collect_icon_files(&root, 0, &mut found);
        if found.len() >= 32 {
            break;
        }
    }
    found
}

fn collect_icon_files(root: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > 4 || found.len() >= 32 {
        return;
    }

    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        if found.len() >= 32 {
            break;
        }
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            collect_icon_files(&path, depth + 1, found);
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if matches!(ext, "png" | "svg" | "jpg" | "jpeg" | "webp") {
            found.push(path);
        }
    }
}

fn random_icon<R: RngExt + ?Sized>(rng: &mut R, icons: &[PathBuf], toggle: Toggle) -> String {
    let include = match toggle {
        Toggle::Always => true,
        Toggle::Never => false,
        Toggle::Random => !rng.random_bool(0.5),
    };

    if icons.is_empty() || !include {
        return String::new();
    }

    icons
        .choose(rng)
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

fn build_hints<R: RngExt + ?Sized>(rng: &mut R) -> HashMap<String, OwnedValue> {
    let mut hints = HashMap::new();

    if rng.random_bool(0.5) {
        hints.insert(
            "urgency".to_string(),
            OwnedValue::from(rng.random_range(0_u8..=2_u8)),
        );
    }

    if rng.random_bool(0.3) {
        hints.insert("transient".to_string(), OwnedValue::from(true));
    }

    if rng.random_bool(0.4) {
        hints.insert(
            "category".to_string(),
            OwnedValue::from(Str::from("test.random")),
        );
    }

    hints
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("wisp_random=info".parse()?))
        .init();

    let cfg = parse_args()?;
    let icons = discover_icon_files();
    let conn = zbus::Connection::session().await?;
    let mut rng = rand::rng();

    info!(
        count = cfg.count,
        interval_ms = cfg.interval_ms,
        replace_id = ?cfg.replace_id,
        persistent_only = cfg.persistent_only,
        actions = ?cfg.actions,
        icons = ?cfg.icons,
        loop_forever = cfg.loop_forever,
        discovered_icons = icons.len(),
        "sending random notifications"
    );

    let mut idx = 0_usize;
    loop {
        if !cfg.loop_forever && idx >= cfg.count {
            break;
        }

        let summary = random_summary(&mut rng);
        let body = random_body(&mut rng);
        let app_icon = random_icon(&mut rng, &icons, cfg.icons);
        let actions = random_actions(&mut rng, cfg.actions);
        let hints = build_hints(&mut rng);
        let timeout_ms = if cfg.persistent_only {
            -1
        } else if rng.random_bool(0.5) {
            rng.random_range(1500_i32..=8000_i32)
        } else {
            -1
        };

        let msg = conn
            .call_method(
                Some(DBUS_NAME),
                DBUS_PATH,
                Some(DBUS_IFACE),
                "Notify",
                &(
                    String::from("wisp-random"),
                    cfg.replace_id.unwrap_or(0),
                    app_icon.clone(),
                    summary.clone(),
                    body.clone(),
                    actions.clone(),
                    hints,
                    timeout_ms,
                ),
            )
            .await?;

        let id: u32 = msg.body().deserialize()?;
        idx += 1;
        info!(
            iteration = idx,
            id,
            summary = %summary,
            body_len = body.len(),
            has_icon = !app_icon.is_empty(),
            action_count = actions.len() / 2,
            timeout_ms,
            replace_id = cfg.replace_id.unwrap_or(0),
            "random notification sent"
        );

        if cfg.interval_ms > 0 && (cfg.loop_forever || idx < cfg.count) {
            tokio::time::sleep(Duration::from_millis(cfg.interval_ms)).await;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_defaults() {
        assert_eq!(
            Config::default(),
            Config {
                count: 1,
                interval_ms: 0,
                replace_id: None,
                persistent_only: false,
                actions: Toggle::Random,
                icons: Toggle::Random,
                loop_forever: false,
            }
        );
    }

    #[test]
    fn random_actions_are_even_pairs() {
        let mut rng = rand::rng();
        let actions = random_actions(&mut rng, Toggle::Random);
        assert_eq!(actions.len() % 2, 0);
    }

    #[test]
    fn icon_discovery_handles_missing_path() {
        let mut found = Vec::new();
        collect_icon_files(Path::new("/definitely/missing/path"), 0, &mut found);
        assert!(found.is_empty());
    }
}
