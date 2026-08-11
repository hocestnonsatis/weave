# ADR-0002: Global store location at ~/.weave

## Status

Accepted (MVP)

## Date

2026-08-11

## Context

WEAVE.md §16 describes `~/.weave/store/objects/…`. The `directories` crate would place data under XDG (`~/.local/share/weave` on Linux).

## Decision

Use `$HOME/.weave` as the default global home, overridable with `WEAVE_HOME`. Object layout is `store/objects/sha256/…`.

## Consequences

- Matches the architecture document literally.
- Slightly non-XDG on Linux; can revisit once multi-platform support matters (open question Q5 remains open for metadata tech).
