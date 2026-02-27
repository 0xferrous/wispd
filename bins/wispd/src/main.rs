use std::{
    collections::{HashMap, VecDeque},
    fs,
    panic::{AssertUnwindSafe, catch_unwind, set_hook, take_hook},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use iced::widget::{button, column, container, row, text};
use iced::{Background, Color, Element, Font, Length, Subscription, Task, border};
use iced_layershell::daemon;
use iced_layershell::reexport::{Anchor, IcedId, Layer, NewLayerShellSettings};
use iced_layershell::settings::{LayerShellSettings, Settings};
use iced_layershell::to_layer_message;
use serde::Deserialize;
use tokio::sync::mpsc as tokio_mpsc;
use tracing::{info, warn};
use wisp_source::{SourceConfig, WispSource};
use wisp_types::{Notification, NotificationAction, NotificationEvent, Urgency};

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
            capabilities: vec!["body".to_string(), "actions".to_string()],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct UiSection {
    format: String,
    max_visible: usize,
    width: u32,
    height: u32,
    gap: u16,
    padding: u16,
    font_size: u16,
    font_family: String,
    anchor: String,
    margin: MarginConfig,
    colors: UrgencyColors,
    show_timeout_progress: bool,
    timeout_progress_height: u16,
    timeout_progress_position: String,
}

impl Default for UiSection {
    fn default() -> Self {
        Self {
            format: "{app_name}: {summary}\n{body}".to_string(),
            max_visible: 5,
            width: 420,
            height: 64,
            gap: 8,
            padding: 10,
            font_size: 15,
            font_family: "sans-serif".to_string(),
            anchor: "top-right".to_string(),
            margin: MarginConfig::default(),
            colors: UrgencyColors::default(),
            show_timeout_progress: true,
            timeout_progress_height: 3,
            timeout_progress_position: "bottom".to_string(),
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
    timeout_progress: String,
}

impl Default for UrgencyColors {
    fn default() -> Self {
        Self {
            low: "#6aa9ff".to_string(),
            normal: "#7dcf7d".to_string(),
            critical: "#ff6b6b".to_string(),
            background: "#1e1e2ecc".to_string(),
            text: "#f8f8f2".to_string(),
            timeout_progress: "#f8f8f2".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct UiAction {
    key: String,
    label: String,
}

#[derive(Debug, Clone)]
struct UiNotification {
    id: u32,
    app_name: String,
    summary: String,
    body: String,
    urgency: Urgency,
    actions: Vec<UiAction>,
    timeout_ms: Option<u32>,
    created_at: Instant,
}

#[derive(Debug, Clone, Copy)]
struct WindowBinding {
    window_id: IcedId,
    notification_id: u32,
}

#[derive(Debug, Clone)]
enum SourceCommand {
    InvokeAction { id: u32, key: String },
    Dismiss { id: u32 },
}

#[derive(Debug)]
struct WispdUi {
    events: Arc<Mutex<mpsc::Receiver<NotificationEvent>>>,
    cmd_tx: tokio_mpsc::UnboundedSender<SourceCommand>,
    notifications: HashMap<u32, UiNotification>,
    windows: VecDeque<WindowBinding>,
    ui: UiSection,
    default_timeout_ms: Option<i32>,
}

impl WispdUi {
    fn new(
        events: Arc<Mutex<mpsc::Receiver<NotificationEvent>>>,
        cmd_tx: tokio_mpsc::UnboundedSender<SourceCommand>,
        ui: UiSection,
        default_timeout_ms: Option<i32>,
    ) -> Self {
        Self {
            events,
            cmd_tx,
            notifications: HashMap::new(),
            windows: VecDeque::new(),
            ui,
            default_timeout_ms,
        }
    }

    fn on_tick(&mut self) -> Task<Message> {
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
        let mut tasks = Vec::new();
        for event in pending {
            tasks.push(self.apply_event(event));
        }

        if processed > 0 {
            info!(processed, visible = self.windows.len(), "ui state updated");
        }

        Task::batch(tasks)
    }

    fn apply_event(&mut self, event: NotificationEvent) -> Task<Message> {
        match event {
            NotificationEvent::Received { id, notification } => self.insert_new(id, *notification),
            NotificationEvent::Replaced { id, current, .. } => {
                self.notifications.insert(
                    id,
                    to_ui_notification(id, *current, self.default_timeout_ms),
                );
                Task::none()
            }
            NotificationEvent::Closed { id, .. } => self.remove_notification(id),
            NotificationEvent::ActionInvoked { .. } => Task::none(),
        }
    }

    fn insert_new(&mut self, id: u32, notification: Notification) -> Task<Message> {
        self.notifications.insert(
            id,
            to_ui_notification(id, notification, self.default_timeout_ms),
        );

        if self.windows.iter().any(|w| w.notification_id == id) {
            return Task::none();
        }

        let mut tasks = Vec::new();
        let popup_height = self.popup_height_for_id(id);

        let (window_id, open_task) = Message::layershell_open(NewLayerShellSettings {
            size: Some((self.ui.width.max(1), popup_height.max(1))),
            layer: Layer::Top,
            anchor: layer_anchor_from_str(&self.ui.anchor),
            exclusive_zone: Some(0),
            margin: Some((
                self.ui.margin.top,
                self.ui.margin.right,
                self.ui.margin.bottom,
                self.ui.margin.left,
            )),
            ..Default::default()
        });
        self.windows.push_front(WindowBinding {
            window_id,
            notification_id: id,
        });
        tasks.push(open_task);

        while self.windows.len() > self.ui.max_visible {
            if let Some(evicted) = self.windows.pop_back() {
                self.notifications.remove(&evicted.notification_id);
                tasks.push(Task::done(Message::RemoveWindow(evicted.window_id)));
            }
        }

        tasks.push(self.relayout_task());
        Task::batch(tasks)
    }

    fn remove_notification(&mut self, id: u32) -> Task<Message> {
        self.notifications.remove(&id);

        if let Some(index) = self.windows.iter().position(|w| w.notification_id == id)
            && let Some(binding) = self.windows.remove(index)
        {
            return Task::batch([
                Task::done(Message::RemoveWindow(binding.window_id)),
                self.relayout_task(),
            ]);
        }

        Task::none()
    }

    fn relayout_task(&self) -> Task<Message> {
        let anchor = layer_anchor_from_str(&self.ui.anchor);
        let mut offset = 0_i32;

        let updates = self.windows.iter().map(|binding| {
            let popup_height = self.popup_height_for_id(binding.notification_id);
            let mut margin = (
                self.ui.margin.top,
                self.ui.margin.right,
                self.ui.margin.bottom,
                self.ui.margin.left,
            );

            if anchor.contains(Anchor::Top) {
                margin.0 += offset;
            } else {
                margin.2 += offset;
            }
            offset += popup_height as i32 + self.ui.gap as i32;

            Task::batch([
                Task::done(Message::MarginChange {
                    id: binding.window_id,
                    margin,
                }),
                Task::done(Message::AnchorSizeChange {
                    id: binding.window_id,
                    anchor,
                    size: (self.ui.width.max(1), popup_height.max(1)),
                }),
            ])
        });

        Task::batch(updates)
    }

    fn popup_height_for_id(&self, id: u32) -> u32 {
        self.notifications
            .get(&id)
            .map(|n| estimate_popup_height(&self.ui, n))
            .unwrap_or(self.ui.height.max(1))
    }

    fn timeout_progress_for(&self, id: u32) -> Option<f32> {
        let n = self.notifications.get(&id)?;
        let timeout_ms = n.timeout_ms?;
        let elapsed = n.created_at.elapsed().as_secs_f32() * 1000.0;
        let progress = (elapsed / timeout_ms as f32).clamp(0.0, 1.0);
        Some(progress)
    }
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    Tick,
    ActionClicked { id: u32, key: String },
    DismissClicked { id: u32 },
}

fn namespace() -> String {
    String::from("wispd")
}

fn subscription(_: &WispdUi) -> Subscription<Message> {
    iced::time::every(Duration::from_millis(33)).map(|_| Message::Tick)
}

fn update(state: &mut WispdUi, message: Message) -> Task<Message> {
    match message {
        Message::Tick => state.on_tick(),
        Message::ActionClicked { id, key } => {
            if let Err(err) = state.cmd_tx.send(SourceCommand::InvokeAction { id, key }) {
                warn!(?err, "failed to send action command to source thread");
            }
            Task::none()
        }
        Message::DismissClicked { id } => {
            if let Err(err) = state.cmd_tx.send(SourceCommand::Dismiss { id }) {
                warn!(?err, "failed to send dismiss command to source thread");
            }
            Task::none()
        }
        _ => Task::none(),
    }
}

fn app_style(_state: &WispdUi, theme: &iced::Theme) -> iced::theme::Style {
    iced::theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: theme.palette().text,
    }
}

fn view(state: &WispdUi, window_id: iced::window::Id) -> Element<'_, Message> {
    let Some(binding) = state.windows.iter().find(|w| w.window_id == window_id) else {
        return container(text(""))
            .width(Length::Fixed(1.0))
            .height(Length::Fixed(1.0))
            .style(|_| {
                iced::widget::container::Style::default()
                    .background(Background::Color(Color::TRANSPARENT))
            })
            .into();
    };

    let Some(n) = state.notifications.get(&binding.notification_id) else {
        return container(text(""))
            .width(Length::Fixed(1.0))
            .height(Length::Fixed(1.0))
            .style(|_| {
                iced::widget::container::Style::default()
                    .background(Background::Color(Color::TRANSPARENT))
            })
            .into();
    };

    let formatted = render_format(&state.ui.format, n);
    let border_color = urgency_color(&state.ui.colors, n.urgency.clone());
    let bg_color = parse_hex_color(&state.ui.colors.background)
        .unwrap_or(Color::from_rgba(0.12, 0.12, 0.18, 0.8));
    let text_color = parse_hex_color(&state.ui.colors.text).unwrap_or(Color::WHITE);
    let progress_color = parse_hex_color(&state.ui.colors.timeout_progress).unwrap_or(text_color);
    let card_width = state.ui.width as f32;
    let card_height = estimate_popup_height(&state.ui, n) as f32;
    let card_padding = state.ui.padding;
    let font_size = state.ui.font_size as u32;

    let font = resolve_font(&state.ui.font_family);

    let close_button = button(text("✕")).on_press(Message::DismissClicked { id: n.id });
    let header = row![
        container(text(formatted).size(font_size).font(font)).width(Length::Fill),
        close_button,
    ]
    .spacing(8);

    let mut card_content = column![header].spacing(8);

    if !n.actions.is_empty() {
        for action_chunk in n.actions.chunks(3) {
            let mut actions_row = row![].spacing(8);
            for action in action_chunk {
                actions_row = actions_row.push(button(text(action.label.clone())).on_press(
                    Message::ActionClicked {
                        id: n.id,
                        key: action.key.clone(),
                    },
                ));
            }
            card_content = card_content.push(actions_row);
        }
    }

    let body = container(card_content)
        .padding(card_padding)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| iced::widget::container::Style::default().color(text_color));

    let timeout_progress = state
        .timeout_progress_for(n.id)
        .filter(|_| state.ui.show_timeout_progress);

    let progress_height = state.ui.timeout_progress_height.max(1) as f32;

    let card_stack = if let Some(progress) = timeout_progress {
        let fill_width = (card_width * progress).max(0.0);
        let fill = container(text(""))
            .width(Length::Fixed(fill_width))
            .height(Length::Fixed(progress_height))
            .style(move |_| {
                iced::widget::container::Style::default()
                    .background(Background::Color(progress_color))
            });
        let empty = container(text(""))
            .width(Length::Fill)
            .height(Length::Fixed(progress_height))
            .style(|_| {
                iced::widget::container::Style::default()
                    .background(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.08)))
            });
        let progress_bar = row![fill, empty].spacing(0);

        if state
            .ui
            .timeout_progress_position
            .eq_ignore_ascii_case("top")
        {
            column![progress_bar, body]
        } else {
            column![body, progress_bar]
        }
    } else {
        column![body]
    };

    let card = container(card_stack)
        .width(Length::Fixed(card_width))
        .height(Length::Fixed(card_height))
        .style(move |_| {
            iced::widget::container::Style::default()
                .background(Background::Color(bg_color))
                .border(border::width(2).color(border_color))
        });

    container(column![card])
        .width(Length::Shrink)
        .style(|_| {
            iced::widget::container::Style::default()
                .background(Background::Color(Color::TRANSPARENT))
        })
        .into()
}

fn to_ui_notification(
    id: u32,
    notification: Notification,
    default_timeout_ms: Option<i32>,
) -> UiNotification {
    let timeout_ms = effective_timeout_ms(notification.timeout_ms, default_timeout_ms);

    UiNotification {
        id,
        app_name: notification.app_name,
        summary: notification.summary,
        body: notification.body,
        urgency: notification.urgency,
        actions: notification.actions.into_iter().map(to_ui_action).collect(),
        timeout_ms,
        created_at: Instant::now(),
    }
}

fn to_ui_action(action: NotificationAction) -> UiAction {
    UiAction {
        key: action.key,
        label: action.label,
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

fn effective_timeout_ms(requested_timeout_ms: i32, default_timeout_ms: Option<i32>) -> Option<u32> {
    let effective = match requested_timeout_ms {
        0 => return None,
        x if x < 0 => default_timeout_ms?,
        x => x,
    };

    u32::try_from(effective).ok().filter(|value| *value > 0)
}

fn estimate_popup_height(ui: &UiSection, n: &UiNotification) -> u32 {
    let rendered = render_format(&ui.format, n);
    let content_width_px = (ui.width as f32 - (ui.padding as f32 * 2.0)).max(80.0);
    let approx_char_width = (ui.font_size as f32 * 0.54).max(1.0);
    let chars_per_line = (content_width_px / approx_char_width).floor().max(1.0) as usize;

    let wrapped_lines = rendered
        .lines()
        .map(|line| wrapped_line_count(line, chars_per_line))
        .sum::<usize>()
        .max(1);

    let line_height = (ui.font_size as f32 * 1.30).ceil() as u32;
    let text_height = wrapped_lines as u32 * line_height;

    let actions_rows = n.actions.len().div_ceil(3) as u32;
    let action_row_height = (ui.font_size as f32 * 1.9).ceil() as u32;
    let actions_height = if actions_rows == 0 {
        0
    } else {
        actions_rows * action_row_height + 8
    };

    let progress_height = if ui.show_timeout_progress && n.timeout_ms.is_some() {
        ui.timeout_progress_height.max(1) as u32
    } else {
        0
    };

    let chrome = ui.padding as u32 * 2 + 6 + progress_height;

    text_height
        .saturating_add(actions_height)
        .saturating_add(chrome)
        .max(ui.height.max(1))
}

fn wrapped_line_count(line: &str, max_chars: usize) -> usize {
    if line.is_empty() {
        return 1;
    }

    let mut lines = 1usize;
    let mut current = 0usize;

    for word in line.split_whitespace() {
        let word_len = word.chars().count();

        if current == 0 {
            if word_len <= max_chars {
                current = word_len;
            } else {
                lines += word_len.div_ceil(max_chars).saturating_sub(1);
                current = word_len % max_chars;
            }
            continue;
        }

        let needed = 1 + word_len;
        if current + needed <= max_chars {
            current += needed;
        } else {
            lines += 1;
            if word_len <= max_chars {
                current = word_len;
            } else {
                lines += word_len.div_ceil(max_chars).saturating_sub(1);
                current = word_len % max_chars;
            }
        }
    }

    lines
}

fn resolve_font(raw: &str) -> Font {
    match raw.trim().to_ascii_lowercase().as_str() {
        "sans" | "sans-serif" => Font::DEFAULT,
        "serif" => Font {
            family: iced::font::Family::Serif,
            ..Font::DEFAULT
        },
        "monospace" | "mono" => Font::MONOSPACE,
        "cursive" => Font {
            family: iced::font::Family::Cursive,
            ..Font::DEFAULT
        },
        "fantasy" => Font {
            family: iced::font::Family::Fantasy,
            ..Font::DEFAULT
        },
        custom => {
            let leaked: &'static str = Box::leak(custom.to_string().into_boxed_str());
            Font::with_name(leaked)
        }
    }
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
    let (cmd_tx, mut cmd_rx) = tokio_mpsc::unbounded_channel::<SourceCommand>();
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

                loop {
                    tokio::select! {
                        maybe_event = source_events.recv() => {
                            let Some(event) = maybe_event else {
                                info!("source events channel ended");
                                break;
                            };
                            if ui_tx.send(event).is_err() {
                                warn!("ui channel receiver dropped; stopping source forwarder");
                                break;
                            }
                        }
                        maybe_cmd = cmd_rx.recv() => {
                            let Some(cmd) = maybe_cmd else {
                                info!("source command channel ended");
                                break;
                            };
                            match cmd {
                                SourceCommand::InvokeAction { id, key } => {
                                    match source_handle.invoke_action(id, &key).await {
                                        Ok(invoked) => info!(id, action_key = %key, invoked, "action command processed"),
                                        Err(err) => warn!(id, action_key = %key, ?err, "failed to process action command"),
                                    }
                                }
                                SourceCommand::Dismiss { id } => {
                                    match source_handle.close(id, wisp_types::CloseReason::Dismissed).await {
                                        Ok(closed) => info!(id, closed, "dismiss command processed"),
                                        Err(err) => warn!(id, ?err, "failed to process dismiss command"),
                                    }
                                }
                            }
                        }
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
    let ui_default_timeout_ms = app_cfg.source.default_timeout_ms;
    let boot_cmd_tx = cmd_tx.clone();

    let settings = Settings {
        layer_settings: LayerShellSettings {
            // Bootstrap surface kept minimal; real notification windows are opened dynamically.
            anchor: Anchor::Top | Anchor::Left,
            layer: Layer::Top,
            exclusive_zone: 0,
            margin: (0, 0, 0, 0),
            size: Some((1, 1)),
            ..Default::default()
        },
        ..Default::default()
    };

    let app = daemon(
        move || {
            WispdUi::new(
                Arc::clone(&boot_events),
                boot_cmd_tx.clone(),
                ui_cfg.clone(),
                ui_default_timeout_ms,
            )
        },
        namespace,
        update,
        view,
    )
    .style(app_style)
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
        let (cmd_tx, _cmd_rx) = tokio_mpsc::unbounded_channel();
        let mut ui = WispdUi::new(Arc::new(Mutex::new(rx)), cmd_tx, UiSection::default(), None);

        let _ = ui.apply_event(sample(1, "one"));
        let _ = ui.apply_event(sample(2, "two"));

        assert_eq!(ui.windows.len(), 2);
        assert_eq!(ui.windows[0].notification_id, 2);
        assert_eq!(ui.windows[1].notification_id, 1);
    }

    #[test]
    fn replacement_keeps_slot() {
        let (_tx, rx) = mpsc::channel();
        let (cmd_tx, _cmd_rx) = tokio_mpsc::unbounded_channel();
        let mut ui = WispdUi::new(Arc::new(Mutex::new(rx)), cmd_tx, UiSection::default(), None);

        let _ = ui.apply_event(sample(1, "one"));
        let _ = ui.apply_event(sample(2, "two"));
        let _ = ui.apply_event(NotificationEvent::Replaced {
            id: 1,
            previous: Box::new(Notification::default()),
            current: Box::new(Notification {
                summary: String::from("one-new"),
                ..Notification::default()
            }),
        });

        assert_eq!(ui.windows[1].notification_id, 1);
        assert_eq!(ui.notifications.get(&1).unwrap().summary, "one-new");
    }

    #[test]
    fn close_removes_notification() {
        let (_tx, rx) = mpsc::channel();
        let (cmd_tx, _cmd_rx) = tokio_mpsc::unbounded_channel();
        let mut ui = WispdUi::new(Arc::new(Mutex::new(rx)), cmd_tx, UiSection::default(), None);

        let _ = ui.apply_event(sample(1, "one"));
        let _ = ui.apply_event(NotificationEvent::Closed {
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
            actions: vec![],
            timeout_ms: None,
            created_at: Instant::now(),
        };

        let rendered = render_format("{id} {app_name} {summary} {body} {urgency}", &n);
        assert_eq!(rendered, "9 mail new message hello critical");
    }

    #[test]
    fn wrapped_line_count_wraps_long_words() {
        assert_eq!(wrapped_line_count("abcdefghij", 4), 3);
    }

    #[test]
    fn wrapped_line_count_wraps_words_with_spaces() {
        assert_eq!(wrapped_line_count("one two three four", 7), 3);
    }

    #[test]
    fn effective_timeout_uses_default_for_negative() {
        assert_eq!(effective_timeout_ms(-1, Some(5_000)), Some(5_000));
    }

    #[test]
    fn effective_timeout_disables_for_zero() {
        assert_eq!(effective_timeout_ms(0, Some(5_000)), None);
    }
}
