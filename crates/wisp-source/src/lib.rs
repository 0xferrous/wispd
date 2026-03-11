use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use thiserror::Error;
use tokio::runtime::Handle;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{RwLock as AsyncRwLock, mpsc};
use tracing::{debug, info, warn};
use wisp_types::{
    CloseReason, Notification, NotificationAction, NotificationEvent, NotificationHints, Urgency,
};
use zbus::{connection::Builder as ConnectionBuilder, object_server::SignalEmitter, zvariant};

/// Default freedesktop notification bus name.
pub const DEFAULT_DBUS_NAME: &str = "org.freedesktop.Notifications";
/// Default freedesktop notification object path.
pub const DEFAULT_DBUS_PATH: &str = "/org/freedesktop/Notifications";
/// Freedesktop notifications D-Bus interface name.
pub const DBUS_INTERFACE: &str = "org.freedesktop.Notifications";

/// Configuration for [`WispSource`].
#[derive(Debug, Clone)]
pub struct SourceConfig {
    /// Capabilities returned by `GetCapabilities`.
    pub capabilities: Vec<String>,
    /// Capacity of the internal notification event channel.
    pub channel_capacity: usize,
    /// D-Bus name to own.
    pub dbus_name: String,
    /// D-Bus object path to serve.
    pub dbus_path: String,
    /// Server name returned by `GetServerInformation`.
    pub server_name: String,
    /// Server vendor returned by `GetServerInformation`.
    pub server_vendor: String,
    /// Server version returned by `GetServerInformation`.
    pub server_version: String,
    /// Spec version returned by `GetServerInformation`.
    pub spec_version: String,
    /// Default timeout used when incoming timeout is negative.
    ///
    /// If `None`, negative incoming timeout values are treated as persistent.
    pub default_timeout_ms: Option<i32>,
}

impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            capabilities: vec!["body".to_string()],
            channel_capacity: 256,
            dbus_name: DEFAULT_DBUS_NAME.to_string(),
            dbus_path: DEFAULT_DBUS_PATH.to_string(),
            server_name: "wispd".to_string(),
            server_vendor: "wispd".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            spec_version: "1.2".to_string(),
            default_timeout_ms: None,
        }
    }
}

/// Errors produced by source runtime operations.
#[derive(Debug, Error)]
pub enum SourceError {
    /// Event receiver dropped and source can no longer publish events.
    #[error("event channel closed")]
    EventChannelClosed,
}

/// Errors produced while starting the D-Bus server.
#[derive(Debug, Error)]
pub enum StartupError {
    /// A D-Bus error occurred.
    #[error("dbus error: {0}")]
    Dbus(#[from] zbus::Error),
}

/// In-memory notification source plus lifecycle logic.
#[derive(Debug, Clone)]
pub struct WispSource {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    cfg: SourceConfig,
    capabilities: RwLock<Vec<String>>,
    default_timeout_ms: RwLock<Option<i32>>,
    sender: mpsc::Sender<NotificationEvent>,
    notifications: Mutex<HashMap<u32, StoredNotification>>,
    next_id: AtomicU32,
    dbus_connection: AsyncRwLock<Option<zbus::Connection>>,
    runtime_handle: Option<Handle>,
}

#[derive(Debug, Clone)]
struct StoredNotification {
    notification: Notification,
    generation: u64,
}

/// Handle that keeps the D-Bus service connection alive.
#[derive(Debug)]
pub struct DbusService {
    connection: zbus::Connection,
}

impl DbusService {
    /// Returns the underlying active D-Bus connection.
    pub fn connection(&self) -> &zbus::Connection {
        &self.connection
    }
}

impl WispSource {
    /// Creates a new source and returns it with its event receiver.
    pub fn new(cfg: SourceConfig) -> (Self, mpsc::Receiver<NotificationEvent>) {
        let (sender, receiver) = mpsc::channel(cfg.channel_capacity);
        let source = Self {
            inner: Arc::new(Inner {
                capabilities: RwLock::new(cfg.capabilities.clone()),
                default_timeout_ms: RwLock::new(cfg.default_timeout_ms),
                cfg,
                sender,
                notifications: Mutex::new(HashMap::new()),
                next_id: AtomicU32::new(1),
                dbus_connection: AsyncRwLock::new(None),
                runtime_handle: Handle::try_current().ok(),
            }),
        };

        (source, receiver)
    }

    /// Starts a session-bus freedesktop notifications service.
    ///
    /// Returns the initialized source, event receiver, and a [`DbusService`] handle
    /// that must be kept alive for the service to remain available.
    pub async fn start_dbus(
        cfg: SourceConfig,
    ) -> Result<(Self, mpsc::Receiver<NotificationEvent>, DbusService), StartupError> {
        let (source, receiver) = Self::new(cfg.clone());
        let iface = NotificationsInterface {
            source: source.clone(),
        };

        info!(dbus_name = %cfg.dbus_name, dbus_path = %cfg.dbus_path, "starting dbus notification service");
        let connection = ConnectionBuilder::session()?
            .name(cfg.dbus_name.as_str())?
            .serve_at(cfg.dbus_path.as_str(), iface)?
            .build()
            .await?;

        info!(dbus_name = %cfg.dbus_name, "dbus notification service ready");
        source.set_dbus_connection(connection.clone()).await;

        Ok((source, receiver, DbusService { connection }))
    }

