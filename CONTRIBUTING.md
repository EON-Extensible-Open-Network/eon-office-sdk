<!--
SPDX-License-Identifier: MIT
SPDX-FileCopyrightText: 2026 EON contributors
-->

# Contributing to eon-office-sdk

The organisation-wide guide,
[`.github/CONTRIBUTING.md`](https://github.com/EON-Extensible-Open-Network/.github/blob/main/CONTRIBUTING.md),
applies: DCO sign-off (`git commit -s`), Conventional Commits (`feat(sdk): ...`).

- The SDK follows the protocol in `eon-office-spec`; it never adds behaviour the protocol
  does not have. Something missing is a spec change first.
- Keep it to one dependency. An application author inherits every crate here.
- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo test -- --include-ignored` with `EON_OFFICE_CORE_BIN` set must pass.
- Contributions are licensed MIT.
