# wispd reliability / stability / durability task list

This is an implementation-oriented task list focused on improving `wispd` reliability, stability, and operational durability.

It intentionally avoids broad feature-parity work that does not materially improve current robustness.

## Prioritization legend

- **P0**: high value, should reduce regressions or user-visible breakage now
- **P1**: strong value, should follow after P0
- **P2**: useful hardening or future-facing groundwork

## Guiding principle

Prefer work that improves one or more of:

- correctness of notification lifecycle
- predictable popup behavior under reload/output changes
- test coverage for current behavior
- recovery from compositor/session-bus/environment edge cases
- durable notification state/history

---

## P0 — highest-value tasks

### 1. Fill the current `wisp-source` behavior test gaps

**Why**
These are core daemon invariants. They are cheap relative to their value and directly protect existing behavior.

**Tasks**

- [x] Add test: `replaces_id == 0` allocates a fresh ID and emits `Received`
- [x] Add test: missing `replaces_id` allocates a fresh ID rather than mutating another entry
- [x] Add test: replacement resets effective timeout
- [x] Add test: negative timeout uses `default_timeout_ms`
- [x] Add test: negative timeout with no configured default remains persistent
- [x] Add test: `expire_timeout == 0` never schedules expiry
- [x] Add test: known hints (`urgency`, `category`, `desktop-entry`, `transient`) are parsed exactly
- [x] Add test: unknown hints are preserved without breaking delivery
- [x] Add test: notification snapshot reflects replacement and close state correctly
- [x] Add test: closing unknown/nonexistent ID is a safe no-op
- [x] Add test: duplicate action keys or empty action lists are handled safely

**Likely area**
- `crates/wisp-source`

**Value**
Protects core protocol correctness and lifecycle behavior.

---

### 2. Add queue-policy tests for `wispd`

**Why**
The frontend’s live popup behavior is central to the user experience and easy to regress during refactors.

**Tasks**

- [x] Add test: newest notifications appear at the top
- [x] Add test: replacement keeps the same visible slot instead of jumping position
- [x] Add test: closing/removal compacts the visible stack correctly
- [x] Add test: `max_visible` truncates only the visible popup set, not source state
- [x] Add test: popup order remains stable across a burst of notifications
- [x] Add test: replacing a currently hidden notification updates it without corrupting visible ordering

**Likely area**
- `bins/wispd`

**Value**
Protects day-to-day UI behavior and avoids “weird popup ordering” regressions.

---

### 3. Add multi-output/window lifecycle regression tests

**Why**
The architecture doc already identifies focused-output behavior as a known bug area. This is one of the highest-risk stability areas in the UI.

**Tasks**

- [x] Add test: sticky stack-output state resets when the popup stack becomes empty
- [x] Add test: sticky stack-output state resets when compositor-closes all visible windows
- [x] Add test: output removal triggers visible popup rebuild only when current stack binding is affected
- [x] Add test: visible notifications recover cleanly after output disappearance
- [x] Add test: `output = "focused"` + no existing stack chooses output according to current policy
- [x] Add test: later notifications stick to the active stack output while stack is visible
- [x] Add test: config reload does not strand windows on stale outputs

**Likely area**
- `bins/wispd`
- `docs/ARCHITECTURE.md` should stay aligned with actual behavior

**Value**
Targets the most explicitly known reliability issue.

---

### 4. Add config reload regression coverage

**Why**
Runtime reload is a sharp edge: it touches both source and UI settings without restarting D-Bus ownership.

**Tasks**

- [ ] Add test: `SIGHUP` reload updates UI config without losing active notifications
- [x] Add test: source settings (`capabilities`, `default_timeout_ms`) update in place on reload
- [x] Add test: invalid config reload fails safely and leaves running state unchanged
- [x] Add test: reload while notifications are visible preserves sane popup state/order
- [x] Add test: reload while timers are active does not duplicate or drop expiry handling

**Likely area**
- `bins/wispd`
- `crates/wisp-source`

**Value**
Prevents hard-to-debug runtime inconsistencies.

---

### 5. Strengthen end-to-end D-Bus integration tests

**Why**
Unit tests help, but daemon reliability depends on actual D-Bus method/signal behavior.

**Tasks**

- [x] Add integration test: rapid burst of multiple `Notify` calls preserves ordering and IDs
- [x] Add integration test: replace storm on same ID leaves one final live notification
- [x] Add integration test: `CloseNotification` during active timeout race produces one final close event/signal only
- [x] Add integration test: action invocation on replaced notification targets current generation correctly
- [x] Add integration test: capabilities and server info stay correct after reload

**Likely area**
- `crates/wisp-source` integration tests

**Value**
Catches race-ish behavior and API regressions that unit tests miss.

---

## P1 — strong follow-up work

### 6. Introduce durable notification history (#1)

**Why**
This is the main “durability” feature: without it, notifications disappear after leaving the live stack.

**Tasks**

- [ ] Define history model in `wisp-types`
- [ ] Record every received notification into history in `wisp-source`
- [ ] Preserve replacement chains/generation lineage in history
- [ ] Track terminal state for history entries (expired, dismissed, closed-by-call, action-invoked, replaced)
- [ ] Add retention policy config:
  - [ ] max entries
  - [ ] max age
  - [ ] optional persistence toggle
