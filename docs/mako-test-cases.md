# Mako source exploration: candidate test cases

This document is a source-driven test inventory for the upstream `mako` notification daemon, based on reading:

- `../open-source/mako/config.c`
- `../open-source/mako/criteria.c`
- `../open-source/mako/types.c`
- `../open-source/mako/notification.c`
- `../open-source/mako/dbus/xdg.c`
- `../open-source/mako/dbus/mako.c`
- `../open-source/mako/wayland.c`
- `../open-source/mako/render.c`
- `../open-source/mako/icon.c`
- `../open-source/mako/event-loop.c`
- `../open-source/mako/mode.c`
- `../open-source/mako/makoctl.c`
- plus README/manpages in `../open-source/mako/doc/`

It is not a list of existing upstream tests. It is a proposed test plan extracted from implementation behavior, edge cases, and invariants.

## Suggested test strategy

Split coverage into four layers:

1. **Pure/unit-ish parsing tests**
   - `types.c`
   - `config.c`
   - `criteria.c`
   - `mode.c`
2. **Stateful daemon logic tests**
   - `notification.c`
   - `dbus/xdg.c`
   - `dbus/mako.c`
3. **Rendering/layout tests**
   - `render.c`
   - `icon.c`
4. **Wayland/integration tests**
   - `wayland.c`
   - `event-loop.c`
   - `makoctl.c`

## Coverage notes from the source

High-risk areas worth prioritizing first:

- config parsing and validation rules
- grouping behavior and regrouping after close/reload
- hint parsing in `Notify`
- timeout/history interactions
- hidden placeholder rendering and `max-visible`
- mode changes + config reapplication
- icon resolution precedence and scaling
- `makoctl` command argument validation
- Wayland surface recreation when output/size changes

---

## 1. Primitive parsing (`types.c`)

| ID | Area | Intent | Stimulus | Expected result | Source |
|---|---|---|---|---|---|
| TYP-01 | boolean parsing | Accept canonical true values | `true`, `TRUE`, `1` | parsed as `true` | `parse_boolean` |
| TYP-02 | boolean parsing | Accept canonical false values | `false`, `FALSE`, `0` | parsed as `false` | `parse_boolean` |
| TYP-03 | boolean parsing | Reject invalid boolean tokens | `yes`, `on`, empty string | parse failure | `parse_boolean` |
| TYP-04 | int parsing | Accept valid decimal integer | `42`, `-1`, `0` | parse success | `parse_int` |
| TYP-05 | int parsing | Reject trailing junk | `12px`, `4 ` | parse failure | `parse_int` |
| TYP-06 | int lower bound | Enforce non-negative or positive minimums | values below `min` for width/height/border/etc. | parse failure | `parse_int_ge` |
| TYP-07 | color parsing | Accept `#RRGGBB` and append alpha `FF` | `#112233` | output `0x112233FF` | `parse_color` |
| TYP-08 | color parsing | Accept `#RRGGBBAA` | `#11223344` | exact value preserved | `parse_color` |
| TYP-09 | color parsing | Reject malformed color syntax | missing `#`, wrong length, non-hex digits | parse failure | `parse_color` |
| TYP-10 | progress color | Default operator to `over` when omitted | `#11223344` | operator=`OVER`, value parsed | `parse_mako_color` |
| TYP-11 | progress color | Accept explicit `over` / `source` prefix | `over #...`, `source #...` | correct operator selected | `parse_mako_color` |
| TYP-12 | progress color | Reject operator without color | `over` | parse failure | `parse_mako_color` |
| TYP-13 | urgency parsing | Accept `low`, `normal`, `critical`, alias `high` | each token | mapped enum value | `parse_urgency` |
| TYP-14 | directional parsing | 1-value CSS shorthand | `5` | all edges = 5 | `parse_directional` |
| TYP-15 | directional parsing | 2-value shorthand | `5,10` | top/bottom=5, left/right=10 | `parse_directional` |
| TYP-16 | directional parsing | 3-value shorthand | `5,10,15` | top=5, left/right=10, bottom=15 | `parse_directional` |
| TYP-17 | directional parsing | 4-value shorthand | `1,2,3,4` | top/right/bottom/left match | `parse_directional` |
| TYP-18 | directional parsing | Reject non-integer component | `1,a,3,4` | parse failure | `parse_directional` |
| TYP-19 | criteria spec parsing | Accept supported `group-by` fields | e.g. `app-name,summary,urgency` | requested bits set | `parse_criteria_spec` |
| TYP-20 | criteria spec parsing | Reject unknown `group-by` field | `foo` | parse failure | `parse_criteria_spec` |
| TYP-21 | format parsing | Accept valid format specifiers | `%a %s %b %g %h %t %i %%` | parse success | `parse_format` |
| TYP-22 | format parsing | Preserve `\n` and `\\` escapes | format with escapes | newline/backslash emitted | `parse_format` |
| TYP-23 | format parsing | Reject invalid format specifier | `%q` | parse failure | `parse_format` |
| TYP-24 | anchor parsing | Accept all documented anchor values | each anchor string | expected bitmask | `parse_anchor` |
| TYP-25 | anchor parsing | Reject invalid anchor name | `middle-right` | parse failure | `parse_anchor` |

