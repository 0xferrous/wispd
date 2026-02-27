# wispd

`wispd` is an experimental Wayland notification daemon for `org.freedesktop.Notifications`.
It includes:

- **`wispd`**: layer-shell popup UI (one popup window per notification)
- **`wisp-debug`**: CLI/debug daemon to inspect incoming notifications and test close/action flows
- **`wispd-monitor`**: passive D-Bus monitor for notifications traffic (does not own `org.freedesktop.Notifications`)
- **`wispd-forward`**: forwards host notifications into a VM over SSH (keeps host daemon like `mako` active)
- Reusable crates:
  - `wisp-source` (D-Bus server + notification lifecycle)
  - `wisp-types` (shared notification/event types)

## Current status

Early-stage but functional:

- Implements `Notify`, `CloseNotification`, `GetCapabilities`, `GetServerInformation`
- Supports replacement (`replaces_id`), timeouts, actions, and D-Bus close/action signals
- Configurable popup layout/colors/format via TOML
- Optional timeout progress bar (top/bottom edge) for timed notifications

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

### 3) Run passive monitor (no name ownership)

```bash
cargo run -p wispd-monitor
```

Or via flake app:

```bash
nix run .#wispd-monitor
```

### 4) Forward host notifications into VM (while keeping host mako)

```bash
# defaults: wisp@127.0.0.1:2222 and remote notify-send
cargo run -p wispd-forward

# or
nix run .#wispd-forward
```

Useful env vars:

- `WISPD_FORWARD_SSH_HOST` (default: `127.0.0.1`)
- `WISPD_FORWARD_SSH_PORT` (default: `2222`)
- `WISPD_FORWARD_SSH_USER` (default: `wisp`)
- `WISPD_FORWARD_SSH_PASSWORD` (default: `wisp`)
- `WISPD_FORWARD_NOTIFY_SEND` (default: `notify-send`)
- `WISPD_FORWARD_SSH_STARTUP_WAIT_SECS` (default: `60`)
- `WISPD_FORWARD_SSH_STARTUP_POLL_MS` (default: `500`)

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
show_timeout_progress = true
timeout_progress_height = 3
timeout_progress_position = "bottom"

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
timeout_progress = "#f8f8f2"
```

## Niri + wispd MicroVM (QEMU)

A ready-to-run MicroVM configuration is included via `github:microvm-nix/microvm.nix`.

What it configures:

- QEMU MicroVM with graphics enabled
- Niri compositor started at boot via `greetd`
- `wispd` started as a `systemd --user` service from `/work/wispd/target/debug/wispd`
- host workspace is shared into the guest at `/work/wispd` via a relative `9p` share (`source = "."`)
- `alacritty` installed (and exported as `TERMINAL`)
- SSH enabled in guest, forwarded as host `127.0.0.1:2222 -> guest:22` (for `wispd-forward`)

Run it:

```bash
nix run .#wispd-microvm
```

Inside the VM:

- user: `wisp`
- graphical login: passwordless (auto-login via `greetd`)
- SSH password (for `wispd-forward`): `wisp`

Test notifications in `alacritty`:

```bash
notify-send "hello" "from wispd microvm"
```

Hot-reload-ish dev loop (no VM reboot):

```bash
# host
cargo build -p wispd

# guest
systemctl --user restart wispd
```

## Development

Rust checks:

```bash
cargo fmt
cargo clippy --workspace --all-targets
cargo test --workspace
```

Nix package build (uses `ipetkov/crane` for faster incremental dependency reuse):

```bash
nix build .#wispd
```

MicroVM development / validation:

```bash
# evaluate exposed outputs
nix flake show

# dry-run build of the runnable qemu microvm package
nix build .#wispd-microvm --dry-run

# run the VM
nix run .#wispd-microvm
```

MicroVM config source:

- `flake.nix` (inputs + `nixosConfigurations.wispd-microvm` + app/package outputs)
- `nix/microvm/wispd-microvm.nix` (Niri/greetd/wispd/alacritty VM config)

Dev convenience setup:

- `wispd-forward` defaults are aligned with the bundled microvm (`127.0.0.1:2222`, user `wisp`, password `wisp`).
- Forwarder startup polls until VM SSH is reachable, so it can be started before or during VM boot.

See [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) for implementation details and current behavior.