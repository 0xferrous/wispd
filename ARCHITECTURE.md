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
  wispd        # iced + layer-shell frontend with queue policy + popup rendering
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
8. `wispd` runs `wisp-source` on a dedicated Tokio runtime thread and forwards events to the UI via a std channel.
9. `wispd` applies queue policy (max visible, newest on top, replacement in-place).
10. `wispd` renders notification popups via `iced` + `iced_layershell`.

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

- interactive UI controls for actions/closing (current action trigger lives in `wisp-debug` commands)
- richer hint coverage (images/sound/etc beyond current parsed subset)
- polished visual styling/layout behavior expected from mature daemons

## 5) Types and events

Main shared types in `wisp-types`:

- `Notification` (includes `app_icon`, `actions`, `hints`)
- `NotificationHints` (`category`, `desktop_entry`, `transient`, `extra`)
- `NotificationAction`
- `Urgency`
- `CloseReason`
- `NotificationEvent` (`Received`, `Replaced`, `Closed`, `ActionInvoked`)

Event transport is currently `tokio::mpsc` (single consumer stream per source instance).

`wispd` currently applies queue behavior:
- max visible: 5
- newest notifications at top
- replacement updates existing item in-place (keeps slot)
- close removes item

## 6) Config surface (current)

Config file is loaded from:
- `$XDG_CONFIG_HOME/wispd/config.toml`
- fallback: `~/.config/wispd/config.toml`

`source` config currently supports:
- `capabilities` list (reported by `GetCapabilities`)
- `default_timeout_ms` (used when incoming timeout is negative)
  - if unset, negative incoming timeouts are treated as persistent

`ui` config currently supports:
- `format` string with placeholders (`{id}`, `{app_name}`, `{summary}`, `{body}`, `{urgency}`)
- `max_visible`
- `width`
- `height`
- `gap`
- `padding`
- `font_size`
- `anchor`
- `margin` (`top`, `right`, `bottom`, `left`)
- urgency colors (`low`, `normal`, `critical`) plus base `background` and `text`

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

`wisp-debug` also accepts stdin commands:

- `list`
- `close <id>`
- `action <id> <action-key>`
- `help`
- `quit`

If startup fails with "name already taken on the bus", stop the currently running notification daemon first.

`wispd` requires a Wayland session and Wayland runtime libraries. If you see `NoWaylandLib`, run inside `nix develop` (this flake exports `LD_LIBRARY_PATH` for `wayland`/`libxkbcommon`) and verify `WAYLAND_DISPLAY` is set.