---

## 2. Config parsing and validation (`config.c`, `criteria.c`)

| ID | Area | Intent | Stimulus | Expected result | Source |
|---|---|---|---|---|---|
| CFG-01 | default config | Built-in criteria list is initialized correctly | call `init_default_config` | root + grouped + group-index=0 + hidden criteria exist | `init_default_config` |
| CFG-02 | default style | Defaults match docs | inspect initialized style | width/height/colors/actions/history/etc. equal defaults | `init_default_style` |
| CFG-03 | style application | Applying partial style only overwrites specified fields | `apply_style` with subset spec | unspecified fields unchanged | `apply_style` |
| CFG-04 | style application | String fields are deep-copied | apply style with font/format/output | target owns independent copies | `apply_style` |
| CFG-05 | superset style | Capability superset includes max widths/heights and unioned format specifiers | build config with multiple criteria | computed `superstyle` is max/union form | `apply_superset_style` |
| CFG-06 | config include path | Absolute path include works | `include=/abs/path` | included file loaded | `expand_config_path`, `load_config_file` |
| CFG-07 | config include path | `~/...` include expands via `HOME` | `include=~/cfg` | home-expanded path loaded | `expand_config_path` |
| CFG-08 | config include path | Relative include is rejected | `include=foo/bar` | parse failure | `expand_config_path` |
| CFG-09 | config file parsing | Leading/trailing whitespace is ignored | spaced `key=value` lines | option still applies | `load_config_file` |
| CFG-10 | config file parsing | Comments and empty lines are ignored | blank/comment lines | no errors, no changes | `load_config_file` |
| CFG-11 | config file parsing | Non-`key=value` line fails | malformed line | parse failure with line number | `load_config_file` |
| CFG-12 | config file parsing | Criteria section parsing starts/stops correctly | config with multiple sections | per-section styles isolated | `load_config_file` |
| CFG-13 | style option parsing | `padding` is clamped up to border radius when both specified | border radius > padding | left/right padding raised to radius | `apply_style_option` |
| CFG-14 | style option parsing | Same clamp works when option order is reversed | set padding then border-radius, and reverse | consistent final padding | `apply_style_option` |
| CFG-15 | style option parsing | `icons` is accepted or ignored cleanly when icon support is not compiled | set `icons=1` without icon build | no parse failure, warning only | `apply_style_option` |
| CFG-16 | style option parsing | `icon-location` accepts all valid values | left/right/top/bottom | parse success | `apply_style_option` |
| CFG-17 | style option parsing | `layer` accepts all valid values | background/bottom/top/overlay | parse success | `apply_style_option` |
| CFG-18 | style option parsing | binding action parsing supports all documented actions | dismiss, no-history, dismiss-all, dismiss-group, invoke-default-action, invoke-action, exec | expected binding enum and payload | `apply_style_option` |
| CFG-19 | global option parsing | repeated `sort=` lines accumulate criteria flags | `+time` then `-priority` | both sort modes reflected in config bitmasks | `apply_config_option` |
| CFG-20 | config discovery | default path prefers existing `~/.mako/config` then `$XDG_CONFIG_HOME/mako/config` | mock filesystem | first readable config found | `get_default_config_path` |
| CFG-21 | CLI override parsing | command-line global style options override file config | config + `--font=...` | CLI wins | `parse_config_arguments` |
| CFG-22 | reload behavior | invalid new config leaves current config intact | call `reload_config` with broken input | reload fails, old config preserved | `reload_config` |
| CFG-23 | criteria syntax | escaped spaces and quoted values round-trip | `[app-name="Google Chrome"]`, escaped forms | exact stored value | `parse_criteria` |
| CFG-24 | criteria syntax | unmatched quote is rejected | broken criteria string | parse failure | `parse_criteria` |
| CFG-25 | criteria syntax | trailing backslash is rejected | broken criteria string | parse failure | `parse_criteria` |
| CFG-26 | criteria booleans | bare boolean and negated bare boolean work | `[actionable]`, `[!actionable]` | true / false respectively | `apply_criteria_field` |
| CFG-27 | criteria regex | invalid regex in `summary~` / `body~` fails | malformed POSIX regex | parse failure | `apply_criteria_field` |
| CFG-28 | criteria validation | `summary` and `summary~` cannot coexist | one section containing both | invalid criteria | `validate_criteria` |
| CFG-29 | criteria validation | `body` and `body~` cannot coexist | one section containing both | invalid criteria | `validate_criteria` |
| CFG-30 | criteria validation | matching `grouped`/`group-index`/`output`/`anchor` cannot set `anchor`, `output`, or `group-by` | crafted section | invalid criteria | `validate_criteria` |
| CFG-31 | criteria validation | `max-visible` only allowed for `output` and/or `anchor` criteria | combine `max-visible` with app-name | invalid criteria | `validate_criteria` |
| CFG-32 | criteria validation | `hidden=true` only allowed with `output` and/or `anchor` | combine hidden + app-name | invalid criteria | `validate_criteria` |
| CFG-33 | criteria validation | `group-by` cannot use `group-index`, `grouped`, `anchor`, `output` | invalid `group-by` field set | invalid criteria | `validate_criteria` |

