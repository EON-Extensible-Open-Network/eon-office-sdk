<!--
SPDX-License-Identifier: MIT
SPDX-FileCopyrightText: 2026 EON contributors
-->

# eon-office-sdk

**Build an EON Office application.** A small Rust client for the
[EON Office protocol](https://github.com/EON-Extensible-Open-Network/eon-office-spec/blob/main/docs/protocol.md):
it finds the core, authenticates, calls methods, and hands you the events, calls from other
applications and background jobs the core sends. One dependency (`serde_json`). MIT, so
building on EON Office never raises a licensing question.

> Protocol `v0`, unstable: expect breaking changes until `v1`.

```rust
use eon_office_sdk::{Client, Incoming};
use serde_json::json;

let client = Client::connect_app("org.example.notes")?;
client.put("notes", "n1", &json!({ "title": "Shopping" }), None)?;
client.index("n1", "Shopping", "milk, bread", true)?;
match client.call_app("org.example.tasks", "task.create", json!({ "title": "Buy milk" })) {
    Ok(task) => println!("task {task}"),
    Err(e) if e.code() == "app_unavailable" => {} // Tasks is optional. Carry on.
    Err(e) => return Err(e),
}
while let Some(message) = client.next(None) {
    match message {
        Incoming::Invoke { id, method, params, .. } => { /* client.answer(&id, ...) */ }
        Incoming::Event { topic, payload, .. } => { /* ... */ }
        Incoming::JobDue { name, .. } => { /* ... */ }
        Incoming::Shutdown => break,
    }
}
```

`Client::call(method, params)` reaches every method in the protocol; the typed helpers
(`put`, `get`, `publish`, `subscribe`, `notify`, `index`, `search`, `schedule`, `call_app`,
`prefs`) cover the common ones.

## The reference integration

`examples/` holds two independent applications and their manifests:

- **EON Hello** (`hello.rs`, `hello.manifest.json`): stores a document, shares a search
  entry, posts a notification, schedules a job, publishes events, follows preferences,
  provides `greet`, and saves and exits when the core asks.
- **EON Caller** (`caller.rs`, `caller.manifest.json`): calls Hello's `greet` if Hello is
  running, and carries on when it is not.

`tests/reference.rs` runs the whole thing against a real `eon-office-core`: the core
launches Hello as its own process, Caller runs as a third process and reaches Hello only
through the core, and Caller is checked with Hello absent, present, and stopped again.

```
# build eon-office-core, then:
EON_OFFICE_CORE_BIN=/path/to/eon-office-core cargo test -- --include-ignored
```

## Writing an application: the rules that matter

1. **Declare everything in the manifest**: capabilities, the methods you `provide`, the
   events you `publish`. The core refuses anything undeclared.
2. **Every integration is optional.** `app_unavailable` is a normal answer.
3. **Answer `invoke` promptly**, and handle `shutdown` by saving and exiting.
4. **Switch on `Error::code()`**, never on the message, and show your own words in the
   person's language.
5. **Follow the person's preferences** (`prefs`, and `core/prefs.changed`): reduced motion,
   text size, contrast, theme.

Licence: MIT. Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Security:
[SECURITY.md](SECURITY.md).