    /// Returns currently advertised freedesktop capabilities.
    pub fn capabilities(&self) -> Vec<String> {
        self.inner
            .capabilities
            .read()
            .expect("capabilities lock poisoned")
            .clone()
    }

    /// Updates runtime-configurable source values.
    pub fn update_runtime_config(
        &self,
        capabilities: Vec<String>,
        default_timeout_ms: Option<i32>,
    ) {
        *self
            .inner
            .capabilities
            .write()
            .expect("capabilities lock poisoned") = capabilities;
        *self
            .inner
            .default_timeout_ms
            .write()
            .expect("default timeout lock poisoned") = default_timeout_ms;
    }

    /// Inserts or replaces a notification and emits the corresponding event.
    ///
    /// If `replaces_id` points to an existing notification, replacement happens in-place
    /// and the same id is returned.
    pub async fn notify(
        &self,
        notification: Notification,
        replaces_id: u32,
    ) -> Result<u32, SourceError> {
        let timeout_ms = notification.timeout_ms;
        debug!(app = %notification.app_name, summary = %notification.summary, replaces_id, timeout_ms, "processing notification");
        debug!("acquiring notifications lock for notify");
        let mut store = self
            .inner
            .notifications
            .lock()
            .expect("notifications mutex poisoned");

        if replaces_id != 0
            && let Some(entry) = store.get_mut(&replaces_id)
        {
            let previous = entry.notification.clone();
            entry.notification = notification.clone();
            entry.generation = entry.generation.saturating_add(1);
            let generation = entry.generation;
            drop(store);

            self.schedule_timeout(replaces_id, generation, timeout_ms);
            self.send_event(NotificationEvent::Replaced {
                id: replaces_id,
                previous: Box::new(previous),
                current: Box::new(notification),
            })?;
            debug!(id = replaces_id, "notification replaced");
            return Ok(replaces_id);
        }

        let id = self.alloc_id();
        debug!(id, "allocated notification id");

        let generation = 0;
        store.insert(
            id,
            StoredNotification {
                notification: notification.clone(),
                generation,
            },
        );
        drop(store);

        self.schedule_timeout(id, generation, timeout_ms);
        self.send_event(NotificationEvent::Received {
            id,
            notification: Box::new(notification),
        })?;
        debug!(id, "notification stored");
        Ok(id)
    }

    /// Closes a notification by id.
    ///
    /// Returns `Ok(true)` if a notification was closed, `Ok(false)` if it was not found.
    pub async fn close(&self, id: u32, reason: CloseReason) -> Result<bool, SourceError> {
        let removed = self
            .inner
            .notifications
            .lock()
            .expect("notifications mutex poisoned")
            .remove(&id);
        if removed.is_none() {
            return Ok(false);
        }

        self.send_closed(id, reason).await?;
        Ok(true)
    }

    /// Invokes an action for a notification.
    ///
    /// On success, emits `ActionInvoked` and then closes the notification as dismissed.
    /// Returns `Ok(false)` if notification or action key is not found.
    pub async fn invoke_action(&self, id: u32, action_key: &str) -> Result<bool, SourceError> {
        let action_exists = {
            let mut store = self
                .inner
                .notifications
                .lock()
                .expect("notifications mutex poisoned");
            let Some(stored) = store.remove(&id) else {
                return Ok(false);
            };

            if !stored
                .notification
                .actions
                .iter()
                .any(|a| a.key == action_key)
            {
                store.insert(id, stored);
                false
            } else {
                true
            }
        };

        if !action_exists {
            return Ok(false);
        }

        self.send_event(NotificationEvent::ActionInvoked {
            id,
            action_key: action_key.to_string(),
        })?;
        self.emit_action_invoked_signal(id, action_key).await;
        self.send_closed(id, CloseReason::Dismissed).await?;

        Ok(true)
    }

    /// Returns a snapshot of current notifications keyed by id.
    pub async fn snapshot(&self) -> Vec<(u32, Notification)> {
        let store = self
            .inner
            .notifications
            .lock()
            .expect("notifications mutex poisoned");
        store
            .iter()
            .map(|(id, stored)| (*id, stored.notification.clone()))
            .collect()
    }

    /// Returns `(name, vendor, version, spec_version)` for `GetServerInformation`.
    pub fn server_information(&self) -> (String, String, String, String) {
        (
            self.inner.cfg.server_name.clone(),
            self.inner.cfg.server_vendor.clone(),
            self.inner.cfg.server_version.clone(),
            self.inner.cfg.spec_version.clone(),
        )
    }

    async fn set_dbus_connection(&self, connection: zbus::Connection) {
        *self.inner.dbus_connection.write().await = Some(connection);
    }

    fn schedule_timeout(&self, id: u32, generation: u64, requested_timeout_ms: i32) {
        let Some(duration) = self.effective_timeout_duration(requested_timeout_ms) else {
            return;
        };

        let handle = self
            .inner
            .runtime_handle
            .clone()
            .or_else(|| Handle::try_current().ok());
        let Some(handle) = handle else {
            warn!(
                id,
                "no tokio runtime handle available; skipping timeout scheduling"
            );
            return;
        };

        let source = self.clone();
        handle.spawn(async move {
            tokio::time::sleep(duration).await;
            if let Err(err) = source.expire_if_current(id, generation).await {
                warn!(id, ?err, "failed to process timeout expiration");
            }
        });
    }

