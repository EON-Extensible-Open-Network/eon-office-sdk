// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 EON contributors

//! **EON Hello** -- the reference integration: an independent application
//! that uses every part of the core an application normally touches.
//!
//! On start it stores a greeting, makes it searchable to other applications,
//! posts a notification, schedules a job, listens for preference changes and
//! announces `hello/ready`. It provides one method, `greet`, that any
//! application granted `call:org.eon.office.hello` can call. It exits when
//! the core asks it to.
//!
//! Its manifest is `examples/hello.manifest.json`. Run it under a core:
//!
//! ```text
//! eon-office-core serve &
//! # as the owner: apps.install the manifest (entry.command = this binary),
//! # grant its capabilities, then apps.launch it -- or start it yourself:
//! cargo run --example hello
//! ```

use eon_office_sdk::{AppError, Client, Incoming};
use serde_json::json;
use std::process::ExitCode;

const APP: &str = "org.eon.office.hello";

fn run() -> Result<(), eon_office_sdk::Error> {
    let client = Client::connect_app(APP)?;
    eprintln!("hello: connected; granted {:?}", client.granted());

    // Its own data.
    client.put(
        "greetings",
        "default",
        &json!({ "text": "Merhaba, dünya" }),
        None,
    )?;
    // Searchable by other applications, Turkish letters and all.
    client.index(
        "default",
        "Merhaba dünya",
        "İlk selamlama: EON Office'e hoş geldin.",
        true,
    )?;
    // The person's attention, once.
    client.notify(
        "EON Hello is running",
        "It answers `greet` for other applications.",
    )?;
    // A wake-up in half a second.
    client.schedule("tick", 500, None, &json!({ "count": 1 }))?;
    // Preferences arrive without subscribing; core events need a subscription.
    client.subscribe("core/*")?;
    client.publish("ready", &json!({ "version": env!("CARGO_PKG_VERSION") }))?;

    while let Some(message) = client.next(None) {
        match message {
            Incoming::Invoke {
                id,
                from,
                method,
                params,
            } => {
                let answer = match method.as_str() {
                    "greet" => match params["name"].as_str() {
                        Some(name) if !name.trim().is_empty() => Ok(json!({
                            "greeting": format!("Merhaba, {name}!"),
                            "for": from,
                        })),
                        _ => Err(AppError::new("missing_name", "greet needs a `name`")),
                    },
                    _ => Err(AppError::new("unknown_method", "not provided")),
                };
                client.answer(&id, answer)?;
            }
            Incoming::JobDue { name, payload, .. } => {
                client.publish("job.done", &json!({ "job": name, "payload": payload }))?;
            }
            Incoming::Event { topic, payload, .. } if topic == "core/prefs.changed" => {
                eprintln!(
                    "hello: preference {} is now {}",
                    payload["key"], payload["value"]
                );
            }
            Incoming::Event { .. } => {}
            Incoming::Shutdown => {
                eprintln!("hello: asked to stop; saving and leaving");
                client.put("greetings", "last-run", &json!({ "clean": true }), None)?;
                break;
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hello: {error}");
            ExitCode::FAILURE
        }
    }
}
