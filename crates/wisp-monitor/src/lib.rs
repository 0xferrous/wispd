use std::collections::HashMap;

use anyhow::{Context, Result};
use zbus::{Message, message::Type as MessageType, zvariant};

pub const DBUS_NAME: &str = "org.freedesktop.DBus";
pub const DBUS_PATH: &str = "/org/freedesktop/DBus";
pub const DBUS_MONITORING_IFACE: &str = "org.freedesktop.DBus.Monitoring";
pub const NOTIFY_IFACE: &str = "org.freedesktop.Notifications";

#[derive(Debug, Clone)]
pub struct NotifyCall {
    pub app_name: String,
    pub replaces_id: u32,
    pub summary: String,
    pub body: String,
    pub actions: Vec<String>,
    pub hints: HashMap<String, zvariant::OwnedValue>,
    pub expire_timeout: i32,
}

#[derive(Debug, Clone)]
pub enum NotificationMessage {
    Notify(NotifyCall),
    CloseNotification { id: u32 },
    NotificationClosed { id: u32, reason: u32 },
    ActionInvoked { id: u32, action_key: String },
}

pub async fn become_monitor(conn: &zbus::Connection, rules: Vec<String>) -> Result<()> {
    conn.call_method(
        Some(DBUS_NAME),
        DBUS_PATH,
        Some(DBUS_MONITORING_IFACE),
        "BecomeMonitor",
        &(rules, 0u32),
    )
    .await
    .context("failed to become D-Bus monitor")?;

    Ok(())
}

pub fn rules_all_notifications() -> Vec<String> {
    vec![
        format!("type='method_call',interface='{NOTIFY_IFACE}'"),
        format!("type='signal',interface='{NOTIFY_IFACE}'"),
    ]
}

pub fn rules_notify_only() -> Vec<String> {
    vec![format!(
        "type='method_call',interface='{NOTIFY_IFACE}',member='Notify'"
    )]
}

pub fn parse_notification_message(msg: &Message) -> Result<Option<NotificationMessage>> {
    let header = msg.header();

    let iface_is_notify = header
        .interface()
        .is_some_and(|iface| iface.as_str() == NOTIFY_IFACE);

    if !iface_is_notify {
        return Ok(None);
    }

    let member = header.member().map(|m| m.as_str());

    match (msg.message_type(), member) {
        (MessageType::MethodCall, Some("Notify")) => {
            let (app_name, replaces_id, _app_icon, summary, body, actions, hints, expire_timeout) =
                msg.body().deserialize::<(
                    String,
                    u32,
                    String,
                    String,
                    String,
                    Vec<String>,
                    HashMap<String, zvariant::OwnedValue>,
                    i32,
                )>()?;

            Ok(Some(NotificationMessage::Notify(NotifyCall {
                app_name,
                replaces_id,
                summary,
                body,
                actions,
                hints,
                expire_timeout,
            })))
        }
        (MessageType::MethodCall, Some("CloseNotification")) => {
            let (id,) = msg.body().deserialize::<(u32,)>()?;
            Ok(Some(NotificationMessage::CloseNotification { id }))
        }
        (MessageType::Signal, Some("NotificationClosed")) => {
            let (id, reason) = msg.body().deserialize::<(u32, u32)>()?;
            Ok(Some(NotificationMessage::NotificationClosed { id, reason }))
        }
        (MessageType::Signal, Some("ActionInvoked")) => {
            let (id, action_key) = msg.body().deserialize::<(u32, String)>()?;
            Ok(Some(NotificationMessage::ActionInvoked { id, action_key }))
        }
        _ => Ok(None),
    }
}
