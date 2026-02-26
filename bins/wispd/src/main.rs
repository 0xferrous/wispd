use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind, set_hook, take_hook},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use anyhow::{Context, Result};
use iced::widget::{column, container, text};
use iced::{Element, Length, Subscription, Task};
use iced_layershell::application;
use iced_layershell::reexport::{Anchor, Layer};
use iced_layershell::settings::{LayerShellSettings, Settings};
use iced_layershell::to_layer_message;
use tracing::{info, warn};
use wisp_source::{SourceConfig, WispSource};
use wisp_types::{Notification, NotificationEvent, Urgency};

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
    max_visible: usize,
}

impl WispdUi {
    fn new(events: Arc<Mutex<mpsc::Receiver<NotificationEvent>>>) -> Self {
        Self {
            events,
            notifications: VecDeque::new(),
            max_visible: 5,
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
        while self.notifications.len() > self.max_visible {
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
    let mut content = column![].spacing(8).padding(12).width(Length::Fill);

    if state.notifications.is_empty() {
        content = content.push(text("wispd is running…").size(16));
        content = content.push(text("waiting for notifications").size(14));
    } else {
        for n in &state.notifications {
            let urgency = match n.urgency {
                Urgency::Low => "low",
                Urgency::Normal => "normal",
                Urgency::Critical => "critical",
            };

            let card = column![
                text(format!("#{} [{}] {}", n.id, urgency, n.app_name)).size(13),
                text(n.summary.clone()).size(18),
                text(n.body.clone()).size(14),
            ]
            .spacing(4)
            .padding(10);

            content = content.push(container(card));
        }
    }

    container(content).width(Length::Fill).into()
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

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;

    let cfg = SourceConfig::default();
    let (_source, mut source_events, _dbus) = runtime
        .block_on(WispSource::start_dbus(cfg.clone()))
        .context("failed to start wisp source over dbus")?;

    let (ui_tx, ui_rx) = mpsc::channel::<NotificationEvent>();
    runtime.spawn(async move {
        while let Some(event) = source_events.recv().await {
            if ui_tx.send(event).is_err() {
                break;
            }
        }
    });

    info!(
        dbus_name = %cfg.dbus_name,
        dbus_path = %cfg.dbus_path,
        "wispd ui started"
    );

    let events = Arc::new(Mutex::new(ui_rx));
    let boot_events = Arc::clone(&events);

    let settings = Settings {
        layer_settings: LayerShellSettings {
            anchor: Anchor::Top | Anchor::Right,
            layer: Layer::Overlay,
            exclusive_zone: 0,
            margin: (16, 16, 16, 16),
            size: Some((420, 300)),
            ..Default::default()
        },
        ..Default::default()
    };

    let app = application(
        move || WispdUi::new(Arc::clone(&boot_events)),
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
        Ok(Err(err)) => Err(anyhow::anyhow!("failed to run iced layer-shell app: {err}")),
        Err(_) => Err(anyhow::anyhow!(
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
        let mut ui = WispdUi::new(Arc::new(Mutex::new(rx)));

        ui.apply_event(sample(1, "one"));
        ui.apply_event(sample(2, "two"));

        assert_eq!(ui.notifications[0].id, 2);
        assert_eq!(ui.notifications[1].id, 1);
    }

    #[test]
    fn replacement_keeps_slot() {
        let (_tx, rx) = mpsc::channel();
        let mut ui = WispdUi::new(Arc::new(Mutex::new(rx)));

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
        let mut ui = WispdUi::new(Arc::new(Mutex::new(rx)));

        ui.apply_event(sample(1, "one"));
        ui.apply_event(NotificationEvent::Closed {
            id: 1,
            reason: CloseReason::ClosedByCall,
        });

        assert!(ui.notifications.is_empty());
    }
}
