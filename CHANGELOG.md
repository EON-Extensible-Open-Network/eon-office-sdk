<!--
SPDX-License-Identifier: MIT
SPDX-FileCopyrightText: 2026 EON contributors
-->

# Changelog

## 0.1.0 — first version

- `Client`: discovery (`EON_OFFICE_*` environment from the core, else `endpoint.json` and
  the token file), hello as an application or the owner, concurrent calls, incoming
  events, invocations, jobs and shutdown, answers to invocations.
- Typed helpers for storage, events, notifications, search, jobs, calls and preferences.
- `Error::code()` and `Error::missing_capability()`.
- The reference integration: EON Hello and EON Caller, run end to end against a real core
  in `tests/reference.rs`.