---

## 3. Criteria matching and grouping (`criteria.c`, `notification.c`)

| ID | Area | Intent | Stimulus | Expected result | Source |
|---|---|---|---|---|---|
| CRT-01 | criteria matching | root criteria with `none=false` matches generic notifications | default/global criteria | match succeeds | `match_criteria` |
| CRT-02 | criteria matching | `none` short-circuits all other fields | criteria spec includes `none` | no match | `match_criteria` |
| CRT-03 | criteria matching | actionable criteria reflects action list emptiness | notif with/without actions | correct match result | `match_criteria` |
| CRT-04 | criteria matching | expiring criteria reflects requested timeout != 0 | timeout 0 and non-zero | correct match result | `match_criteria` |
| CRT-05 | criteria matching | regex criteria evaluate summary/body | regex + matching/non-matching text | correct result | `match_criteria` |
| CRT-06 | criteria matching | second-pass anchor/output criteria require assigned surface/output | notif before/after surface assignment | only matches after assignment | `match_criteria` |
| CRT-07 | criteria matching | mode criteria depends on active mode set | enable/disable mode | criteria enters/leaves match set | `match_criteria`, `has_mode` |
| CRT-08 | style application order | later matching criteria win for overlapping style options | two matching sections define same field | last section takes precedence | `apply_each_criteria` + list order |
| CRT-09 | surface assignment | matching criteria reuses existing surface with same output/anchor/layer | second notif with same placement | same `mako_surface` reused | `apply_each_criteria` |
| CRT-10 | surface assignment | new surface is created for unseen output/anchor/layer tuple | notif style targets new tuple | new `mako_surface` created | `apply_each_criteria`, `create_surface` |
| CRT-11 | grouping | no `group-by` means notification stays ungrouped | `group_criteria_spec.none=true` | `group_index=-1` | default style + `group_notifications` |
| CRT-12 | grouping | two matching notifications become contiguous and indexed 0..n-1 | insert matching pair | both grouped with stable indices | `group_notifications` |
| CRT-13 | grouping | single matched notification is forced back to ungrouped | group candidate count=1 | `group_index=-1` | `group_notifications` |
| CRT-14 | grouping | grouped notification count is propagated for `%g` formatting | grouped set size N | each member has `group_count=N` | `group_notifications` |
| CRT-15 | sorting | default `-time` inserts newest at front | insert sequential notifications | order is newest-first | `insert_notification` |
| CRT-16 | sorting | `+time` inserts oldest first | ascending time config | order is oldest-first | `insert_notification` |
| CRT-17 | sorting | urgency sorting respects configured direction and time tiebreak offset | mixed urgency set | list order matches algorithm | `insert_notification`, `get_last_notif_by_urgency` |

