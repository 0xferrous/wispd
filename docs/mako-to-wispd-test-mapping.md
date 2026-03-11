# Mako-derived test mapping for wispd

This table filters the broader `docs/mako-test-cases.md` inventory down to what is useful for `wispd`.

Status meanings:

- **Relevant now**: should map directly to current `wispd`/`wisp-source` behavior
- **Adapt**: useful idea, but needs rewriting for `wispd`'s Rust/TOML/iced architecture
- **Future**: likely useful later, but not a good current test target
- **Skip**: mako-specific and not relevant to current `wispd`

| Mako area | IDs | wispd status | Notes / wispd equivalent |
|---|---|---:|---|
| Primitive parsing: booleans/ints/colors/directionals/anchors | `TYP-01`..`TYP-20`, `TYP-24`, `TYP-25` | Skip | These cover mako's C config/parser types, not `wispd`'s TOML config model. |
| Format parsing (`%`-style) | `TYP-21`..`TYP-23` | Adapt | Only the intent carries over. Rewrite around `wispd` placeholders like `{id}`, `{summary}`, `{body}`, `{urgency}`. |
| Config defaults/style merging/include logic | `CFG-01`..`CFG-22` | Skip | Mako criteria/style engine does not exist in `wispd`. |
| Criteria syntax/regex/validation | `CFG-23`..`CFG-33` | Skip | `wispd` has no mako-style criteria sections, regex matching, or `group-by` validation. |
| Criteria matching and style application | `CRT-01`..`CRT-10` | Skip | Same reason: mako-specific criteria pipeline. |
| Grouping behavior | `CRT-11`..`CRT-14` | Skip | `wispd` currently has queueing/max-visible/replacement, not mako-style grouping. |
| Sorting behavior | `CRT-15`..`CRT-17` | Adapt | Relevant as queue-policy checks: newest on top, stable replace-in-place behavior, max-visible ordering. |
| Notification id allocation | `NTF-01` | Relevant now | Maps directly to `wisp-source` ID allocation semantics. |
| Notification reset/low-level memory cleanup | `NTF-02` | Skip | C implementation detail, not meaningful as-is in Rust. |
| Close/history behavior | `NTF-03`..`NTF-06` | Adapt | Relevant if/when `wispd` grows a history buffer. Today only close/expiry semantics matter directly. |
| Regroup on close / close group | `NTF-07`, `NTF-08` | Skip | Mako grouping only. |
| Close-all safety | `NTF-09` | Adapt | Good general store robustness test: bulk close/removal should be safe. |
| Lookup by id | `NTF-10` | Relevant now | Maps to source/store lookup behavior. |
| Lookup by tag | `NTF-11` | Future | Only relevant if `wisp-source` adopts canonical/dunst stack-tag replacement semantics. |
| Formatting/escaping/markup | `NTF-12`..`NTF-15` | Adapt | Useful for `wispd` formatter/render text behavior, but rewrite for your formatting system. |
| Button binding dispatch | `NTF-16`, `NTF-17` | Future | Relevant once click handling is fully implemented in `wispd`. |
| Action invocation and unknown actions | `NTF-18`, `NTF-19` | Relevant now | Already matches `wisp-source` action API and signal behavior. |
| Exec binding / dismiss-no-history | `NTF-20`, `NTF-21` | Skip | Mako binding system does not exist in `wispd`. |
| XDG capabilities reporting | `XDG-01`..`XDG-03` | Relevant now | Maps directly to `GetCapabilities`; capability contents come from `wispd` config rather than mako superstyle. |
| Basic `Notify` field copying | `XDG-04` | Relevant now | Core server behavior for `wisp-source`. |
| Replace-by-id semantics | `XDG-05`, `XDG-06` | Relevant now | Direct match for current replace semantics. |
| Action list parsing | `XDG-07` | Relevant now | Direct match for parsed notification actions. |
| Urgency/core hint parsing | `XDG-08`..`XDG-11` | Relevant now | Matches implemented hint parsing in `wisp-source`. |
| Image-path / image-data / icon hints | `XDG-12`, `XDG-14` | Future | Architecture doc says richer hint coverage is not implemented yet. |
| Tag hints / tag replacement | `XDG-13`, `XDG-20` | Future | Useful if you add stack-tag semantics; not core today unless already implemented in code. |
| Unknown hint handling | `XDG-15` | Relevant now | Maps directly to preserving/ignoring unknown hints safely. |
| Timeout handling | `XDG-16`..`XDG-18` | Relevant now | Direct match for requested timeout / default timeout / persistent behavior. |
| Icon creation gating | `XDG-19` | Future | Relevant once richer icon/image handling is implemented. |
| Notify-time side effects / grouping | `XDG-21`, `XDG-22` | Skip | Grouping and notify bindings are mako-specific. |
| `CloseNotification` API | `XDG-23` | Relevant now | Direct match for close-by-call behavior and signal emission. |
| `GetServerInformation` | `XDG-24` | Relevant now | Direct match. |
| `ActionInvoked` gating / activation token emission | `XDG-25`, `XDG-26` | Adapt | Signal behavior relevant; xdg-activation token behavior depends on whether/when `wispd` supports equivalent click-action focus semantics. |
| Mako private D-Bus API | `MKO-01`..`MKO-14` | Skip | `wispd` does not implement `fr.emersion.Mako` runtime control API. |
| `makoctl` CLI behavior | `MKO-15`..`MKO-26` | Skip | No `makoctl` equivalent in `wispd` today. |
| Empty render / visibility / size/layout | `RND-01`, `RND-02`, `RND-07`..`RND-10`, `RND-13`, `RND-15` | Adapt | Good frontend/UI tests for iced popup sizing, visibility limits, progress bar, and click region behavior. |
| Group-aware visibility and hidden placeholder | `RND-03`..`RND-05` | Skip | Hidden placeholder/group semantics are mako-specific. |
| Render-time second-pass criteria matching | `RND-06` | Skip | No equivalent criteria system in `wispd`. |
| Radius/border/operator details | `RND-11`, `RND-12`, `RND-14`, `RND-16` | Adapt | These become visual/layout regression tests only if your renderer exposes equivalent styling features. |
| Icon resolution and themed icon search | `ICO-01`..`ICO-12` | Future | Useful once `wispd` implements richer icon support; not primary current coverage. |
| Wayland global init/output bookkeeping | `WLD-01`..`WLD-04` | Adapt | Relevant as frontend integration tests, but rewrite for iced/layer-shell abstractions rather than mako internals. |
| Pointer/touch hit testing | `WLD-05`..`WLD-07` | Future | Good once click interaction is fully in place. |
| Cursor handling | `WLD-08`, `WLD-09` | Skip | Mako-specific low-level cursor management; likely not important for `wispd`. |
| Input region shaping | `WLD-10` | Adapt | Relevant if you need click-through/click-region correctness for popup windows. |
| Surface teardown / relocation / recreation | `WLD-11`..`WLD-17` | Relevant now | Very relevant to `wispd` multi-window lifecycle, output changes, compositor-closed windows, and reflow. |
| Activation token creation | `WLD-18`, `WLD-19` | Future | Only if `wispd` adds equivalent focus-token support for action invocation. |
| Event-loop/timer ordering | `EVT-01`..`EVT-04` | Adapt | Relevant in spirit for timeout scheduling and shutdown behavior, but rewrite for Tokio/runtime abstractions. |

