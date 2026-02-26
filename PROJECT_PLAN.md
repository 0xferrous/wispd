# wispd plan

## Goal
Build a Rust notification daemon ecosystem with:
1. a reusable library for receiving/normalizing notification events
2. one binary (`wispd`) that renders notifications on Wayland
3. a debug consumer (`wisp-debug`) that logs events

## Protocol (v1)
Use `org.freedesktop.Notifications` on D-Bus.
No portal integration.

## Workspace (proposed)
```text
wispd/
  Cargo.toml
  crates/
    wisp-types/        # shared models/events
    wisp-source/       # D-Bus service + store/lifecycle + event stream
  bins/
    wispd/             # main daemon + UI
    wispd-ui/          # optional future split frontend
    wisp-debug/        # debug consumer: log incoming notifications
```

## Locked decisions
1. **Process model**: single binary in v1 (`wispd`), with reusable logic in library.
2. **Library ownership**: `wisp-source` owns DBus methods/signals, state store, IDs, replace/timeout lifecycle, and event emission.
3. **Event distribution**: `tokio::mpsc` for v1.
4. **Replacement semantics**:
   - `replaces_id == 0` => create new ID
   - existing `replaces_id` => replace in place, keep same ID, reset timeout
   - missing `replaces_id` => create new ID
5. **Spec surface**: implement `Notify`, `CloseNotification`, `GetCapabilities`, `GetServerInformation`, and signals `NotificationClosed`, `ActionInvoked`.
6. **Capabilities**: set via constructor/config by caller (not hardcoded).
7. **Types/hints**: typed known hints + raw extra hints map.
8. **UI contract**: UI gets event stream + snapshot/query API.
   - defaults: max visible 5, newest on top, replacement keeps slot.
9. **Config v1**: TOML at `$XDG_CONFIG_HOME/wispd/config.toml` (fallback `~/.config/wispd/config.toml`).
   - no rule engine in v1.
10. **Errors/logging**: `thiserror` in libs, `anyhow` at binaries, `tracing` structured logs.

## Event API sketch
```rust
pub enum NotificationEvent {
    Received(Notification),
    Closed { id: u32, reason: CloseReason },
    ActionInvoked { id: u32, action_key: String },
    Replaced { old_id: u32, new: Notification },
}
```

## Milestones
- [ ] **M0 setup**: workspace + deps + fmt/clippy/tracing
- [ ] **M1 library MVP**: ingest notifications, normalize, emit events, replace-id/timeout basics
- [ ] **M2 daemon MVP**: own `org.freedesktop.Notifications`, implement core methods, state + expiration
- [ ] **M3 Wayland UI**: popup rendering, stacking, urgency styles, action handling
- [ ] **M4 debug consumer**: `wisp-debug` CLI with human and `--json` output

## Test plan (v1)
- [ ] integration test: send a notification (notify-send equivalent via DBus call) and assert daemon receives it
- [ ] replacement semantics test
- [ ] timeout expiration test
- [ ] `CloseNotification` behavior test
- [ ] `wisp-debug` output test (human + json)
- [ ] `cargo fmt`
- [ ] `cargo clippy`