---

## 4. Notification lifecycle and formatting (`notification.c`)

| ID | Area | Intent | Stimulus | Expected result | Source |
|---|---|---|---|---|---|
| NTF-01 | creation | new notifications get unique increasing ids | create several notifications | ids monotonically increase | `create_notification` |
| NTF-02 | reset | reset frees actions/timer/icon/image data and restores empty strings/default urgency/progress | populate notification then reset | state cleared safely | `reset_notification` |
| NTF-03 | close/history | non-history notifications are destroyed immediately on close | `style.history=false` | notification removed, not added to history | `close_notification` |
| NTF-04 | close/history | history disabled globally destroys on close | `max_history <= 0` | no history entry | `close_notification` |
| NTF-05 | close/history | close with history inserts into history list head | close expiring notif | latest closed becomes first in history | `close_notification` |
| NTF-06 | close/history | history buffer is capped and trims oldest entries | close `max_history+1` notifications | oldest history entry removed | `close_notification` |
| NTF-07 | regroup on close | closing a grouped notification causes remaining peers to regroup | close member of group | surviving peers get updated group indices/count | `close_notification`, `group_notifications` |
| NTF-08 | close-group | `close_group_notifications` closes all peers matching top notification's group spec | grouped notifications | whole group closes | `close_group_notifications` |
| NTF-09 | close-all | closing all notifications walks full list safely | many notifications present | all removed without iterator corruption | `close_all_notifications` |
| NTF-10 | lookup by id | `get_notification` returns exact id or NULL | existing/missing ids | correct pointer/null | `get_notification` |
| NTF-11 | lookup by tag | tag replacement is scoped by tag + app name | same tag different app names | only same-app notification matches | `get_tagged_notification` |
| NTF-12 | format escaping | non-markup substitution content is XML-escaped | body contains `<>&'"` | rendered string escaped | `escape_markup`, `format_text` |
| NTF-13 | format markup | valid Pango markup in body survives when markup enabled | body contains valid markup | markup preserved, not escaped | `format_notif_text`, `format_text` |
| NTF-14 | format fallback | invalid markup is escaped instead of passed through | malformed markup body | safe escaped text | `format_text` |
| NTF-15 | format trimming | final formatted text trims surrounding whitespace | format produces leading/trailing space/newline | trimmed output | `trim_space`, `format_text` |
| NTF-16 | bindings | left/right/middle button dispatch maps to configured binding | synthesize button presses | expected binding executed | `get_button_binding`, `notification_handle_button` |
| NTF-17 | bindings | release events do not trigger actions | send non-pressed pointer state | no binding executed | `notification_handle_button` |
| NTF-18 | invoke action | invoking named/default action emits action and then closes notification | action exists | client signal emitted and notif dismissed | `try_invoke_action` |
| NTF-19 | invoke action missing | missing action still closes notification | target action absent | no action signal, notif still dismissed | `try_invoke_action` |
| NTF-20 | exec binding | `exec` binding passes notification id via shell variable `$id` | binding `exec ...` | subprocess sees correct id | `notification_execute_binding` |
| NTF-21 | dismiss-no-history | binding variant skips history insertion | use `dismiss --no-history` | closed without history entry | `notification_execute_binding` |

---

## 5. Freedesktop notifications D-Bus API (`dbus/xdg.c`)