## Recommended wispd-focused subset

These are the mako-derived areas worth actively translating into `wispd` tests now:

### Source / D-Bus

- `NTF-01`
- `NTF-10`
- `NTF-18`
- `NTF-19`
- `XDG-01`..`XDG-11`
- `XDG-15`..`XDG-18`
- `XDG-23`
- `XDG-24`

### Queue / frontend behavior

- `CRT-15`..`CRT-17` adapted into queue policy tests
- `RND-01`
- `RND-02`
- `RND-07`
- `RND-08`
- `RND-09`
- `RND-13`
- `RND-15`

### Output / window lifecycle

- `WLD-11`..`WLD-17`

## Immediate Rust-native equivalents to write

| New wispd test theme | Derived from |
|---|---|
| notify creates notification event with parsed summary/body/icon/actions | `XDG-04`, `XDG-07` |
| replacing existing id keeps same id and updates in place | `XDG-05`, `XDG-06` |
| `CloseNotification` emits `ClosedByCall` and D-Bus `NotificationClosed` | `XDG-23` |
| negative timeout uses configured default timeout | `XDG-16` |
| zero timeout produces persistent notification | `XDG-18` |
| unknown hint is preserved/ignored without failure | `XDG-15` |
| invoking known action emits `ActionInvoked` and closes notification if that is your chosen policy | `NTF-18`, `XDG-26` |
| invoking unknown action is a no-op / returns false | `NTF-19` |
| newest notification is shown at top | `CRT-15` |
| replacement keeps visible slot rather than moving item | adapted `CRT-*` sorting semantics |
| `max_visible` truncates visible popup set | `RND-02` |
| progress bar uses effective timeout, not raw requested timeout, when defaulting is in effect | `XDG-16`, `RND-13` |
| popup stack resets output affinity when stack becomes empty | `WLD-12`..`WLD-17` |
| output removal/compositor close does not leave stuck invisible windows | `WLD-15`..`WLD-17` |