    fn effective_timeout_duration(&self, requested_timeout_ms: i32) -> Option<Duration> {
        let default_timeout_ms = *self
            .inner
            .default_timeout_ms
            .read()
            .expect("default timeout lock poisoned");

        let effective_ms = match requested_timeout_ms {
            0 => return None,
            x if x < 0 => default_timeout_ms?,
            x => x,
        };

        let millis = u64::try_from(effective_ms).ok()?;
        if millis == 0 {
            None
        } else {
            Some(Duration::from_millis(millis))
        }
    }

    async fn expire_if_current(&self, id: u32, generation: u64) -> Result<(), SourceError> {
        let removed = {
            let mut store = self
                .inner
                .notifications
                .lock()
                .expect("notifications mutex poisoned");

            let should_expire = store
                .get(&id)
                .is_some_and(|entry| entry.generation == generation);

            if !should_expire {
                return Ok(());
            }

            store.remove(&id)
        };

        if removed.is_none() {
            return Ok(());
        }

        self.send_closed(id, CloseReason::Expired).await
    }

    async fn send_closed(&self, id: u32, reason: CloseReason) -> Result<(), SourceError> {
        self.send_event(NotificationEvent::Closed {
            id,
            reason: reason.clone(),
        })?;
        self.emit_notification_closed_signal(id, reason).await;
        Ok(())
    }

    async fn emit_notification_closed_signal(&self, id: u32, reason: CloseReason) {
        let Some(connection) = self.inner.dbus_connection.read().await.clone() else {
            return;
        };

        if let Err(err) = connection
            .emit_signal(
                None::<&str>,
                self.inner.cfg.dbus_path.as_str(),
                DBUS_INTERFACE,
                "NotificationClosed",
                &(id, close_reason_code(reason)),
            )
            .await
        {
            warn!(id, ?err, "failed to emit NotificationClosed signal");
        }
    }

    async fn emit_action_invoked_signal(&self, id: u32, action_key: &str) {
        let Some(connection) = self.inner.dbus_connection.read().await.clone() else {
            return;
        };

        if let Err(err) = connection
            .emit_signal(
                None::<&str>,
                self.inner.cfg.dbus_path.as_str(),
                DBUS_INTERFACE,
                "ActionInvoked",
                &(id, action_key),
            )
            .await
        {
            warn!(id, ?err, "failed to emit ActionInvoked signal");
        }
    }

    fn alloc_id(&self) -> u32 {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        debug!(id, "next_id advanced");
        id
    }

    fn send_event(&self, event: NotificationEvent) -> Result<(), SourceError> {
        debug!(?event, "sending notification event");
        match self.inner.sender.try_send(event) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                warn!("event queue full; dropping notification event");
                Ok(())
            }
            Err(TrySendError::Closed(_)) => {
                warn!("event receiver dropped");
                Err(SourceError::EventChannelClosed)
            }
        }
    }
}