| ID | Area | Intent | Stimulus | Expected result | Source |
|---|---|---|---|---|---|
| XDG-01 | capabilities | `body` capability only advertised when superset format contains `%b` | config with/without body in formats | capability present/absent accordingly | `handle_get_capabilities` |
| XDG-02 | capabilities | `body-markup`, `actions`, `icon-static` reflect superset config booleans | toggle config values across criteria | capability list mirrors superstyle | `handle_get_capabilities` |
| XDG-03 | capabilities | proprietary sync/tag capabilities are always advertised | query capabilities | contains canonical/dunst tags | `handle_get_capabilities` |
| XDG-04 | notify basic | notification fields from `Notify` body are copied into state | send app name/icon/summary/body | notification contains exact values | `handle_notify` |
| XDG-05 | replace by id | valid `replaces_id` resets and reuses existing notification object | notify with existing id | same id updated, no duplicate inserted | `handle_notify` |
| XDG-06 | replace by id invalid | invalid `replaces_id` creates fresh notification | notify with missing id | new notification with new id | `handle_notify` |
| XDG-07 | action parsing | actions array is stored key/title pairs in order | send action list | linked action list created correctly | `handle_notify` |
| XDG-08 | urgency hints | urgency accepts variant types `u`, `y`, and `i` | send each variant form | urgency parsed successfully | `handle_notify` |
| XDG-09 | urgency hints | unsupported urgency variant type is rejected | send e.g. string variant | method fails | `handle_notify` |
| XDG-10 | standard hints | category and desktop-entry hints overwrite defaults | send both hints | notification fields updated | `handle_notify` |
| XDG-11 | progress hint | integer `value` hint sets progress | send `value=0/50/100` | progress stored exactly | `handle_notify` |
| XDG-12 | image-path precedence | `image-path` / deprecated `image_path` override `app_icon` | send both | icon path from hint wins | `handle_notify` |
| XDG-13 | tag hints | canonical/dunst tag is stored for replacement | send tag hint | tag retained on notification | `handle_notify` |
| XDG-14 | image data parsing | image-data/image_data/icon_data variants are accepted | send image tuple | image buffer stored | `handle_notify` |
| XDG-15 | unknown hint | unknown hints are skipped, not fatal | include unsupported hint | notification still succeeds | `handle_notify` |
| XDG-16 | timeout handling | negative timeout uses `default-timeout` | send `-1` and configured default | timer uses default timeout | `handle_notify` |
| XDG-17 | timeout handling | `ignore-timeout=true` forces requested timeout to default | send explicit timeout with ignore enabled | default timeout used | `handle_notify` |
| XDG-18 | timeout handling | timeout `0` schedules no timer | send non-expiring notification | `notif->timer == NULL` | `handle_notify` |
| XDG-19 | icon creation | icons are created only when style enables icons | style icons on/off | icon object created or omitted | `handle_notify` |
| XDG-20 | tag replacement | same tag + same app replaces prior notification and preserves id | send two tagged notifications | old notif removed, new one gets old id | `handle_notify` |
| XDG-21 | grouping after notify | new notification triggers grouping according to final style | send matching notifications | grouped state updated immediately | `handle_notify`, `group_notifications` |
| XDG-22 | notify binding | `on-notify` binding executes after notify processing | configure exec/dismiss action | side effect occurs after insertion/style/grouping | `handle_notify` |
| XDG-23 | close request API | `CloseNotification` removes an existing notification and dirties surface | call with valid id | notif closed with request reason | `handle_close_notification` |
| XDG-24 | server info | `GetServerInformation` returns fixed daemon/spec metadata | query method | fixed strings returned | `handle_get_server_information` |
| XDG-25 | action signal gating | `ActionInvoked` is suppressed when actions disabled in style | invoke action on actions-disabled notif | no action signal sent | `notify_action_invoked` |
| XDG-26 | activation token | activation token signal is emitted before action invoked when token exists | invoke action via pointer/touch context | `ActivationToken` then `ActionInvoked` | `notify_action_invoked` |

---

## 6. Mako private D-Bus API and runtime control (`dbus/mako.c`, `mode.c`, `makoctl.c`)

