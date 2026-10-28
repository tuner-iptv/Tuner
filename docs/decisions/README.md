# Architecture Decision Records

Short records of decisions that shape the codebase. Read them before reversing
one; write a new ADR (don't edit an accepted one) when you do.

| #    | Decision                                                          | Status   |
| ---- | ----------------------------------------------------------------- | -------- |
| 0001 | [External mpv over JSON IPC](0001-external-mpv-over-ipc.md)       | Accepted |
| 0002 | [TUI first, with ratatui](0002-tui-first.md)                      | Accepted |
| 0003 | [Defer EPG, keep the join key](0003-defer-epg.md)                 | Accepted |
| 0004 | [Credentials in config, never on the CLI](0004-credentials.md)    | Accepted |
| 0005 | [Enforce provider stream limits](0005-stream-limits.md)           | Accepted |
| 0006 | [GTK4 desktop front-end, mpv still external](0006-gtk4-gui.md)    | Accepted |
| 0007 | [Optional embedded video via libmpv](0007-embedded-video.md)      | Accepted |

## Template

Copy to `NNNN-short-title.md`:

```markdown
# NNNN. Title

- Status: Proposed | Accepted | Superseded by NNNN
- Date: YYYY-MM-DD

## Context
What forces are at play?

## Decision
What we do.

## Consequences
What gets easier, what gets harder, what we must watch for.
```