#[derive(Debug, Clone)]
struct NotificationsInterface {
    source: WispSource,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl NotificationsInterface {
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, zvariant::OwnedValue>,
        expire_timeout: i32,
    ) -> zbus::fdo::Result<u32> {
        info!(app = %app_name, summary = %summary, replaces_id, expire_timeout, action_pairs = actions.len() / 2, "dbus Notify called");
        let (urgency, parsed_hints) = parse_hints(&hints);
        let notification = Notification {
            app_name,
            app_icon,
            summary,
            body,
            urgency,
            timeout_ms: expire_timeout,
            actions: parse_actions(actions),
            hints: parsed_hints,
        };

        let id = self
            .source
            .notify(notification, replaces_id)
            .await
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))?;

        info!(id, "dbus Notify handled");
        Ok(id)
    }

    async fn close_notification(&self, id: u32) -> zbus::fdo::Result<()> {
        info!(id, "dbus CloseNotification called");
        let closed = self
            .source
            .close(id, CloseReason::ClosedByCall)
            .await
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))?;
        info!(id, closed, "dbus CloseNotification handled");
        Ok(())
    }

    fn get_capabilities(&self) -> Vec<String> {
        self.source.capabilities()
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        self.source.server_information()
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

fn parse_actions(flat_actions: Vec<String>) -> Vec<NotificationAction> {
    flat_actions
        .chunks_exact(2)
        .map(|chunk| NotificationAction {
            key: chunk[0].clone(),
            label: chunk[1].clone(),
        })
        .collect()
}

fn parse_hints(hints: &HashMap<String, zvariant::OwnedValue>) -> (Urgency, NotificationHints) {
    let urgency = hints
        .get("urgency")
        .and_then(|raw| u8::try_from(raw).ok())
        .map(|value| match value {
            0 => Urgency::Low,
            2 => Urgency::Critical,
            _ => Urgency::Normal,
        })
        .unwrap_or(Urgency::Normal);

    let category = hints
        .get("category")
        .and_then(|raw| <&str>::try_from(raw).ok())
        .map(ToOwned::to_owned);
    let desktop_entry = hints
        .get("desktop-entry")
        .and_then(|raw| <&str>::try_from(raw).ok())
        .map(ToOwned::to_owned);
    let transient = hints
        .get("transient")
        .and_then(|raw| bool::try_from(raw).ok());

    let extra = hints
        .iter()
        .filter(|(key, _)| {
            key.as_str() != "urgency"
                && key.as_str() != "category"
                && key.as_str() != "desktop-entry"
                && key.as_str() != "transient"
        })
        .map(|(key, value)| (key.clone(), format_hint_value(key, value)))
        .collect();

    (
        urgency,
        NotificationHints {
            category,
            desktop_entry,
            transient,
            extra,
        },
    )
}

fn format_hint_value(key: &str, value: &zvariant::OwnedValue) -> String {
    if matches!(key, "image-data" | "image_data" | "icon_data") {
        return "<omitted image payload>".to_string();
    }

    let signature = value.value_signature().to_string();
    if signature.contains("ay") {
        return format!("<omitted binary payload signature={signature}>");
    }

    format!("{value:?}")
}

fn close_reason_code(reason: CloseReason) -> u32 {
    match reason {
        CloseReason::Expired => 1,
        CloseReason::Dismissed => 2,
        CloseReason::ClosedByCall => 3,
        CloseReason::Undefined => 4,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use futures_util::StreamExt;

    use super::*;

    fn test_notification(summary: &str) -> Notification {
        Notification {
            app_name: "test".into(),
            app_icon: String::new(),
            summary: summary.into(),
            body: String::new(),
            urgency: Default::default(),
            timeout_ms: -1,
            actions: vec![],
            hints: NotificationHints::default(),
        }
    }

    fn test_notification_with_action(summary: &str, action_key: &str) -> Notification {
        Notification {
            app_name: "test".into(),
            app_icon: String::new(),
            summary: summary.into(),
            body: String::new(),
            urgency: Default::default(),
            timeout_ms: -1,
            actions: vec![NotificationAction {
                key: action_key.to_string(),
                label: "Test Action".to_string(),
            }],
            hints: NotificationHints::default(),
        }
    }

    #[test]
    fn image_hints_are_omitted_from_extra_debug_dump() {
        let mut raw_hints: HashMap<String, zvariant::OwnedValue> = HashMap::new();
        raw_hints.insert("image-data".to_string(), true.into());
        raw_hints.insert("suppress-sound".to_string(), false.into());
        raw_hints.insert(
            "blob".to_string(),
            zvariant::OwnedValue::try_from(zvariant::Value::from(vec![1_u8, 2, 3])).unwrap(),
        );

        let (_urgency, hints) = parse_hints(&raw_hints);

        assert_eq!(
            hints.extra.get("image-data").map(String::as_str),
            Some("<omitted image payload>")
        );
        assert!(
            hints
                .extra
                .get("blob")
                .is_some_and(|v| v.contains("<omitted binary payload signature=ay>"))
        );
        assert!(
            hints
                .extra
                .get("suppress-sound")
                .is_some_and(|v| v.contains("Bool(false)"))
        );
    }

    #[tokio::test]
    async fn replacement_uses_same_id() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        let id = source.notify(test_notification("first"), 0).await.unwrap();
        let _ = rx.recv().await;

        let replaced_id = source
            .notify(test_notification("second"), id)
            .await
            .unwrap();
        assert_eq!(id, replaced_id);

        match rx.recv().await.unwrap() {
            NotificationEvent::Replaced { id: event_id, .. } => assert_eq!(event_id, id),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_replaces_id_allocates_fresh_id() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        let first_id = source.notify(test_notification("first"), 0).await.unwrap();
        let _ = rx.recv().await;

        let second_id = source
            .notify(test_notification("second"), 999_999)
            .await
            .unwrap();
        assert_ne!(first_id, second_id);

        match rx.recv().await.unwrap() {
            NotificationEvent::Received { id, notification } => {
                assert_eq!(id, second_id);
                assert_eq!(notification.summary, "second");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_emits_closed_expired_event() {
        let cfg = SourceConfig {
            default_timeout_ms: Some(20),
            ..SourceConfig::default()
        };
        let (source, mut rx) = WispSource::new(cfg);

        let id = source
            .notify(test_notification("expires"), 0)
            .await
            .unwrap();

        let first = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match first {
            NotificationEvent::Received { id: event_id, .. } => assert_eq!(event_id, id),
            other => panic!("unexpected event: {other:?}"),
        }

        let second = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match second {
            NotificationEvent::Closed {
                id: event_id,
                reason,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(reason, CloseReason::Expired);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn negative_timeout_without_default_is_persistent() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        let id = source
            .notify(test_notification("persistent"), 0)
            .await
            .unwrap();

        match rx.recv().await.unwrap() {
            NotificationEvent::Received { id: event_id, .. } => assert_eq!(event_id, id),
            other => panic!("unexpected event: {other:?}"),
        }

        let maybe_event = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(maybe_event.is_err(), "unexpected timeout event was emitted");

        let snapshot = source.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].0, id);
    }

    #[tokio::test]
    async fn zero_timeout_never_schedules_expiry() {
        let (source, mut rx) = WispSource::new(SourceConfig {
            default_timeout_ms: Some(10),
            ..SourceConfig::default()
        });

        let id = source
            .notify(
                Notification {
                    timeout_ms: 0,
                    ..test_notification("persistent-zero")
                },
                0,
            )
            .await
            .unwrap();

        match rx.recv().await.unwrap() {
            NotificationEvent::Received { id: event_id, .. } => assert_eq!(event_id, id),
            other => panic!("unexpected event: {other:?}"),
        }

        let maybe_event = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(maybe_event.is_err(), "unexpected timeout event was emitted");

        let snapshot = source.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].0, id);
    }

    #[tokio::test]
    async fn replacement_resets_timeout_generation() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        let id = source
            .notify(
                Notification {
                    timeout_ms: 20,
                    ..test_notification("first")
                },
                0,
            )
            .await
            .unwrap();
        let _ = rx.recv().await;

        tokio::time::sleep(Duration::from_millis(10)).await;

        let replaced_id = source
            .notify(
                Notification {
                    timeout_ms: 80,
                    ..test_notification("second")
                },
                id,
            )
            .await
            .unwrap();
        assert_eq!(replaced_id, id);

        let replaced = rx.recv().await.unwrap();
        match replaced {
            NotificationEvent::Replaced { id: event_id, .. } => assert_eq!(event_id, id),
            other => panic!("unexpected event: {other:?}"),
        }

        let maybe_event = tokio::time::timeout(Duration::from_millis(30), rx.recv()).await;
        assert!(
            maybe_event.is_err(),
            "replacement should have canceled the old timeout generation"
        );

        let closed = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match closed {
            NotificationEvent::Closed {
                id: event_id,
                reason,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(reason, CloseReason::Expired);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn invoke_action_emits_action_and_closed_events() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        let id = source
            .notify(test_notification_with_action("action", "open"), 0)
            .await
            .unwrap();

        let first = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match first {
            NotificationEvent::Received { id: event_id, .. } => assert_eq!(event_id, id),
            other => panic!("unexpected event: {other:?}"),
        }

        let invoked = source.invoke_action(id, "open").await.unwrap();
        assert!(invoked);

        let second = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match second {
            NotificationEvent::ActionInvoked { id: event_id, .. } => assert_eq!(event_id, id),
            other => panic!("unexpected event: {other:?}"),
        }

        let third = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match third {
            NotificationEvent::Closed {
                id: event_id,
                reason,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(reason, CloseReason::Dismissed);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn invoke_action_returns_false_for_unknown_action() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        let id = source
            .notify(test_notification("no action"), 0)
            .await
            .unwrap();
        let _ = rx.recv().await;

        let invoked = source.invoke_action(id, "open").await.unwrap();
        assert!(!invoked);

        let maybe_event = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(maybe_event.is_err(), "unexpected event was emitted");
    }

    #[tokio::test]
    async fn snapshot_reflects_replace_and_close_state() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        let id = source.notify(test_notification("first"), 0).await.unwrap();
        let _ = rx.recv().await;

        source
            .notify(test_notification("second"), id)
            .await
            .unwrap();
        let _ = rx.recv().await;

        let snapshot = source.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].0, id);
        assert_eq!(snapshot[0].1.summary, "second");

        let closed = source.close(id, CloseReason::ClosedByCall).await.unwrap();
        assert!(closed);
        let _ = rx.recv().await;

        assert!(source.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn close_unknown_id_is_safe_noop() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        let closed = source.close(42, CloseReason::ClosedByCall).await.unwrap();
        assert!(!closed);

        let maybe_event = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(maybe_event.is_err(), "unexpected event was emitted");
    }

    #[test]
    fn parse_hints_extracts_known_fields_exactly() {
        let mut raw_hints: HashMap<String, zvariant::OwnedValue> = HashMap::new();
        raw_hints.insert("urgency".to_string(), 0_u8.into());
        raw_hints.insert(
            "category".to_string(),
            zvariant::OwnedValue::from(zvariant::Str::from("email.arrived")),
        );
        raw_hints.insert(
            "desktop-entry".to_string(),
            zvariant::OwnedValue::from(zvariant::Str::from("org.example.Mail")),
        );
        raw_hints.insert("transient".to_string(), zvariant::OwnedValue::from(true));

        let (urgency, hints) = parse_hints(&raw_hints);

        assert_eq!(urgency, Urgency::Low);
        assert_eq!(hints.category.as_deref(), Some("email.arrived"));
        assert_eq!(hints.desktop_entry.as_deref(), Some("org.example.Mail"));
        assert_eq!(hints.transient, Some(true));
        assert!(hints.extra.is_empty());
    }

    #[test]
    fn parse_actions_handles_empty_and_odd_action_lists_safely() {
        assert!(parse_actions(Vec::new()).is_empty());

        let parsed = parse_actions(vec!["only-key".to_string()]);
        assert!(parsed.is_empty());

        let parsed = parse_actions(vec![
            "open".to_string(),
            "Open".to_string(),
            "dangling".to_string(),
        ]);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].key, "open");
        assert_eq!(parsed[0].label, "Open");
    }

    async fn setup_dbus_source_for_test(
        suffix: &str,
    ) -> Option<(
        SourceConfig,
        WispSource,
        mpsc::Receiver<NotificationEvent>,
        DbusService,
        zbus::Connection,
    )> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();

        let cfg = SourceConfig {
            dbus_name: format!("org.wispd.{suffix}.{unique}"),
            ..SourceConfig::default()
        };

        let Ok((source, rx, service)) = WispSource::start_dbus(cfg.clone()).await else {
            eprintln!("skipping dbus integration test: session bus unavailable");
            return None;
        };

        let Ok(client) = zbus::Connection::session().await else {
            eprintln!("skipping dbus integration test: session bus unavailable");
            return None;
        };

        Some((cfg, source, rx, service, client))
    }

    async fn make_notifications_proxy<'a>(
        client: &'a zbus::Connection,
        cfg: &'a SourceConfig,
    ) -> zbus::Result<zbus::Proxy<'a>> {
        zbus::Proxy::new(
            client,
            cfg.dbus_name.as_str(),
            cfg.dbus_path.as_str(),
            DBUS_INTERFACE,
        )
        .await
    }

    #[tokio::test]
    async fn dbus_notify_emits_received_event() {
        let Some((cfg, _source, mut rx, _service, client)) =
            setup_dbus_source_for_test("Notify").await
        else {
            return;
        };

        let mut hints = HashMap::<String, zvariant::OwnedValue>::new();
        hints.insert("urgency".to_string(), zvariant::OwnedValue::from(2_u8));
        hints.insert(
            "category".to_string(),
            zvariant::OwnedValue::from(zvariant::Str::from("mail.arrived")),
        );
        hints.insert(
            "desktop-entry".to_string(),
            zvariant::OwnedValue::from(zvariant::Str::from("org.example.Mail")),
        );
        hints.insert("transient".to_string(), zvariant::OwnedValue::from(true));
        hints.insert("x-foo".to_string(), zvariant::OwnedValue::from(42_i32));

        let msg = client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "Notify",
                &(
                    String::from("test-client"),
                    0_u32,
                    String::from("test-icon"),
                    String::from("hello"),
                    String::from("world"),
                    Vec::<String>::new(),
                    hints,
                    2_500_i32,
                ),
            )
            .await
            .unwrap();

        let id: u32 = msg.body().deserialize().unwrap();

        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();

        match event {
            NotificationEvent::Received {
                id: event_id,
                notification,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(notification.app_icon, "test-icon");
                assert_eq!(notification.urgency, Urgency::Critical);
                assert_eq!(notification.hints.category.as_deref(), Some("mail.arrived"));
                assert_eq!(
                    notification.hints.desktop_entry.as_deref(),
                    Some("org.example.Mail")
                );
                assert_eq!(notification.hints.transient, Some(true));
                assert!(notification.hints.extra.contains_key("x-foo"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn dbus_close_notification_emits_closed_event() {
        let Some((cfg, _source, mut rx, _service, client)) =
            setup_dbus_source_for_test("Close").await
        else {
            return;
        };

        let notify_msg = client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "Notify",
                &(
                    String::from("test-client"),
                    0_u32,
                    String::new(),
                    String::from("hello"),
                    String::from("world"),
                    Vec::<String>::new(),
                    HashMap::<String, zvariant::OwnedValue>::new(),
                    10_000_i32,
                ),
            )
            .await
            .unwrap();
        let id: u32 = notify_msg.body().deserialize().unwrap();
        let _ = rx.recv().await;

        client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "CloseNotification",
                &(id),
            )
            .await
            .unwrap();

        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();

        match event {
            NotificationEvent::Closed {
                id: event_id,
                reason,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(reason, CloseReason::ClosedByCall);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn dbus_close_notification_emits_notification_closed_signal() {
        let Some((cfg, _source, mut rx, _service, client)) =
            setup_dbus_source_for_test("CloseSignal").await
        else {
            return;
        };

        let proxy = make_notifications_proxy(&client, &cfg).await.unwrap();
        let mut closed_stream = proxy.receive_signal("NotificationClosed").await.unwrap();

        let notify_msg = client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "Notify",
                &(
                    String::from("test-client"),
                    0_u32,
                    String::new(),
                    String::from("hello"),
                    String::from("world"),
                    Vec::<String>::new(),
                    HashMap::<String, zvariant::OwnedValue>::new(),
                    10_000_i32,
                ),
            )
            .await
            .unwrap();
        let id: u32 = notify_msg.body().deserialize().unwrap();
        let _ = rx.recv().await;

        client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "CloseNotification",
                &(id),
            )
            .await
            .unwrap();

        let signal = tokio::time::timeout(Duration::from_secs(2), closed_stream.next())
            .await
            .unwrap()
            .unwrap();
        let (signal_id, reason_code): (u32, u32) = signal.body().deserialize().unwrap();
        assert_eq!(signal_id, id);
        assert_eq!(reason_code, 3);
    }

    #[tokio::test]
    async fn invoke_action_emits_action_invoked_signal() {
        let Some((cfg, source, mut rx, _service, client)) =
            setup_dbus_source_for_test("ActionSignal").await
        else {
            return;
        };

        let proxy = make_notifications_proxy(&client, &cfg).await.unwrap();
        let mut action_stream = proxy.receive_signal("ActionInvoked").await.unwrap();

        let notify_msg = client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "Notify",
                &(
                    String::from("test-client"),
                    0_u32,
                    String::new(),
                    String::from("hello"),
                    String::from("world"),
                    vec![String::from("open"), String::from("Open")],
                    HashMap::<String, zvariant::OwnedValue>::new(),
                    10_000_i32,
                ),
            )
            .await
            .unwrap();
        let id: u32 = notify_msg.body().deserialize().unwrap();
        let _ = rx.recv().await;

        let invoked = source.invoke_action(id, "open").await.unwrap();
        assert!(invoked);

        let signal = tokio::time::timeout(Duration::from_secs(2), action_stream.next())
            .await
            .unwrap()
            .unwrap();
        let (signal_id, action_key): (u32, String) = signal.body().deserialize().unwrap();
        assert_eq!(signal_id, id);
        assert_eq!(action_key, "open");
    }

    #[tokio::test]
    async fn dbus_get_capabilities_returns_configured_capabilities() {
        let Some((cfg, _source, _rx, _service, client)) =
            setup_dbus_source_for_test("Capabilities").await
        else {
            return;
        };

        let msg = client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "GetCapabilities",
                &(),
            )
            .await
            .unwrap();

        let capabilities: Vec<String> = msg.body().deserialize().unwrap();
        assert_eq!(capabilities, cfg.capabilities);
    }

    #[tokio::test]
    async fn dbus_get_server_information_returns_configured_values() {
        let Some((cfg, _source, _rx, _service, client)) =
            setup_dbus_source_for_test("ServerInfo").await
        else {
            return;
        };

        let msg = client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "GetServerInformation",
                &(),
            )
            .await
            .unwrap();

        let info: (String, String, String, String) = msg.body().deserialize().unwrap();
        assert_eq!(
            info,
            (
                cfg.server_name,
                cfg.server_vendor,
                cfg.server_version,
                cfg.spec_version,
            )
        );
    }

    #[tokio::test]
    async fn dbus_capabilities_and_server_info_stay_correct_after_runtime_reload() {
        let Some((cfg, source, _rx, _service, client)) =
            setup_dbus_source_for_test("ReloadInfo").await
        else {
            return;
        };

        source.update_runtime_config(vec!["body".to_string(), "actions".to_string()], Some(50));

        let caps_msg = client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "GetCapabilities",
                &(),
            )
            .await
            .unwrap();
        let capabilities: Vec<String> = caps_msg.body().deserialize().unwrap();
        assert_eq!(
            capabilities,
            vec!["body".to_string(), "actions".to_string()]
        );

        let info_msg = client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "GetServerInformation",
                &(),
            )
            .await
            .unwrap();
        let info: (String, String, String, String) = info_msg.body().deserialize().unwrap();
        assert_eq!(
            info,
            (
                cfg.server_name,
                cfg.server_vendor,
                cfg.server_version,
                cfg.spec_version,
            )
        );
    }

    #[tokio::test]
    async fn runtime_config_update_changes_capabilities_and_default_timeout() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        source.update_runtime_config(vec!["body".to_string(), "actions".to_string()], Some(25));
        assert_eq!(
            source.capabilities(),
            vec!["body".to_string(), "actions".to_string()]
        );

        let id = source
            .notify(test_notification("reload-timeout"), 0)
            .await
            .unwrap();
        match rx.recv().await.unwrap() {
            NotificationEvent::Received { id: event_id, .. } => assert_eq!(event_id, id),
            other => panic!("unexpected event: {other:?}"),
        }

        match tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            NotificationEvent::Closed {
                id: event_id,
                reason,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(reason, CloseReason::Expired);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn runtime_config_update_does_not_duplicate_or_drop_active_timers() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        let id = source
            .notify(
                Notification {
                    timeout_ms: 40,
                    ..test_notification("active-timer")
                },
                0,
            )
            .await
            .unwrap();

        match rx.recv().await.unwrap() {
            NotificationEvent::Received { id: event_id, .. } => assert_eq!(event_id, id),
            other => panic!("unexpected event: {other:?}"),
        }

        source.update_runtime_config(vec!["body".to_string(), "actions".to_string()], Some(5));

        match tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            NotificationEvent::Closed {
                id: event_id,
                reason,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(reason, CloseReason::Expired);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let maybe_event = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(
            maybe_event.is_err(),
            "unexpected extra timeout event was emitted"
        );
    }

    #[tokio::test]
    async fn dbus_notify_burst_preserves_order_and_ids() {
        let Some((cfg, _source, mut rx, _service, client)) =
            setup_dbus_source_for_test("Burst").await
        else {
            return;
        };

        let mut ids = Vec::new();
        for summary in ["one", "two", "three"] {
            let msg = client
                .call_method(
                    Some(cfg.dbus_name.as_str()),
                    cfg.dbus_path.as_str(),
                    Some(DBUS_INTERFACE),
                    "Notify",
                    &(
                        String::from("test-client"),
                        0_u32,
                        String::new(),
                        String::from(summary),
                        String::new(),
                        Vec::<String>::new(),
                        HashMap::<String, zvariant::OwnedValue>::new(),
                        10_000_i32,
                    ),
                )
                .await
                .unwrap();
            ids.push(msg.body().deserialize::<u32>().unwrap());
        }

        assert_eq!(ids.len(), 3);
        assert!(ids[0] < ids[1] && ids[1] < ids[2]);

        for (expected_id, expected_summary) in ids.into_iter().zip(["one", "two", "three"]) {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                NotificationEvent::Received { id, notification } => {
                    assert_eq!(id, expected_id);
                    assert_eq!(notification.summary, expected_summary);
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn dbus_replace_storm_leaves_single_final_live_notification() {
        let Some((cfg, source, mut rx, _service, client)) =
            setup_dbus_source_for_test("ReplaceStorm").await
        else {
            return;
        };

        let first = client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "Notify",
                &(
                    String::from("test-client"),
                    0_u32,
                    String::new(),
                    String::from("first"),
                    String::new(),
                    Vec::<String>::new(),
                    HashMap::<String, zvariant::OwnedValue>::new(),
                    10_000_i32,
                ),
            )
            .await
            .unwrap();
        let id: u32 = first.body().deserialize().unwrap();
        let _ = rx.recv().await;

        for summary in ["second", "third", "final"] {
            let msg = client
                .call_method(
                    Some(cfg.dbus_name.as_str()),
                    cfg.dbus_path.as_str(),
                    Some(DBUS_INTERFACE),
                    "Notify",
                    &(
                        String::from("test-client"),
                        id,
                        String::new(),
                        String::from(summary),
                        String::new(),
                        Vec::<String>::new(),
                        HashMap::<String, zvariant::OwnedValue>::new(),
                        10_000_i32,
                    ),
                )
                .await
                .unwrap();
            let replaced_id: u32 = msg.body().deserialize().unwrap();
            assert_eq!(replaced_id, id);
            match rx.recv().await.unwrap() {
                NotificationEvent::Replaced {
                    id: event_id,
                    current,
                    ..
                } => {
                    assert_eq!(event_id, id);
                    assert_eq!(current.summary, summary);
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }

        let snapshot = source.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].0, id);
        assert_eq!(snapshot[0].1.summary, "final");
    }

    #[tokio::test]
    async fn close_notification_during_active_timeout_emits_single_final_close() {
        let Some((cfg, _source, mut rx, _service, client)) =
            setup_dbus_source_for_test("CloseVsTimeout").await
        else {
            return;
        };

        let proxy = make_notifications_proxy(&client, &cfg).await.unwrap();
        let mut closed_stream = proxy.receive_signal("NotificationClosed").await.unwrap();

        let notify_msg = client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "Notify",
                &(
                    String::from("test-client"),
                    0_u32,
                    String::new(),
                    String::from("hello"),
                    String::new(),
                    Vec::<String>::new(),
                    HashMap::<String, zvariant::OwnedValue>::new(),
                    200_i32,
                ),
            )
            .await
            .unwrap();
        let id: u32 = notify_msg.body().deserialize().unwrap();
        let _ = rx.recv().await;

        client
            .call_method(
                Some(cfg.dbus_name.as_str()),
                cfg.dbus_path.as_str(),
                Some(DBUS_INTERFACE),
                "CloseNotification",
                &(id),
            )
            .await
            .unwrap();

        match tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            NotificationEvent::Closed {
                id: event_id,
                reason,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(reason, CloseReason::ClosedByCall);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let signal = tokio::time::timeout(Duration::from_secs(2), closed_stream.next())
            .await
            .unwrap()
            .unwrap();
        let (signal_id, reason_code): (u32, u32) = signal.body().deserialize().unwrap();
        assert_eq!(signal_id, id);
        assert_eq!(reason_code, 3);

        tokio::time::sleep(Duration::from_millis(250)).await;

        let maybe_event = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(
            maybe_event.is_err(),
            "unexpected extra close event was emitted"
        );

        let maybe_signal =
            tokio::time::timeout(Duration::from_millis(50), closed_stream.next()).await;
        assert!(
            maybe_signal.is_err(),
            "unexpected extra close signal was emitted"
        );
    }

    #[tokio::test]
    async fn invoking_action_after_replacement_targets_current_generation() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        let id = source
            .notify(test_notification_with_action("first", "open"), 0)
            .await
            .unwrap();
        let _ = rx.recv().await;

        let replaced = Notification {
            summary: "second".to_string(),
            actions: vec![NotificationAction {
                key: "reply".to_string(),
                label: "Reply".to_string(),
            }],
            ..test_notification("second")
        };
        let replaced_id = source.notify(replaced, id).await.unwrap();
        assert_eq!(replaced_id, id);
        let _ = rx.recv().await;

        let old_action_result = source.invoke_action(id, "open").await.unwrap();
        assert!(!old_action_result);

        let new_action_result = source.invoke_action(id, "reply").await.unwrap();
        assert!(new_action_result);

        match rx.recv().await.unwrap() {
            NotificationEvent::ActionInvoked {
                id: event_id,
                action_key,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(action_key, "reply");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match rx.recv().await.unwrap() {
            NotificationEvent::Closed {
                id: event_id,
                reason,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(reason, CloseReason::Dismissed);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn duplicate_action_keys_are_handled_safely() {
        let (source, mut rx) = WispSource::new(SourceConfig::default());

        let id = source
            .notify(
                Notification {
                    actions: vec![
                        NotificationAction {
                            key: "open".to_string(),
                            label: "Open primary".to_string(),
                        },
                        NotificationAction {
                            key: "open".to_string(),
                            label: "Open secondary".to_string(),
                        },
                    ],
                    ..test_notification("dup-actions")
                },
                0,
            )
            .await
            .unwrap();
        let _ = rx.recv().await;

        let invoked = source.invoke_action(id, "open").await.unwrap();
        assert!(invoked);

        match rx.recv().await.unwrap() {
            NotificationEvent::ActionInvoked {
                id: event_id,
                action_key,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(action_key, "open");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match rx.recv().await.unwrap() {
            NotificationEvent::Closed {
                id: event_id,
                reason,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(reason, CloseReason::Dismissed);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
