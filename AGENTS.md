# Project agent preferences (wispd)

- Keep docs concise; avoid verbose planning text.
- Keep docs up to date with code changes.
- Make regular commits using Conventional Commits.
- Focus on `org.freedesktop.Notifications` over D-Bus.
- No portal integration in v1.

## Architecture (v1)
- One main binary: `wispd`.
- Core daemon logic should live in library crates, not in the binary.
- Separate debug consumer binary: `wisp-debug`.
- Keep reusable crates in `crates/` and executables in `bins/`.

## Workspace naming
- Prefer `wisp*` crate names.
- Current proposed libraries:
  - `wisp-types`
  - `wisp-source`
- Current proposed binaries:
  - `wispd`
  - `wispd-ui` (optional future split frontend)
  - `wisp-debug`
