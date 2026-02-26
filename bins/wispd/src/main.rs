use std::{
    collections::VecDeque,
    fs,
    panic::{AssertUnwindSafe, catch_unwind, set_hook, take_hook},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use anyhow::{Result, anyhow};
use iced::widget::{column, container, text};
use iced::{Background, Color, Element, Length, Subscription, Task, border};
use iced_layershell::application;
use iced_layershell::reexport::{Anchor, Layer};
use iced_layershell::settings::{LayerShellSettings, Settings};
use iced_layershell::to_layer_message;
use serde::Deserialize;
use tracing::{info, warn};
use wisp_source::{SourceConfig, WispSource};
use wisp_types::{Notification, NotificationEvent, Urgency};

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct AppConfig {
    source: SourceSection,
    ui: UiSection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct SourceSection {
    default_timeout_ms: Option<i32>,
    capabilities: Vec<String>,
}

impl Default for SourceSection {
    fn default() -> Self {
        Self {
            default_timeout_ms: None,
            capabilities: vec!["body".to_string()],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct UiSection {
    format: String,
    max_visible: usize,
    width: u32,
    gap: u16,
    padding: u16,
    font_size: u16,
    anchor: String,
    margin: MarginConfig,
    colors: UrgencyColors,
}

impl Default for UiSection {
    fn default() -> Self {
        Self {
            format: "{app_name}: {summary}\n{body}".to_string(),
            max_visible: 5,
            width: 420,
            gap: 8,
            padding: 10,
            font_size: 15,
            anchor: "top-right".to_string(),
            margin: MarginConfig::default(),
            colors: UrgencyColors::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct MarginConfig {
    top: i32,
    right: i32,
    bottom: i32,
    left: i32,
}

impl Default for MarginConfig {
    fn default() -> Self {
        Self {
            top: 16,
            right: 16,
            bottom: 16,
            left: 16,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct UrgencyColors {
    low: String,
    normal: String,
    critical: String,
    background: String,
    text: String,
}

impl Default for UrgencyColors {
    fn default() -> Self {
        Self {
            low: "#6aa9ff".to_string(),
            normal: "#7dcf7d".to_string(),
            critical: "#ff6b6b".to_string(),
            background: "#1e1e2ecc".to_string(),
            text: "#f8f8f2".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct UiNotification {
    id: u32,
    app_name: String,
    summary: String,
    body: String,
    urgency: Urgency,
}

#[derive(Debug)]
struct WispdUi {
    events: Arc<Mutex<mpsc::Receiver<NotificationEvent>>>,
    notifications: VecDeque<UiNotification>,
    ui: UiSection,
}

impl WispdUi {
    fn new(events: Arc<Mutex<mpsc::Receiver<NotificationEvent>>>, ui: UiSection) -> Self {
        Self {
            events,
            notifications: VecDeque::new(),
            ui,
        }
    }

    fn on_tick(&mut self) {
        let mut pending = Vec::new();

        if let Ok(receiver) = self.events.lock() {
            loop {
                match receiver.try_recv() {
                    Ok(event) => pending.push(event),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        warn!("event channel disconnected");
                        break;
                    }
                }
            }
        }

        let processed = pending.len();
        for event in pending {
            self.apply_event(event);
        }

        if processed > 0 {
            info!(
                processed,
                visible = self.notifications.len(),
                "ui state updated"
            );
        }
    }

    fn apply_event(&mut self, event: NotificationEvent) {
        match event {
            NotificationEvent::Received { id, notification } => {
                self.insert_new(id, *notification);
            }
            NotificationEvent::Replaced { id, current, .. } => {
                if let Some(existing) = self.notifications.iter_mut().find(|n| n.id == id) {
                    *existing = to_ui_notification(id, *current);
                }
            }
            NotificationEvent::Closed { id, .. } => {
                self.notifications.retain(|n| n.id != id);
            }
            NotificationEvent::ActionInvoked { .. } => {}
        }
    }

    fn insert_new(&mut self, id: u32, notification: Notification) {
        self.notifications
            .push_front(to_ui_notification(id, notification));
        while self.notifications.len() > self.ui.max_visible {
            let _ = self.notifications.pop_back();
        }
    }
}

#[to_layer_message]
#[derive(Debug, Clone)]
enum Message {
    Tick,
}

fn namespace() -> String {
    String::from("wispd")
}

fn subscription(_: &WispdUi) -> Subscription<Message> {
    iced::time::every(Duration::from_millis(80)).map(|_| Message::Tick)
}

fn update(state: &mut WispdUi, message: Message) -> Task<Message> {
    match message {
        Message::Tick => state.on_tick(),
        _ => unreachable!(),
    }
    Task::none()
}

fn view(state: &WispdUi) -> Element<'_, Message> {
    let mut stack = column![].spacing(state.ui.gap as u32).width(Length::Shrink);

    for n in &state.notifications {
        let formatted = render_format(&state.ui.format, n);
        let border_color = urgency_color(&state.ui.colors, n.urgency.clone());
        let bg_color = parse_hex_color(&state.ui.colors.background)
            .unwrap_or(Color::from_rgba(0.12, 0.12, 0.18, 0.8));
        let text_color = parse_hex_color(&state.ui.colors.text).unwrap_or(Color::WHITE);
        let card_width = state.ui.width as f32;
        let card_padding = state.ui.padding;
        let font_size = state.ui.font_size as u32;

        let card = container(text(formatted).size(font_size))
            .padding(card_padding)
            .width(Length::Fixed(card_width))
            .style(move |_| {
                iced::widget::container::Style::default()
                    .background(Background::Color(bg_color))
                    .color(text_color)
                    .border(border::width(2).color(border_color).rounded(10))
            });

        stack = stack.push(card);
    }

    container(stack).width(Length::Shrink).into()
}

fn to_ui_notification(id: u32, notification: Notification) -> UiNotification {
    UiNotification {
        id,
        app_name: notification.app_name,
        summary: notification.summary,
        body: notification.body,
        urgency: notification.urgency,
    }
}

fn render_format(format: &str, n: &UiNotification) -> String {
    format
        .replace("{id}", &n.id.to_string())
        .replace("{app_name}", &n.app_name)
        .replace("{summary}", &n.summary)
        .replace("{body}", &n.body)
        .replace("{urgency}", urgency_label(n.urgency.clone()))
}

fn urgency_label(urgency: Urgency) -> &'static str {
    match urgency {
        Urgency::Low => "low",
        Urgency::Normal => "normal",
        Urgency::Critical => "critical",
    }
}

fn urgency_color(colors: &UrgencyColors, urgency: Urgency) -> Color {
    let fallback = match urgency {
        Urgency::Low => Color::from_rgb(0.42, 0.66, 1.0),
        Urgency::Normal => Color::from_rgb(0.49, 0.81, 0.49),
        Urgency::Critical => Color::from_rgb(1.0, 0.42, 0.42),
    };

    let selected = match urgency {
        Urgency::Low => &colors.low,
        Urgency::Normal => &colors.normal,
        Urgency::Critical => &colors.critical,
    };

    parse_hex_color(selected).unwrap_or(fallback)
}

fn parse_hex_color(raw: &str) -> Option<Color> {
    let hex = raw.trim().trim_start_matches('#');
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::from_rgb8(r, g, b))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Color::from_rgba8(r, g, b, a as f32 / 255.0))
        }
        _ => None,
    }
}

fn layer_anchor_from_str(anchor: &str) -> Anchor {
    match anchor {
        "top-left" => Anchor::Top | Anchor::Left,
        "top-right" => Anchor::Top | Anchor::Right,
        "bottom-left" => Anchor::Bottom | Anchor::Left,
        "bottom-right" => Anchor::Bottom | Anchor::Right,
        "top" => Anchor::Top,
        "bottom" => Anchor::Bottom,
        "left" => Anchor::Left,
        "right" => Anchor::Right,
        _ => Anchor::Top | Anchor::Right,
    }
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| {
                let mut p = PathBuf::from(home);
                p.push(".config");
                p
            })
        })
        .unwrap_or_else(|| PathBuf::from("."));

    base.join("wispd").join("config.toml")
}

fn load_config() -> AppConfig {
    let path = config_path();
    let Ok(raw) = fs::read_to_string(&path) else {
        info!(path = %path.display(), "config not found, using defaults");
        return AppConfig::default();
    };

    match toml::from_str::<AppConfig>(&raw) {
        Ok(cfg) => {
            info!(path = %path.display(), "loaded config");
            cfg
        }
        Err(err) => {
            warn!(path = %path.display(), %err, "failed to parse config, using defaults");
            AppConfig::default()
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let app_cfg = load_config();

    let source_cfg = SourceConfig {
        default_timeout_ms: app_cfg.source.default_timeout_ms,
        capabilities: app_cfg.source.capabilities.clone(),
        ..SourceConfig::default()
    };

    let (ui_tx, ui_rx) = mpsc::channel::<NotificationEvent>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<SourceConfig, String>>();

    std::thread::Builder::new()
        .name("wispd-source".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    let _ = ready_tx.send(Err(format!("failed to build tokio runtime: {err}")));
                    return;
                }
            };

            runtime.block_on(async move {
                info!("source thread runtime started");
                let (source_handle, mut source_events, dbus_service) =
                    match WispSource::start_dbus(source_cfg.clone()).await {
                        Ok(parts) => parts,
                        Err(err) => {
                            let _ = ready_tx
                                .send(Err(format!("failed to start wisp source over dbus: {err}")));
                            return;
                        }
                    };

                info!(dbus_name = %source_cfg.dbus_name, "source thread dbus initialized");
                let _ = ready_tx.send(Ok(source_cfg.clone()));

                while let Some(event) = source_events.recv().await {
                    if ui_tx.send(event).is_err() {
                        warn!("ui channel receiver dropped; stopping source forwarder");
                        break;
                    }
                }

                info!("source thread event forwarder exiting");
                drop((source_handle, dbus_service));
            });
        })
        .map_err(|err| anyhow!("failed to spawn source thread: {err}"))?;

    let source_runtime_cfg = match ready_rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(cfg)) => cfg,
        Ok(Err(err)) => return Err(anyhow!(err)),
        Err(err) => return Err(anyhow!("source thread did not initialize in time: {err}")),
    };

    info!(
        dbus_name = %source_runtime_cfg.dbus_name,
        dbus_path = %source_runtime_cfg.dbus_path,
        "wispd ui started"
    );

    let events = Arc::new(Mutex::new(ui_rx));
    let boot_events = Arc::clone(&events);
    let ui_cfg = app_cfg.ui.clone();

    let settings = Settings {
        layer_settings: LayerShellSettings {
            anchor: layer_anchor_from_str(&app_cfg.ui.anchor),
            layer: Layer::Overlay,
            exclusive_zone: 0,
            margin: (
                app_cfg.ui.margin.top,
                app_cfg.ui.margin.right,
                app_cfg.ui.margin.bottom,
                app_cfg.ui.margin.left,
            ),
            size: Some((app_cfg.ui.width, 0)),
            ..Default::default()
        },
        ..Default::default()
    };

    let app = application(
        move || WispdUi::new(Arc::clone(&boot_events), ui_cfg.clone()),
        namespace,
        update,
        view,
    )
    .subscription(subscription)
    .settings(settings);

    let default_hook = take_hook();
    set_hook(Box::new(|_| {}));
    let run_result = catch_unwind(AssertUnwindSafe(|| app.run()));
    set_hook(default_hook);

    match run_result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(anyhow!("failed to run iced layer-shell app: {err}")),
        Err(_) => Err(anyhow!(
            "wispd ui panicked while creating layer-shell window. Make sure you are running inside a Wayland session and have Wayland runtime libraries available (e.g. `wayland`, `libxkbcommon`)."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_types::CloseReason;

    fn sample(id: u32, summary: &str) -> NotificationEvent {
        NotificationEvent::Received {
            id,
            notification: Box::new(Notification {
                app_name: String::from("app"),
                app_icon: String::new(),
                summary: summary.to_string(),
                body: String::new(),
                urgency: Urgency::Normal,
                timeout_ms: 1000,
                actions: vec![],
                hints: Default::default(),
            }),
        }
    }

    #[test]
    fn newest_goes_to_front() {
        let (_tx, rx) = mpsc::channel();
        let mut ui = WispdUi::new(Arc::new(Mutex::new(rx)), UiSection::default());

        ui.apply_event(sample(1, "one"));
        ui.apply_event(sample(2, "two"));

        assert_eq!(ui.notifications[0].id, 2);
        assert_eq!(ui.notifications[1].id, 1);
    }

    #[test]
    fn replacement_keeps_slot() {
        let (_tx, rx) = mpsc::channel();
        let mut ui = WispdUi::new(Arc::new(Mutex::new(rx)), UiSection::default());

        ui.apply_event(sample(1, "one"));
        ui.apply_event(sample(2, "two"));
        ui.apply_event(NotificationEvent::Replaced {
            id: 1,
            previous: Box::new(Notification::default()),
            current: Box::new(Notification {
                summary: String::from("one-new"),
                ..Notification::default()
            }),
        });

        assert_eq!(ui.notifications[1].id, 1);
        assert_eq!(ui.notifications[1].summary, "one-new");
    }

    #[test]
    fn close_removes_notification() {
        let (_tx, rx) = mpsc::channel();
        let mut ui = WispdUi::new(Arc::new(Mutex::new(rx)), UiSection::default());

        ui.apply_event(sample(1, "one"));
        ui.apply_event(NotificationEvent::Closed {
            id: 1,
            reason: CloseReason::ClosedByCall,
        });

        assert!(ui.notifications.is_empty());
    }

    #[test]
    fn format_string_substitutes_placeholders() {
        let n = UiNotification {
            id: 9,
            app_name: "mail".to_string(),
            summary: "new message".to_string(),
            body: "hello".to_string(),
            urgency: Urgency::Critical,
        };

        let rendered = render_format("{id} {app_name} {summary} {body} {urgency}", &n);
        assert_eq!(rendered, "9 mail new message hello critical");
    }
}
