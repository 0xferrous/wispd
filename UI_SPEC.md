# wispd UI spec (v1)

Status: draft, locked decisions from implementation planning.

## 1. Core display

Each notification is rendered as a separate popup card/window-like block in a stack (not a single persistent combined panel).

UI rendering is format-string based.

- Default format:
  - `"{app_name}: {summary}\n{body}"`
- Supported placeholders in v1:
  - `{id}`
  - `{app_name}`
  - `{summary}`
  - `{body}`
  - `{urgency}`
- Unknown placeholders are left literal.

## 2. Queue behavior

- `max_visible` default: `5`
- Newest notifications at top.
- Each visible notification appears as its own popup card with spacing (`gap`) between cards.
- Replacements (`replaces_id`) update in place (keep slot).
- Overflow policy: UI-only truncation.
  - Oldest visible item is removed from UI view.
  - Source state remains unchanged.

## 3. Lifetime behavior

All config is TOML.

- Config path: XDG config path (`$XDG_CONFIG_HOME/wispd/config.toml`, fallback `~/.config/wispd/config.toml`).
- `default_timeout_ms` is user-configurable.
- If `default_timeout_ms` is unset, notifications are persistent by default.
- Explicit per-notification timeout is still respected.

## 4. Interaction (v1)

Implemented in v1:
- Click notification body to dismiss.
- Render action buttons and invoke actions.

Future:
- Right-click dismiss.
- Hover pauses timeout.

## 5. Styling/layout config (v1)

TOML-configurable:
- anchor
- margin
- gap
- max_visible
- width
- height
- urgency colors (`low`, `normal`, `critical`)
- per-notification padding
- font size

## 6. Reliability expectations

- UI must not block D-Bus response path.
- Event backpressure must not hang `Notify`.
- Malformed format string must not crash UI (fallback to default format).

## 7. Deferred but important (post-v1)

- Icon rendering
- Markup parsing
- Animations
- Notification history center
- Rich hint rendering (image/progress/sound)

## 8. Acceptance checklist (v1)

1. `notify-send "t" "b"` shows popup with configured format.
2. Burst of notifications above `max_visible` keeps newest visible and does not hang sender.
3. Replace-id updates existing popup in place.
4. Timeout behavior follows config (`default_timeout_ms` unset => persistent).
5. Click dismiss works.
6. Action button invokes action and closes.
7. Malformed format string does not crash UI.
