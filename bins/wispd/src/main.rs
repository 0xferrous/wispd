use std::{
    collections::{HashMap, VecDeque},
    fs,
    panic::{AssertUnwindSafe, catch_unwind, set_hook, take_hook},
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use iced::widget::button::Status as ButtonStatus;
use iced::widget::{button, column, container, image, mouse_area, row, text};
use iced::{Background, Color, ContentFit, Element, Font, Length, Subscription, Task, border};
use iced_layershell::daemon;
use iced_layershell::reexport::{Anchor, IcedId, Layer, NewLayerShellSettings, OutputOption};
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

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
enum ClickAction {
    #[default]
    Dismiss,
    InvokeDefaultAction,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct UiSection {
    #[allow(dead_code)]
    format: String,
    max_visible: usize,
    width: u32,
    height: u32,
    gap: u16,
    padding: u16,
    font_size: u16,
    #[serde(alias = "font")]
    font_family: String,
    show_icons: bool,
    max_icon_size: u16,
    anchor: String,
    output: String,
    focused_output_command: Option<String>,
    margin: MarginConfig,
    colors: UrgencyColors,
    text: TextStyleConfig,
    buttons: ButtonStyleConfig,
    show_timeout_progress: bool,
    timeout_progress_height: u16,
    timeout_progress_position: String,
    left_click_action: ClickAction,
    right_click_action: ClickAction,
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
            show_icons: true,
            max_icon_size: 32,
            anchor: "top-right".to_string(),
            output: "focused".to_string(),
            focused_output_command: None,
            margin: MarginConfig::default(),
            colors: UrgencyColors::default(),
            text: TextStyleConfig::default(),
            buttons: ButtonStyleConfig::default(),
            show_timeout_progress: true,
            timeout_progress_height: 3,
            timeout_progress_position: "bottom".to_string(),
            left_click_action: ClickAction::Dismiss,
            right_click_action: ClickAction::InvokeDefaultAction,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct TextStyleConfig {
    app_name: TextPartStyle,
    summary: TextPartStyle,
    body: TextPartStyle,
}

impl Default for TextStyleConfig {
    fn default() -> Self {
        Self {
            app_name: TextPartStyle {
                color: "#a89984".to_string(),
                font_size: None,
            },
            summary: TextPartStyle {
                color: "#fabd2f".to_string(),
                font_size: None,
            },
            body: TextPartStyle {
                color: "#ebdbb2".to_string(),
                font_size: None,
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct TextPartStyle {
    color: String,
    font_size: Option<u16>,
}

impl Default for TextPartStyle {
    fn default() -> Self {
        Self {
            color: "#f8f8f2".to_string(),
            font_size: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct ButtonStyleConfig {
    text_color: String,
    background: String,
    border_color: String,
    hover_background: String,
    hover_text_color: String,
    #[serde(alias = "font")]
    font_family: Option<String>,
    font_size: Option<u16>,
    close_font_size: Option<u16>,
}

impl Default for ButtonStyleConfig {
    fn default() -> Self {
        Self {
            text_color: "#ebdbb2".to_string(),
            background: "#3c3836".to_string(),
            border_color: "#665c54".to_string(),
            hover_background: "#504945".to_string(),
            hover_text_color: "#fbf1c7".to_string(),
            font_family: None,
            font_size: None,
            close_font_size: None,
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
    app_icon: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
            output_option: output_option_from_config(
                &self.ui.output,
                self.ui.focused_output_command.as_deref(),
            ),
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

    fn dispatch_click_action(&self, id: u32, action: ClickAction) {
        let cmd = match action {
            ClickAction::Dismiss => SourceCommand::Dismiss { id },
            ClickAction::InvokeDefaultAction => SourceCommand::InvokeAction {
                id,
                key: "default".to_string(),
            },
        };

        if let Err(err) = self.cmd_tx.send(cmd) {
            warn!(?err, "failed to send click action command to source thread");
        }
    }
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    Tick,
    ActionClicked { id: u32, key: String },
    DismissClicked { id: u32 },
    NotificationLeftClick { id: u32 },
    NotificationRightClick { id: u32 },
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
        Message::NotificationLeftClick { id } => {
            state.dispatch_click_action(id, state.ui.left_click_action);
            Task::none()
        }
        Message::NotificationRightClick { id } => {
            state.dispatch_click_action(id, state.ui.right_click_action);
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

    let border_color = urgency_color(&state.ui.colors, n.urgency.clone());
    let bg_color = parse_hex_color(&state.ui.colors.background)
        .unwrap_or(Color::from_rgba(0.12, 0.12, 0.18, 0.8));
    let text_color = parse_hex_color(&state.ui.colors.text).unwrap_or(Color::WHITE);
    let progress_color = parse_hex_color(&state.ui.colors.timeout_progress).unwrap_or(text_color);
    let app_name_color = parse_hex_color(&state.ui.text.app_name.color).unwrap_or(text_color);
    let summary_color = parse_hex_color(&state.ui.text.summary.color).unwrap_or(text_color);
    let body_color = parse_hex_color(&state.ui.text.body.color).unwrap_or(text_color);

    let card_width = state.ui.width as f32;
    let card_height = estimate_popup_height(&state.ui, n) as f32;
    let card_padding = state.ui.padding;

    let app_name_size = state
        .ui
        .text
        .app_name
        .font_size
        .unwrap_or(state.ui.font_size) as u32;
    let summary_size = state
        .ui
        .text
        .summary
        .font_size
        .unwrap_or(state.ui.font_size) as u32;
    let body_size = state.ui.text.body.font_size.unwrap_or(state.ui.font_size) as u32;

    let font = resolve_font(&state.ui.font_family);

    let button_text_color =
        parse_hex_color(&state.ui.buttons.text_color).unwrap_or(Color::from_rgb8(0xeb, 0xdb, 0xb2));
    let button_bg_color =
        parse_hex_color(&state.ui.buttons.background).unwrap_or(Color::from_rgb8(0x3c, 0x38, 0x36));
    let button_border_color = parse_hex_color(&state.ui.buttons.border_color)
        .unwrap_or(Color::from_rgb8(0x66, 0x5c, 0x54));
    let button_hover_bg_color = parse_hex_color(&state.ui.buttons.hover_background)
        .unwrap_or(Color::from_rgb8(0x50, 0x49, 0x45));
    let button_hover_text_color = parse_hex_color(&state.ui.buttons.hover_text_color)
        .unwrap_or(Color::from_rgb8(0xfb, 0xf1, 0xc7));

    let button_font = state
        .ui
        .buttons
        .font_family
        .as_deref()
        .map(resolve_font)
        .unwrap_or(font);
    let button_font_size = state.ui.buttons.font_size.unwrap_or(state.ui.font_size) as u32;
    let close_button_font_size = state.ui.buttons.close_font_size.unwrap_or(
        state
            .ui
            .buttons
            .font_size
            .unwrap_or(state.ui.font_size.saturating_sub(2)),
    ) as u32;

    let close_button = button(
        text("✕")
            .size(close_button_font_size)
            .font(button_font)
            .color(button_text_color),
    )
    .padding([1, 6])
    .style(move |_, status| {
        style_button(
            status,
            button_bg_color,
            button_text_color,
            button_border_color,
            button_hover_bg_color,
            button_hover_text_color,
        )
    })
    .on_press(Message::DismissClicked { id: n.id });

    let mut text_block = column![].spacing(2);

    let mut top_line = row![].spacing(6);
    if !n.app_name.trim().is_empty() {
        top_line = top_line.push(
            text(n.app_name.clone())
                .size(app_name_size)
                .font(font)
                .color(app_name_color),
        );
    }
    if !n.summary.trim().is_empty() {
        top_line = top_line.push(
            text(n.summary.clone())
                .size(summary_size)
                .font(font)
                .color(summary_color),
        );
    }
    if !n.app_name.trim().is_empty() || !n.summary.trim().is_empty() {
        text_block = text_block.push(top_line);
    }

    if !n.body.trim().is_empty() {
        text_block = text_block.push(
            text(n.body.clone())
                .size(body_size)
                .font(font)
                .color(body_color),
        );
    }

    let header = row![container(text_block).width(Length::Fill), close_button].spacing(8);

    let mut card_content = column![header].spacing(8);

    if !n.actions.is_empty() {
        for action_chunk in n.actions.chunks(3) {
            let mut actions_row = row![].spacing(8);
            for action in action_chunk {
                let btn_bg = button_bg_color;
                let btn_fg = button_text_color;
                let btn_border = button_border_color;
                let btn_hover_bg = button_hover_bg_color;
                let btn_hover_fg = button_hover_text_color;

                actions_row = actions_row.push(
                    button(
                        text(action.label.clone())
                            .font(button_font)
                            .size(button_font_size)
                            .color(btn_fg),
                    )
                    .style(move |_, status| {
                        style_button(
                            status,
                            btn_bg,
                            btn_fg,
                            btn_border,
                            btn_hover_bg,
                            btn_hover_fg,
                        )
                    })
                    .on_press(Message::ActionClicked {
                        id: n.id,
                        key: action.key.clone(),
                    }),
                );
            }
            card_content = card_content.push(actions_row);
        }
    }

    let mut content_row = row![].spacing(10);
    if state.ui.show_icons
        && let Some(path) = resolve_icon_path(&n.app_icon)
        && path.is_file()
    {
        let icon_size = state.ui.max_icon_size.max(1) as f32;
        let icon = image(iced::widget::image::Handle::from_path(path))
            .width(Length::Fixed(icon_size))
            .height(Length::Fixed(icon_size))
            .content_fit(ContentFit::Contain);
        content_row = content_row.push(
            container(icon)
                .width(Length::Fixed(icon_size))
                .height(Length::Fixed(icon_size)),
        );
    }
    content_row = content_row.push(container(card_content).width(Length::Fill));

    let body = container(content_row)
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

    let clickable_card = mouse_area(card)
        .on_press(Message::NotificationLeftClick { id: n.id })
        .on_right_press(Message::NotificationRightClick { id: n.id });

    container(column![clickable_card])
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
        app_icon: notification.app_icon,
        summary: notification.summary,
        body: notification.body,
        urgency: notification.urgency,
        actions: notification
            .actions
            .into_iter()
            .filter_map(to_ui_action)
            .collect(),
        timeout_ms,
        created_at: Instant::now(),
    }
}

fn to_ui_action(action: NotificationAction) -> Option<UiAction> {
    if action.label.trim().is_empty() {
        return None;
    }

    Some(UiAction {
        key: action.key,
        label: action.label,
    })
}

#[cfg(test)]
fn render_format(format: &str, n: &UiNotification) -> String {
    format
        .replace("{id}", &n.id.to_string())
        .replace("{app_name}", &n.app_name)
        .replace("{summary}", &n.summary)
        .replace("{body}", &n.body)
        .replace("{urgency}", urgency_label(n.urgency.clone()))
}

fn resolve_icon_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(path) = trimmed.strip_prefix("file://") {
        return Some(PathBuf::from(path));
    }

    Some(PathBuf::from(trimmed))
}

fn style_button(
    status: ButtonStatus,
    background: Color,
    text: Color,
    border_color: Color,
    hover_background: Color,
    hover_text: Color,
) -> iced::widget::button::Style {
    let (bg, fg) = match status {
        ButtonStatus::Hovered | ButtonStatus::Pressed => (hover_background, hover_text),
        _ => (background, text),
    };

    iced::widget::button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: border::width(1).color(border_color),
        ..Default::default()
    }
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
    let app_name_size = ui.text.app_name.font_size.unwrap_or(ui.font_size) as f32;
    let summary_size = ui.text.summary.font_size.unwrap_or(ui.font_size) as f32;
    let body_size = ui.text.body.font_size.unwrap_or(ui.font_size) as f32;

    let content_width_px = (ui.width as f32 - (ui.padding as f32 * 2.0)).max(80.0);

    let header_text = match (n.app_name.trim().is_empty(), n.summary.trim().is_empty()) {
        (false, false) => format!("{} {}", n.app_name, n.summary),
        (false, true) => n.app_name.clone(),
        (true, false) => n.summary.clone(),
        (true, true) => String::new(),
    };

    let header_font_size = app_name_size.max(summary_size).max(1.0);
    let header_char_width = (header_font_size * 0.54).max(1.0);
    let header_chars_per_line = (content_width_px / header_char_width).floor().max(1.0) as usize;
    let header_wrapped_lines = if header_text.is_empty() {
        0
    } else {
        wrapped_line_count(&header_text, header_chars_per_line)
    };
    let header_line_height = (header_font_size * 1.30).ceil() as u32;
    let header_height = header_wrapped_lines as u32 * header_line_height;

    let body_char_width = (body_size * 0.54).max(1.0);
    let body_chars_per_line = (content_width_px / body_char_width).floor().max(1.0) as usize;
    let body_wrapped_lines = if n.body.trim().is_empty() {
        0
    } else {
        n.body
            .lines()
            .map(|line| wrapped_line_count(line, body_chars_per_line))
            .sum::<usize>()
            .max(1)
    };
    let body_line_height = (body_size * 1.30).ceil() as u32;
    let body_height = body_wrapped_lines as u32 * body_line_height;

    let text_height = header_height.saturating_add(body_height);
    let icon_height = if ui.show_icons && resolve_icon_path(&n.app_icon).is_some() {
        ui.max_icon_size.max(1) as u32
    } else {
        0
    };
    let content_height = text_height.max(icon_height);

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

    let chrome = ui.padding as u32 * 2 + 10 + progress_height;

    content_height
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
    let trimmed = raw.trim();

    match trimmed.to_ascii_lowercase().as_str() {
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
        _ => {
            let leaked: &'static str = Box::leak(trimmed.to_string().into_boxed_str());
            Font::with_name(leaked)
        }
    }
}

#[cfg(test)]
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

fn output_option_from_config(output: &str, focused_output_command: Option<&str>) -> OutputOption {
    let trimmed = output.trim();
    let lower = trimmed.to_ascii_lowercase();

    match lower.as_str() {
        "focused" => resolve_focused_output_name(focused_output_command)
            .map(OutputOption::OutputName)
            .unwrap_or(OutputOption::None),
        // Sticky output: follow last active output of this surface family.
        "last-output" | "last_output" => OutputOption::LastOutput,
        "any" | "none" | "default" => OutputOption::None,
        _ if trimmed.is_empty() => OutputOption::None,
        _ => OutputOption::OutputName(trimmed.to_string()),
    }
}

fn resolve_focused_output_name(focused_output_command: Option<&str>) -> Option<String> {
    let cmd = focused_output_command?.trim();
    if cmd.is_empty() {
        return None;
    }

    let out = Command::new("sh").arg("-c").arg(cmd).output().ok()?;
    if !out.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let name = stdout.lines().next()?.trim();
    if name.is_empty() {
        return None;
    }

    Some(name.to_string())
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
            app_icon: String::new(),
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
    fn resolve_icon_path_supports_file_uri() {
        assert_eq!(
            resolve_icon_path("file:///tmp/icon.png"),
            Some(PathBuf::from("/tmp/icon.png"))
        );
    }

    #[test]
    fn empty_action_labels_are_filtered_from_ui() {
        let ui_notification = to_ui_notification(
            1,
            Notification {
                actions: vec![
                    NotificationAction {
                        key: "default".to_string(),
                        label: " ".to_string(),
                    },
                    NotificationAction {
                        key: "open".to_string(),
                        label: "Open".to_string(),
                    },
                ],
                ..Notification::default()
            },
            None,
        );

        assert_eq!(ui_notification.actions.len(), 1);
        assert_eq!(ui_notification.actions[0].key, "open");
        assert_eq!(ui_notification.actions[0].label, "Open");
    }

    #[test]
    fn ui_font_can_be_configured_via_font_alias() {
        let cfg: AppConfig = toml::from_str("[ui]\nfont = \"JetBrains Mono\"\n").unwrap();
        assert_eq!(cfg.ui.font_family, "JetBrains Mono");
    }

    #[test]
    fn button_font_can_be_configured_via_font_alias() {
        let cfg: AppConfig =
            toml::from_str("[ui.buttons]\nfont = \"Recursive Mono Casual Static\"\n").unwrap();
        assert_eq!(
            cfg.ui.buttons.font_family.as_deref(),
            Some("Recursive Mono Casual Static")
        );
    }

    #[test]
    fn ui_output_defaults_to_focused() {
        assert_eq!(AppConfig::default().ui.output, "focused");
    }

    #[test]
    fn output_option_parses_focused() {
        assert_eq!(
            output_option_from_config("focused", None),
            OutputOption::None
        );
    }

    #[test]
    fn output_option_parses_last_output() {
        assert_eq!(
            output_option_from_config("last-output", None),
            OutputOption::LastOutput
        );
    }

    #[test]
    fn output_option_parses_output_name() {
        assert_eq!(
            output_option_from_config("DP-1", None),
            OutputOption::OutputName("DP-1".to_string())
        );
    }

    #[test]
    fn output_option_uses_focused_command_when_provided() {
        assert_eq!(
            output_option_from_config("focused", Some("printf 'DP-3\\n'")),
            OutputOption::OutputName("DP-3".to_string())
        );
    }

    #[test]
    fn effective_timeout_uses_default_for_negative() {
        assert_eq!(effective_timeout_ms(-1, Some(5_000)), Some(5_000));
    }

    #[test]
    fn effective_timeout_disables_for_zero() {
        assert_eq!(effective_timeout_ms(0, Some(5_000)), None);
    }

    #[test]
    fn left_click_can_invoke_default_action() {
        let (_tx, rx) = mpsc::channel();
        let (cmd_tx, mut cmd_rx) = tokio_mpsc::unbounded_channel();
        let ui_cfg = UiSection {
            left_click_action: ClickAction::InvokeDefaultAction,
            ..UiSection::default()
        };
        let mut ui = WispdUi::new(Arc::new(Mutex::new(rx)), cmd_tx, ui_cfg, None);

        let _ = update(&mut ui, Message::NotificationLeftClick { id: 42 });

        assert_eq!(
            cmd_rx.try_recv().unwrap(),
            SourceCommand::InvokeAction {
                id: 42,
                key: "default".to_string(),
            }
        );
    }

    #[test]
    fn right_click_can_dismiss() {
        let (_tx, rx) = mpsc::channel();
        let (cmd_tx, mut cmd_rx) = tokio_mpsc::unbounded_channel();
        let ui_cfg = UiSection {
            right_click_action: ClickAction::Dismiss,
            ..UiSection::default()
        };
        let mut ui = WispdUi::new(Arc::new(Mutex::new(rx)), cmd_tx, ui_cfg, None);

        let _ = update(&mut ui, Message::NotificationRightClick { id: 11 });

        assert_eq!(
            cmd_rx.try_recv().unwrap(),
            SourceCommand::Dismiss { id: 11 }
        );
    }
}
