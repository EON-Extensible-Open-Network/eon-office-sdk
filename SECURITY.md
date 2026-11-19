<!--
SPDX-License-Identifier: MIT
SPDX-FileCopyrightText: 2026 EON contributors
-->

# Security

Report privately through GitHub's **Security → Report a vulnerability** on this repository;
the organisation-wide policy is
[`.github/SECURITY.md`](https://github.com/EON-Extensible-Open-Network/.github/blob/main/SECURITY.md).

The SDK handles the application's token: it reads it from `EON_OFFICE_TOKEN` or the token
file and sends it once, in `session.hello`, to a loopback address. It never logs it. What
the core defends against is in
[`eon-office-core/SECURITY.md`](https://github.com/EON-Extensible-Open-Network/eon-office-core/blob/main/SECURITY.md).
