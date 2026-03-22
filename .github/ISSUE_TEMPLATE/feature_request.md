---
name: Feature request
about: Suggest a new feature or improvement
title: '[FEATURE] '
labels: enhancement
assignees: adityamukho
---

## Summary

A one-sentence description of the feature.

## Motivation

What problem does this solve? What use case does it enable?

## Proposed design

How would it work? Include API sketches, Datalog syntax examples, or storage format changes if applicable.

## Philosophy check

Before submitting, please confirm:

- [ ] This does not break the single-file storage philosophy
- [ ] This does not require client-server architecture or external services
- [ ] This does not add heavy dependencies that increase binary size significantly
- [ ] This is useful for embedded use cases (mobile, WASM, desktop apps)

See [PHILOSOPHY.md](../PHILOSOPHY.md) and [ROADMAP.md](../ROADMAP.md) for context.

## Alternatives considered

What other approaches did you consider and why did you rule them out?
