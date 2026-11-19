// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 EON contributors

//! **EON Caller** -- a second, independent application that integrates with
//! EON Hello *optionally*. If Hello is there, it gets a greeting; if Hello is
//! not installed or not running, it says so and carries on. That second path
//! is the one every EON Office integration must have (madde 43b).
//!
//! Manifest: `examples/caller.manifest.json`.
//!
//! ```text
//! cargo run --example caller -- Ada
//! ```

use eon_office_sdk::Client;
use serde_json::json;
use std::process::ExitCode;

fn main() -> ExitCode {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "dünya".to_owned());
    let client = match Client::connect_app("org.eon.office.caller") {
        Ok(client) => client,
        Err(error) => {
            eprintln!("caller: {error}");
            return ExitCode::FAILURE;
        }
    };
    match client.call_app("org.eon.office.hello", "greet", json!({ "name": name })) {
        Ok(answer) => {
            println!("{}", answer["greeting"].as_str().unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(error) if error.code() == "app_unavailable" => {
            // Not an error for us: Hello is optional.
            println!("EON Hello is not running; carrying on without it.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("caller: {error}");
            ExitCode::FAILURE
        }
    }
}