| ID | Area | Intent | Stimulus | Expected result | Source |
|---|---|---|---|---|---|
| MKO-01 | dismiss API | default dismiss closes first/selected notification | call without id and with id | correct notification closes | `handle_dismiss` |
| MKO-02 | dismiss API | `all` and `group` together are rejected | request both flags | method returns error | `handle_dismiss` |
| MKO-03 | dismiss API | `id` cannot be combined with `all` or `group` | request invalid mix | method returns error | `handle_dismiss` |
| MKO-04 | dismiss API | `history=false` prevents insertion into history | dismiss with history=false | closed notification not restorable | `handle_dismiss`, `close_notification` |
| MKO-05 | invoke API | private InvokeAction emits matching action if present | call with valid action key | action signal emitted | `handle_invoke_action` |
| MKO-06 | restore API | restore pops newest history item back to active notifications | history non-empty | most recent history notif restored | `handle_restore_action` |
| MKO-07 | restore API | restore on empty history is a no-op | empty history | success reply, no state change | `handle_restore_action` |
| MKO-08 | list API | notification listing exposes expected fields and actions map | call list/history | serialized fields match state | `handle_list_for_each` |
| MKO-09 | reload API | successful reload destroys/recreates surfaces and reapplies styles | change config then reload | notifications survive with updated surfaces/styles | `reapply_config`, `handle_reload` |
| MKO-10 | reload API | invalid reload returns named D-Bus error | broken config | `fr.emersion.Mako.InvalidConfig` | `handle_reload` |
| MKO-11 | modes | `set_modes` drops duplicate entries | set duplicated modes | unique list stored | `set_modes` |
| MKO-12 | modes | `mode` criteria react after `SetMode`/`SetModes` | toggle mode | matching styles appear/disappear | `handle_set_mode`, `handle_set_modes`, `reapply_config` |
| MKO-13 | properties/signals | mode changes emit `Modes` property invalidation | change modes | signal emitted | `emit_modes_changed` |
| MKO-14 | properties/signals | notification changes emit `Notifications` property invalidation | add/close notification | signal emitted | `emit_notifications_changed` |
| MKO-15 | makoctl dismiss | CLI rejects `-a` + `-g` | `makoctl dismiss -a -g` | non-zero exit, message | `run_dismiss` |
| MKO-16 | makoctl dismiss | CLI rejects `-n` together with `-a` or `-g` | invalid combinations | non-zero exit | `run_dismiss` |
| MKO-17 | makoctl invoke | default action key is `default` when action omitted | `makoctl invoke` | sends `default` | `run_invoke` |
| MKO-18 | makoctl list/history | printing tolerates missing optional fields | notification with empty app/category/etc. | readable output without garbage | `print_notification` |
| MKO-19 | makoctl menu | missing menu command is rejected | `makoctl menu` | non-zero exit | `run_menu` |
| MKO-20 | makoctl menu | selecting from notification with no actions errors | menu on actionless notification | `Notification has no actions` | `run_menu` |
| MKO-21 | makoctl menu | EOF from menu command is treated as cancellation | menu exits without selection | non-zero exit | `run_menu` |
| MKO-22 | makoctl menu | non-zero menu process exit is treated as failure | menu command exits 1 | non-zero exit | `run_menu` |
| MKO-23 | makoctl menu | selected title is mapped back to action key before invoke | titles differ from keys | correct key invoked | `run_menu` |
| MKO-24 | makoctl mode | `-a/-r/-t` cannot be mixed with `-s` | invalid CLI | non-zero exit | `run_mode` |
| MKO-25 | makoctl mode | positional args are only allowed with `-s` | `makoctl mode foo` | non-zero exit | `run_mode` |
| MKO-26 | makoctl mode | toggle removes existing mode and adds missing mode | repeated `-t` | list changes as expected | `run_mode` |

---

## 7. Rendering and layout (`render.c`)

| ID | Area | Intent | Stimulus | Expected result | Source |
|---|---|---|---|---|---|
| RND-01 | empty render | rendering with no notifications returns zero size | empty state | width=0, height=0 | `render` |
| RND-02 | max-visible | notifications beyond `max-visible` are hidden from direct rendering | list exceeding limit | only visible quota rendered | `render` |
| RND-03 | max-visible groups | grouped notifications count as one toward `max-visible` when `group_index < 1` | grouped list + limit | only first group member counts | `render` |
| RND-04 | hidden placeholder | hidden notifications produce synthetic hidden placeholder notification | hidden count > 0 | placeholder rendered with `%h/%t` data | `render`, `format_hidden_text` |
| RND-05 | hidden placeholder style | hidden placeholder obeys hidden criteria and may itself be invisible | hidden criteria sets `invisible=1` | placeholder omitted | `render` |
| RND-06 | second-pass criteria | notifications are re-matched immediately before render for output/anchor matching | compositor assigns output | style can change based on output/anchor | `render`, `apply_each_criteria` |
| RND-07 | width clamp | notification width is clamped to configured surface width | small compositor-granted width | rendered width shrinks | `render_notification` |
| RND-08 | text layout | empty text can shrink below line height | blank summary/body/format | compact notification height | `render_notification` |
| RND-09 | text alignment | left/center/right text alignment is respected | vary `text-alignment` | expected layout placement | `render_notification` |
| RND-10 | icon horizontal layout | left/right icon reduces text layout width | icon + left/right location | text box narrows appropriately | `render_notification` |
| RND-11 | icon vertical layout | top/bottom icon reduces text layout height and recenters text | icon + top/bottom | vertical layout behaves correctly | `render_notification` |
| RND-12 | border radius | final height is never smaller than needed for rounded corners | large radii + small text | height expanded to fit radii | `render_notification` |
| RND-13 | progress bar clamp | progress less than 0 or greater than 100 is clamped to drawable range | send `value=-10/150` | width clamps to [0, max] | `render_notification` |
| RND-14 | markup fallback | invalid Pango markup logs error and falls back to plain text | malformed text string | notification still renders | `render_notification` |
| RND-15 | hotspot geometry | rendered hotspot matches clickable notification bounds | render notification | hotspot x/y/width/height updated correctly | `render_notification` |
| RND-16 | border operator | translucent borders still mask background correctly | alpha border + progress/background | no background bleed outside clipped border | `render_notification` |

