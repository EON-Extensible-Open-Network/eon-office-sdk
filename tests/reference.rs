// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 EON contributors

//! The reference integration, end to end: a real `eon-office-core` process,
//! EON Hello launched *by the core* as a separate process, and EON Caller
//! started as a third, independent process that reaches Hello only through
//! the core.
//!
//! Needs the core's binary: `EON_OFFICE_CORE_BIN=…/eon-office-core cargo test
//! -- --include-ignored`. CI builds eon-office-core and sets it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use eon_office_sdk::{Client, Incoming};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Core {
    child: Child,
    home: PathBuf,
}

impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn start_core() -> Core {
    let binary = std::env::var_os("EON_OFFICE_CORE_BIN")
        .expect("set EON_OFFICE_CORE_BIN to a built eon-office-core binary");
    let home = std::env::temp_dir().join(format!("eon-office-sdk-ref-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let child = Command::new(binary)
        .args(["serve", "--home"])
        .arg(&home)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while eon_office_sdk::endpoint(&home).is_err() {
        assert!(Instant::now() < deadline, "the core did not start");
        std::thread::sleep(Duration::from_millis(25));
    }
    Core { child, home }
}

/// The example binaries Cargo built beside this test (target/<profile>/examples).
fn example(name: &str) -> PathBuf {
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let path = deps
        .parent()
        .unwrap()
        .join("examples")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(path.is_file(), "{} not built", path.display());
    path
}

fn manifest(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(format!("{name}.manifest.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn wait_event(owner: &Client, topic: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Some(Incoming::Event {
            topic: t, payload, ..
        }) = owner.next(Some(Duration::from_millis(200)))
        {
            if t == topic {
                return payload;
            }
        }
    }
    panic!("no {topic} event");
}

#[test]
#[ignore = "needs EON_OFFICE_CORE_BIN; CI builds eon-office-core and sets it"]
fn an_independent_application_works_through_the_core() {
    let core = start_core();
    let owner = Client::connect_owner(&core.home).unwrap();
    owner.subscribe("org.eon.office.hello/*").unwrap();
    owner.subscribe("core/*").unwrap();

    // Install Hello with every capability it asked for, pointing at the built example.
    let mut hello = manifest("hello");
    hello["entry"]["command"] = json!(example("hello"));
    let requested = hello["capabilities"].clone();
    owner
        .call(
            "apps.install",
            json!({ "manifest": hello, "grant": requested }),
        )
        .unwrap();
    let caller = manifest("caller");
    owner
        .call(
            "apps.install",
            json!({ "manifest": caller, "grant": ["call:org.eon.office.hello"] }),
        )
        .unwrap();

    // Caller first, with Hello absent: the optional integration degrades, it does not fail.
    let run_caller = || {
        Command::new(example("caller"))
            .arg("Ada")
            .env("EON_OFFICE_HOME", &core.home)
            .env_remove("EON_OFFICE_APP")
            .output()
            .unwrap()
    };
    let alone = run_caller();
    assert!(alone.status.success());
    assert!(String::from_utf8_lossy(&alone.stdout).contains("not running"));

    // The core launches Hello as its own process.
    owner
        .call("apps.launch", json!({ "app": "org.eon.office.hello" }))
        .unwrap();
    let ready = wait_event(&owner, "org.eon.office.hello/ready");
    assert_eq!(ready["version"], "0.1.0");

    // A third process reaches Hello through the core.
    let together = run_caller();
    assert!(
        together.status.success(),
        "{}",
        String::from_utf8_lossy(&together.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&together.stdout).trim(),
        "Merhaba, Ada!"
    );

    // The owner can call it too; a bad call comes back as Hello's own error.
    let greeted = owner
        .call_app("org.eon.office.hello", "greet", json!({ "name": "Grace" }))
        .unwrap();
    assert_eq!(greeted["greeting"], "Merhaba, Grace!");
    let refused = owner
        .call_app("org.eon.office.hello", "greet", json!({}))
        .unwrap_err();
    assert_eq!(refused.code(), "remote_error");

    // Its scheduled job fired and it reported back.
    let done = wait_event(&owner, "org.eon.office.hello/job.done");
    assert_eq!(done["payload"]["count"], 1);

    // What it shared is searchable across the suite, however it is typed.
    let hits = owner.search("MERHABA").unwrap();
    assert!(hits.iter().any(|h| h["app"] == "org.eon.office.hello"));
    // Its notification is in the shared list.
    let notes = owner.call("notifications.list", json!({})).unwrap();
    assert_eq!(notes["items"][0]["app"], "org.eon.office.hello");

    // Asked to stop, it saves and leaves on its own -- not killed.
    let stopped = owner
        .call("apps.stop", json!({ "app": "org.eon.office.hello" }))
        .unwrap();
    assert_eq!(stopped["forced"], false);
    wait_event(&owner, "core/app.stopped");

    // And with Hello gone again, Caller still carries on.
    let after = run_caller();
    assert!(String::from_utf8_lossy(&after.stdout).contains("not running"));
    assert_eq!(owner.call("audit.verify", json!({})).unwrap()["ok"], true);
}
