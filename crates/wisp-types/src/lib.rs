use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Notification urgency level as defined by freedesktop notifications.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Urgency {
    /// Low-priority notification.
    Low,
    /// Normal-priority notification.
    #[default]
    Normal,
    /// Critical notification.
    Critical,
}

/// Reason why a notification was closed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CloseReason {
    /// Notification timed out and expired.
    Expired,
    /// Notification was dismissed (for example by user interaction).
    Dismissed,
    /// Notification was closed by a direct close call.
    ClosedByCall,
    /// Unknown/unspecified reason.
    Undefined,
}

/// An actionable button attached to a notification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationAction {
    /// Stable action identifier used by clients.
    pub key: String,
    /// Human-readable label shown in UI.
    pub label: String,
}

/// Parsed/normalized hint fields from the freedesktop `hints` map.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NotificationHints {
    /// Notification category (e.g. `email.arrived`).
    pub category: Option<String>,
    /// Desktop entry identifier for matching app metadata.
    pub desktop_entry: Option<String>,
    /// Whether this is marked transient by sender.
    pub transient: Option<bool>,
    /// Unrecognized hints preserved as debug strings.
    pub extra: HashMap<String, String>,
}

/// Normalized notification data used by `wisp` components.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Notification {
    /// Name of the sending application.
    pub app_name: String,
    /// App icon name/path from sender.
    pub app_icon: String,
    /// Notification title/summary.
    pub summary: String,
    /// Notification body text.
    pub body: String,
    /// Notification urgency.
    pub urgency: Urgency,
    /// Requested timeout in milliseconds.
    pub timeout_ms: i32,
    /// Declared actions for this notification.
    pub actions: Vec<NotificationAction>,
    /// Parsed notification hints.
    pub hints: NotificationHints,
}

/// Event emitted by the source daemon lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NotificationEvent {
    /// A new notification was received.
    Received {
        /// Notification id allocated by the source.
        id: u32,
        /// Notification payload.
        notification: Box<Notification>,
    },
    /// A notification was closed.
    Closed {
        /// Closed notification id.
        id: u32,
        /// Closure reason.
        reason: CloseReason,
    },
    /// A notification action was invoked.
    ActionInvoked {
        /// Notification id for which action was triggered.
        id: u32,
        /// Invoked action key.
        action_key: String,
    },
    /// An existing notification was replaced in-place.
    Replaced {
        /// Notification id that was replaced.
        id: u32,
        /// Previous notification payload.
        previous: Box<Notification>,
        /// New notification payload.
        current: Box<Notification>,
    },
}