---

## 8. Icon resolution and loading (`icon.c`)

| ID | Area | Intent | Stimulus | Expected result | Source |
|---|---|---|---|---|---|
| ICO-01 | icon resolution | empty icon name resolves to no icon | `app_icon=""` | NULL result | `resolve_icon` |
| ICO-02 | icon resolution | absolute icon path is used as-is | `/tmp/icon.png` | same path duplicated | `resolve_icon` |
| ICO-03 | icon resolution | `file://` URI is URL-decoded into filesystem path | encoded URI | decoded absolute path returned | `resolve_icon`, `url_decode` |
| ICO-04 | icon name validation | invalid freedesktop icon names are rejected | names with `/`, spaces, very long input | NULL result | `validate_icon_name`, `resolve_icon` |
| ICO-05 | icon theme search | search honors configured `icon-path` and fallback `hicolor` | themed icon name | best path found from search roots | `resolve_icon` |
| ICO-06 | icon theme search | exact size + scale match wins immediately | multiple candidates including exact one | exact match selected | `resolve_icon` |
| ICO-07 | icon theme search | otherwise largest fitting icon up to scaled max is chosen | multiple non-exact candidates | largest acceptable one chosen | `resolve_icon` |
| ICO-08 | theme boundary | first theme with any match stops cross-theme search | same icon in multiple themes | first matching theme wins | `resolve_icon` |
| ICO-09 | pixmaps fallback | `/usr/share/pixmaps` is used if themed search fails | missing themed icon, present pixmap | pixmap path selected | `resolve_icon` |
| ICO-10 | icon precedence | raw `image-data` takes precedence over resolved file path | notif has image_data + app_icon | image_data used | `create_icon` |
| ICO-11 | icon scaling | icon scales down to fit square max size, but not up | large and small source images | scale <= 1 and dimensions match | `fit_to_square`, `create_icon` |
| ICO-12 | load failure | unreadable image file fails cleanly without crash | broken path/file | icon creation returns NULL | `load_image`, `create_icon` |

---

## 9. Wayland surfaces, input, and event loop (`wayland.c`, `surface.c`, `event-loop.c`)