- [ ] Add pruning tests for retention limits
- [ ] Add persistence round-trip tests if disk persistence is added

**Likely area**
- `crates/wisp-types`
- `crates/wisp-source`

**Value**
Improves durability and makes debugging/user trust much better.

---

### 7. Add daemon control API + `wispctl` foundation (#3)

**Why**
A control surface improves observability and scripted recovery, and will make future reliability testing easier.

**Tasks**

- [ ] Define minimal control API for:
  - [ ] list live notifications
  - [ ] dismiss notification(s)
  - [ ] invoke action
  - [ ] reload config
  - [ ] query history (once #1 exists)
- [ ] Implement stable machine-readable output semantics
- [ ] Add integration tests for control actions against running daemon
- [ ] Add CLI tests for exit codes and JSON output shape

**Likely area**
- `bins/wispctl` (new)
- control API in source/daemon layer

**Value**
Improves operability and supports scripted testing and debugging.

---

### 8. Add daemon-level DND / focus mode as presentation policy (#4)

**Why**
This is a reliability-of-behavior feature: it separates delivery from presentation and prevents lost notifications during suppression.

**Tasks**

- [ ] Introduce DND state model separate from notification delivery
- [ ] Suppress popup creation while still accepting/storing notifications
- [ ] Define and test bypass policy for critical urgency
- [ ] Add runtime controls: on/off/toggle/status
- [ ] Add tests that suppressed notifications are not replayed as a popup burst when DND is disabled
- [ ] If history exists, assert suppressed notifications remain present in history

**Likely area**
- `bins/wispd`
- control API / `wispctl`
- `crates/wisp-source` if policy state needs source awareness

**Value**
Improves behavior predictability without dropping notification data.

---

### 9. Add frontend interaction reliability once click handling lands

**Why**
Input behavior is user-facing and easy to get subtly wrong.

**Tasks**

- [ ] Implement click-to-dismiss in `wispd`
- [ ] Add test: left/right click actions dispatch correctly
- [ ] Add test: invoke-default-action path only fires once
- [ ] Add test: click on one popup does not affect adjacent popups
- [ ] Add test: hidden/closing popup cannot be double-click-raced into duplicate actions

**Likely area**
- `bins/wispd`

**Value**
Improves correctness under real interaction.

---

## P2 — useful hardening

### 10. Improve formatter robustness

**Why**
Formatting bugs become rendering bugs and can destabilize the popup layer.

**Tasks**

- [ ] Add tests for all documented placeholders: `{id}`, `{app_name}`, `{summary}`, `{body}`, `{urgency}`
- [ ] Add tests for missing/empty fields
- [ ] Add tests for very long body/summary strings
- [ ] Add tests for multiline body formatting
- [ ] Add tests for malformed/unexpected placeholder syntax

**Likely area**
- `bins/wispd`

**Value**
Reduces text/render regressions.

---

### 11. Stress and soak testing

**Why**
Some failures only show up under load or over time.

**Tasks**

- [ ] Add a test/dev tool that sends bursts of notifications with mixed timeouts and replacements
- [ ] Add a long-running soak scenario: repeated notify/replace/close cycles
- [ ] Add a scenario with repeated output changes while notifications are active
- [ ] Add a scenario that repeatedly reloads config while traffic is active
- [ ] Record memory growth / state growth expectations during soak runs

**Likely area**
- test harness / dev scripts
- integration environment

**Value**
Finds race conditions and resource leaks earlier.

---

### 12. Improve startup and failure-path diagnostics

**Why**
Operational stability includes failing predictably and understandably.

**Tasks**

- [ ] Add explicit tests for session-bus unavailable / name ownership failure cases where practical
- [ ] Ensure startup errors clearly distinguish:
  - [ ] bus name already taken
  - [ ] no Wayland runtime
  - [ ] bad config file
  - [ ] unsupported output selection situation
- [ ] Audit logs so close/action/reload/output-reset paths are visible in debug logs

**Likely area**
- `bins/wispd`
- `crates/wisp-source`

**Value**
Improves debuggability and field reliability.

---

## What not to prioritize yet

These may be interesting, but they are not the best near-term reliability investments:

- full mako-style criteria/rule engine
- live popup grouping parity with mako
- icon-theme resolution parity with mako
- deep styling parity work unrelated to current bugs
- a mako-compatible private IPC surface

These are feature-parity efforts, not the best current reliability multipliers.

---

## Recommended execution order

### Phase 1: lock down current behavior

1. `wisp-source` test gaps
2. `wispd` queue-policy tests
3. multi-output/window lifecycle tests
4. reload regression tests
5. stronger D-Bus integration tests

### Phase 2: add durability and operability

6. notification history (#1)
7. control API + `wispctl` (#3)
8. DND/focus mode (#4)

### Phase 3: harden for real-world use

9. click/input reliability
10. formatter robustness
11. stress/soak testing
12. startup/failure-path diagnostics

---

## Smallest high-value next step

If only one short burst of work is possible, do this first:

- [ ] add the missing `wisp-source` timeout/replacement/hint tests
- [ ] add one `wispd` queue-order test
- [ ] add one multi-output reset/regression test
- [ ] run `cargo fmt`
- [ ] run `cargo clippy`

That gives the best immediate reliability return for the least implementation churn.
