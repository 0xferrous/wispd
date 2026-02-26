# wispd architecture (living doc)

This file explains how the project works today and should be updated as implementation changes.

## 1) D-Bus basics for this project

`notify-send` and most Linux apps send desktop notifications over D-Bus to:

- bus name: `org.freedesktop.Notifications`
- object path: `/org/freedesktop/Notifications`
- interface: `org.freedesktop.Notifications`

Only **one process** can own that bus name at a time.

- If `mako`/`dunst` already owns it, `wisp-debug`/`wispd` cannot start as server.
- The notification daemon is the **server** (name owner).
- Apps are **clients** (they call methods like `Notify`).

## 2) Workspace layout

```text
crates/
  wisp-types   # shared Rust types/events
  wisp-source  # D-Bus server + notification store + event stream

bins/
  wispd        # main daemon entrypoint (currently logs events)
  wisp-debug   # debug daemon entrypoint that logs incoming notifications
```

## 3) Current runtime flow

1. `wisp-debug` or `wispd` calls `WispSource::start_dbus(config)`.
2. `wisp-source` connects to session bus, requests name `org.freedesktop.Notifications`, and serves interface at `/org/freedesktop/Notifications`.
3. Client apps call `Notify`.
4. `wisp-source` converts D-Bus args into `wisp_types::Notification`.
5. Notification is inserted/replaced in in-memory store.
6. `wisp-source` schedules timeout expiry (if applicable).
7. `wisp-source` emits `NotificationEvent` through `tokio::mpsc`.
8. Binary receives events and logs them (UI rendering not implemented yet).

## 4) `wisp-source` responsibilities

Implemented now:

- Owns notification state (`HashMap<u32, StoredNotification>`) with generation counters
- Allocates IDs
- Replacement semantics:
  - `replaces_id == 0`: new ID
  - existing `replaces_id`: replace in place, keep same ID, increment generation
  - missing `replaces_id`: create new ID
- Timeout/expiry scheduler
  - `expire_timeout > 0`: uses requested timeout
  - `expire_timeout < 0`: uses `default_timeout_ms`
  - `expire_timeout == 0`: no automatic expiry
- Exposes snapshot API (`snapshot()`)
- Exposes action API (`invoke_action(id, action_key)`)
- D-Bus methods:
  - `Notify`
  - `CloseNotification`
  - `GetCapabilities`
  - `GetServerInformation`
- Declares D-Bus signals:
  - `NotificationClosed`
  - `ActionInvoked`
- Parses core hints (`urgency`, `category`, `desktop-entry`, `transient`) and preserves unknown hints as debug strings
- Emits `NotificationClosed` signal for close paths handled by source (`CloseNotification`, timeout expiry, action dismiss)
- Emits `ActionInvoked` signal when an action is invoked

Not implemented yet:

- wiring UI/input to call `invoke_action`
- richer hint coverage (images/sound/etc beyond current parsed subset)

## 5) Types and events

Main shared types in `wisp-types`:

- `Notification` (includes `app_icon`, `actions`, `hints`)
- `NotificationHints` (`category`, `desktop_entry`, `transient`, `extra`)
- `NotificationAction`
- `Urgency`
- `CloseReason`
- `NotificationEvent` (`Received`, `Replaced`, `Closed`, `ActionInvoked`)

Event transport is currently `tokio::mpsc` (single consumer stream per source instance).

## 6) Config surface (current)

`SourceConfig` currently includes:

- capabilities list (reported by `GetCapabilities`)
- event channel capacity
- D-Bus name/path
- server information strings for `GetServerInformation`
- `default_timeout_ms` (used when client sends negative timeout)

## 7) Testing status

Implemented tests in `wisp-source`:

- replacement keeps same ID
- timeout expiry emits `Closed(Expired)` event
- action invoke emits `ActionInvoked` + `Closed(Dismissed)`
- unknown action returns false and emits no extra events
- D-Bus integration tests (skip when session bus unavailable):
  - `Notify` emits received event (including parsed icon/hints)
  - `CloseNotification` emits closed event with `ClosedByCall`
  - `NotificationClosed` signal is emitted with expected reason code
  - `ActionInvoked` signal is emitted for action invocation
  - `GetCapabilities` returns configured capabilities
  - `GetServerInformation` returns configured values

## 8) How to run debug daemon

```bash
cargo run -p wisp-debug
```

In another terminal:

```bash
notify-send "hello" "from notify-send"
```

If startup fails with "name already taken on the bus", stop the currently running notification daemon first.