| ID | Area | Intent | Stimulus | Expected result | Source |
|---|---|---|---|---|---|
| WLD-01 | init | daemon refuses to start without compositor, shm, or layer-shell globals | mock registry missing one global | init failure with diagnostic | `init_wayland` |
| WLD-02 | output metadata | output name/scale/subpixel are captured from registry listeners | advertise output events | output struct updated | `output_handle_*`, `create_output` |
| WLD-03 | output removal | removing an output clears surface output pointers | destroy active output | surfaces lose stale output refs | `destroy_output` |
| WLD-04 | seat capabilities | pointer/touch objects are recreated when capabilities change | toggle capability bits | old objects released, new ones added | `seat_handle_capabilities` |
| WLD-05 | pointer hit-testing | pointer button dispatch only triggers notification under cursor hotspot | click inside/outside hotspot | only hit notification reacts | `pointer_handle_button`, `hotspot_at` |
| WLD-06 | touch hit-testing | touch up dispatches to notification captured under touchpoint | touch down/up sequence | expected notification reacts | `touch_handle_down`, `touch_handle_up` |
| WLD-07 | touch bounds | touch ids >= `MAX_TOUCHPOINTS` are ignored safely | oversize touch id | no crash/state corruption | `touch_handle_*` |
| WLD-08 | cursor loading | invalid cursor size env falls back to default and logs warning | bad `XCURSOR_SIZE` | size=24 default | `init_wayland` |
| WLD-09 | cursor scale | cursor theme reloads only when scale changes | pointer enters same/different scale outputs | cursor reused or reloaded accordingly | `load_default_cursor`, `pointer_handle_enter` |
| WLD-10 | input region | input region covers only visible notification hotspots on a surface | multiple notifications on one surface | region is union of hotspot rectangles | `get_input_region` |
| WLD-11 | surface teardown | zero rendered height destroys layer surface and wl_surface | no notifications remain | surface torn down cleanly | `send_frame` |
| WLD-12 | surface relocation | moving to different configured output recreates surface | output assignment changes | old layer surface destroyed, new one created | `send_frame` |
| WLD-13 | configure loop | first render after surface creation requests size and waits for configure before drawing | new surface | size request committed, actual draw deferred | `send_frame`, `layer_surface_handle_configure` |
| WLD-14 | configure fast-path | configure with unchanged size just commits surface | repeated configure same size | no redundant redraw path | `layer_surface_handle_configure` |
| WLD-15 | layer close recovery | compositor-closed layer surface marks surface dirty and schedules redraw/recreation | send `closed` event while notifications still exist | surface recreated | `layer_surface_handle_closed` |
| WLD-16 | dirty scheduling | repeated `set_dirty` while already dirty is coalesced | call twice | one scheduled frame path | `set_dirty` |
| WLD-17 | frame callback | dirty surface redraws on frame done, clean surface does not | mark dirty/clean before callback | only dirty case redraws | `frame_handle_done` |
| WLD-18 | activation token | token creation returns NULL when compositor lacks protocol | no `xdg_activation_v1` | graceful NULL token | `create_xdg_activation_token` |
| WLD-19 | activation token | token creation blocks until done event and returns token string | protocol available | returned string matches compositor-provided token | `create_xdg_activation_token` |
| EVT-01 | timer ordering | earliest timer is armed in timerfd | add multiple timers | next timer matches earliest deadline | `update_event_loop_timer` |
| EVT-02 | timer destruction | destroying active next timer re-arms for following timer | remove earliest timer | second-earliest becomes active | `destroy_timer`, `update_event_loop_timer` |
| EVT-03 | timer callback | timer expiry destroys timer then invokes callback once | let timer fire | callback called once, timer removed | `handle_event_loop_timer` |
| EVT-04 | signal handling | SIGINT/SIGTERM/SIGQUIT are blocked into signalfd | start event loop and send signal | orderly loop shutdown path | `init_signalfd`, `run_event_loop` |

---

## 10. Prioritized first-pass suite

If only a small initial suite is possible, start with these:

1. `CFG-28` / `CFG-29` / `CFG-30` / `CFG-31` / `CFG-33` — config validation invariants
2. `CRT-12` / `CRT-13` / `NTF-07` — grouping and regrouping
3. `XDG-08` / `XDG-12` / `XDG-14` / `XDG-20` — tricky `Notify` hint handling
4. `XDG-16` / `XDG-17` / `NTF-05` / `MKO-06` — timeout/history/restore behavior
5. `RND-02` / `RND-03` / `RND-04` — max-visible and hidden placeholder behavior
6. `MKO-09` / `MKO-10` / `MKO-12` — reload/mode reapplication
7. `ICO-06` / `ICO-07` / `ICO-10` — icon selection precedence
8. `WLD-11` / `WLD-12` / `WLD-15` — surface lifecycle

## Gaps / things worth special attention

- `wayland.c` explicitly contains a TODO about possible infinite resize/configure looping if the compositor never grants the requested size. That deserves a stress/integration test.
- `dbus/mako.c:reapply_config` uses `free(notif_criteria)` instead of `destroy_criteria(notif_criteria)` in one path; even if harmless in practice, config reload/grouping should be exercised heavily.
- `makoctl.c:run_dismiss` contains dead code after an earlier `return`; not user-visible, but indicates this file should get focused CLI regression coverage.
- Hidden placeholder behavior depends on criteria matching plus runtime render pass logic, so render tests should verify both serialized state and pixel/layout-level behavior.
