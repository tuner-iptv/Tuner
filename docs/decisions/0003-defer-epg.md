# 0003. Defer EPG, keep the join key

- Status: Accepted
- Date: 2026-10-27

## Context

XMLTV guides are large (often hundreds of MB, gzipped) and need streaming
parsing plus indexed storage to be usable. That is more infrastructure than the
MVP needs.

## Decision

EPG is out of scope until v0.4 (see [roadmap](../roadmap.md)). The parser
still records `tvg-id` into `Channel::tvg_id`, which the EPG will join on.

## Consequences

- MVP ships sooner.
- Don't drop or rename `tvg_id`. When EPG lands, it goes in a separate
  `tuner-epg` crate, parsed with a streaming XML reader into storage, never
  loaded whole into memory.
