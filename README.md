# wispd

`wispd` is an experimental Wayland notification daemon for `org.freedesktop.Notifications`.
It includes:

- **`wispd`**: layer-shell popup UI (one popup window per notification)
- **`wisp-debug`**: CLI/debug daemon to inspect incoming notifications and test close/action flows
- Reusable crates:
  - `wisp-source` (D-Bus server + notification lifecycle)
  - `wisp-types` (shared notification/event types)

## Current status

Early-stage but functional:

- Implements `Notify`, `CloseNotification`, `GetCapabilities`, `GetServerInformation`
- Supports replacement (`replaces_id`), timeouts, actions, and D-Bus close/action signals
- Configurable popup layout/colors/format via TOML

## freedesktop.org Notifications API coverage

Checklist for `org.freedesktop.Notifications` support right now:

### Core D-Bus methods

- [x] `Notify`
- [x] `CloseNotification`
- [x] `GetCapabilities`
- [x] `GetServerInformation`

### D-Bus signals

- [x] `NotificationClosed`
- [x] `ActionInvoked`

### Behavior/details

- [x] Replacement via `replaces_id`
- [x] Action invocation from UI/debug path
- [x] Timeout handling (`> 0`, `0`, and `< 0` + configurable default timeout)
- [x] Basic hints parsing: `urgency`, `category`, `desktop-entry`, `transient`
- [~] Extra hints preserved as debug strings (not fully interpreted)
- [ ] Rich hints/attachments (images, sound, progress, etc.)
- [ ] Markup rendering
- [ ] Icon rendering in UI

## Requirements

- Linux Wayland session (for `wispd` UI)
- Session D-Bus
- No other daemon owning `org.freedesktop.Notifications` (e.g. stop `mako`/`dunst` first)

## Quick start

### 1) Run debug daemon (easiest first test)

```bash
cargo run -p wisp-debug
```

In another terminal:

```bash
notify-send "hello" "from notify-send"
```

### 2) Run UI daemon

```bash
cargo run -p wispd
```

If Wayland libraries are missing, use the flake dev shell:

```bash
nix develop
cargo run -p wispd
```

## Configuration

Config file path:

- `$XDG_CONFIG_HOME/wispd/config.toml`
- fallback: `~/.config/wispd/config.toml`

Example:

```toml
[source]
default_timeout_ms = 5000
capabilities = ["body", "actions"]

[ui]
format = "{app_name}: {summary}\n{body}"
max_visible = 5
width = 420
height = 64
gap = 8
padding = 10
font_size = 15
font_family = "sans-serif"
anchor = "top-right"

[ui.margin]
top = 16
right = 16
bottom = 16
left = 16

[ui.colors]
low = "#6aa9ff"
normal = "#7dcf7d"
critical = "#ff6b6b"
background = "#1e1e2ecc"
text = "#f8f8f2"
```

## Development

```bash
cargo fmt
cargo clippy --workspace --all-targets
cargo test --workspace
```

See [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) for implementation details and current behavior.