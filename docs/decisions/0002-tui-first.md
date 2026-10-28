# 0002. TUI first, with ratatui

- Status: Accepted
- Date: 2026-10-27

## Context

The target users run keyboard-driven Linux desktops, where terminal tools are
the norm and terminal colours follow the system theme. A GUI toolkit (GTK4,
Slint, Iced) adds weight and theming work before the core is proven.

## Decision

The first front-end is a ratatui TUI (crossterm backend, re-exported by
ratatui). It uses only the terminal palette and text modifiers.

## Consequences

- Very small binary and dependency tree; themes apply automatically.
- All domain logic must stay in `tuner-core` so a GUI front-end can be added
  later without rewriting it.
- No channel logos or inline video in the UI.
