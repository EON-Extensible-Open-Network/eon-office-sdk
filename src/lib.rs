// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 EON contributors

//! # eon-office-sdk
//!
//! Everything an EON Office application needs to talk to the core:
//!
//! ```no_run
//! use eon_office_sdk::{Client, Incoming};
//! use serde_json::json;
//!
//! # fn main() -> Result<(), eon_office_sdk::Error> {
//! let client = Client::connect_app("org.example.hello")?;
//! client.call("storage.put", json!({ "collection": "notes", "id": "a", "doc": { "t": "hi" } }))?;
//! while let Some(message) = client.next(None) {
//!     match message {
//!         Incoming::Invoke { id, method, .. } if method == "greet" => {
//!             client.answer(&id, Ok(json!("Merhaba!")))?;
//!         }
//!         Incoming::Shutdown => break,
//!         _ => {}
//!     }
//! }
//! # Ok(()) }
//! ```
//!
//! The contract is `eon-office-spec/docs/protocol.md`; this crate is a thin,
//! honest layer over it. Every method in the protocol can be reached with
//! [`Client::call`]; the typed helpers exist for the common ones.
//!
//! **Integrations are optional.** When another application is not installed
//! or not running, [`Client::call_app`] returns an error whose
//! [`Error::code`] is `app_unavailable`. Treat that as a normal answer: your
//! application must keep working without it.
//!
//! MIT-licensed, so building an application on EON Office never raises a
//! licensing question.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, PoisonError};
use std::time::Duration;

/// The protocol version this SDK speaks.
pub const API_VERSION: u64 = 0;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- errors ----------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Error {
    /// The core answered with a protocol error. `code` is the stable string
    /// (`forbidden`, `app_unavailable`, …); choose your own words from it.
    Protocol {
        code: String,
        message: String,
        data: Value,
    },
    /// The core could not be found or reached, or the connection broke.
    Connection(String),
}

impl Error {
    /// The stable error code, or `connection` when the core was not reached.
    pub fn code(&self) -> &str {
        match self {
            Error::Protocol { code, .. } => code,
            Error::Connection(_) => "connection",
        }
    }

    /// For [`Error::Protocol`] with `forbidden`: the capability to ask for.
    pub fn missing_capability(&self) -> Option<&str> {
        match self {
            Error::Protocol { code, data, .. } if code == "forbidden" => {
                data.get("capability").and_then(Value::as_str)
            }
            _ => None,
        }
    }

    fn from_json(error: &Value) -> Self {
        Error::Protocol {
            code: error["data"]["code"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned(),
            message: error["message"].as_str().unwrap_or_default().to_owned(),
            data: error.get("data").cloned().unwrap_or(Value::Null),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Protocol { code, message, .. } => write!(f, "{code}: {message}"),
            Error::Connection(message) => write!(f, "connection: {message}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

// ---- discovery -------------------------------------------------------------

/// The core's home directory, as `protocol.md` §1 defines it.
pub fn default_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("EON_OFFICE_HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(home));
    }
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|b| PathBuf::from(b).join("EON").join("Office"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|b| PathBuf::from(b).join("Library/Application Support/EON Office"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|b| b.join("eon-office"))
    }
}

/// The address of the core running for `home`, from `runtime/endpoint.json`.
pub fn endpoint(home: &Path) -> Result<String> {
    let path = home.join("runtime").join("endpoint.json");
    let text = std::fs::read_to_string(&path).map_err(|_| {
        Error::Connection(format!(
            "no core is running for {} (no endpoint.json)",
            home.display()
        ))
    })?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Connection(format!("endpoint.json is not JSON: {e}")))?;
    value["address"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Connection("endpoint.json has no address".into()))
}

fn read_token(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .map(|t| t.trim().to_owned())
        .map_err(|_| Error::Connection(format!("cannot read the token at {}", path.display())))
}

// ---- incoming messages -----------------------------------------------------

/// Something the core sent without being asked.
#[derive(Debug, Clone)]
pub enum Incoming {
    /// An event this connection subscribed to (or `core/prefs.changed`,
    /// `core/storage.changed`, which every application receives).
    Event {
        topic: String,
        from: String,
        payload: Value,
    },
    /// Another application (or the owner) called a method this application
    /// provides. Answer with [`Client::answer`] and the same `id`.
    Invoke {
        id: Value,
        from: String,
        method: String,
        params: Value,
    },
    /// A scheduled job fell due.
    JobDue {
        name: String,
        payload: Value,
        scheduled_at: i64,
    },
    /// The core asks this application to save and exit.
    Shutdown,
}

fn classify(message: &Value) -> Option<Incoming> {
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let text = |key: &str| params[key].as_str().unwrap_or_default().to_owned();
    match message.get("method")?.as_str()? {
        "event" => Some(Incoming::Event {
            topic: text("topic"),
            from: text("from"),
            payload: params["payload"].clone(),
        }),
        "invoke" => Some(Incoming::Invoke {
            id: message.get("id")?.clone(),
            from: text("from"),
            method: text("method"),
            params: params["params"].clone(),
        }),
        "job.due" => Some(Incoming::JobDue {
            name: text("name"),
            payload: params["payload"].clone(),
            scheduled_at: params["scheduled_at"].as_i64().unwrap_or_default(),
        }),
        "shutdown" => Some(Incoming::Shutdown),
        _ => None,
    }
}

// ---- the client --------------------------------------------------------------

type Waiting = Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>;

/// A connection to the core. Cheap to share between threads (`&Client` is
/// enough to call); calls from several threads proceed concurrently.
pub struct Client {
    writer: Mutex<TcpStream>,
    waiting: Waiting,
    incoming: Mutex<mpsc::Receiver<Incoming>>,
    next_id: AtomicU64,
    timeout: Duration,
    hello: Value,
}

impl Client {
    /// Connect as the application `app`. Uses `EON_OFFICE_ADDRESS` and
    /// `EON_OFFICE_TOKEN` when the core launched this process, otherwise the
    /// home directory's `endpoint.json` and `apps/<app>/token`.
    pub fn connect_app(app: &str) -> Result<Client> {
        let from_core = std::env::var("EON_OFFICE_APP").ok().filter(|a| a == app);
        let (address, token) = match (
            from_core,
            std::env::var("EON_OFFICE_ADDRESS"),
            std::env::var("EON_OFFICE_TOKEN"),
        ) {
            (Some(_), Ok(address), Ok(token)) if !address.is_empty() => (address, token),
            _ => {
                let home =
                    default_home().ok_or_else(|| Error::Connection("no home directory".into()))?;
                (
                    endpoint(&home)?,
                    read_token(&home.join("apps").join(app).join("token"))?,
                )
            }
        };
        Client::connect(&address, Some(app), &token)
    }

    /// Connect as the owner, with `runtime/owner.token` from `home`.
    pub fn connect_owner(home: &Path) -> Result<Client> {
        let token = read_token(&home.join("runtime").join("owner.token"))?;
        Client::connect(&endpoint(home)?, None, &token)
    }

    /// Connect to `address` and say hello. `app: None` is the owner.
    pub fn connect(address: &str, app: Option<&str>, token: &str) -> Result<Client> {
        let stream = TcpStream::connect(address)
            .map_err(|e| Error::Connection(format!("cannot reach the core at {address}: {e}")))?;
        let _ = stream.set_nodelay(true);
        let reader = stream
            .try_clone()
            .map_err(|e| Error::Connection(e.to_string()))?;
        let waiting: Waiting = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::channel();
        let pending = Arc::clone(&waiting);
        std::thread::spawn(move || read_loop(reader, &pending, &tx));
        let mut client = Client {
            writer: Mutex::new(stream),
            waiting,
            incoming: Mutex::new(rx),
            next_id: AtomicU64::new(1),
            timeout: Duration::from_secs(70),
            hello: Value::Null,
        };
        let mut params = json!({ "token": token, "api": API_VERSION });
        if let Some(app) = app {
            params["app"] = json!(app);
        }
        client.hello = client.call("session.hello", params)?;
        Ok(client)
    }

    /// What `session.hello` answered: `granted`, `pending`, `core_version`…
    pub fn hello(&self) -> &Value {
        &self.hello
    }

    /// Capabilities granted to this application at connect time. The core
    /// checks again on every call, so treat this as a hint for the interface.
    pub fn granted(&self) -> Vec<String> {
        self.hello["granted"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn write(&self, message: &Value) -> Result<()> {
        let mut writer = lock(&self.writer);
        let mut line = message.to_string();
        line.push('\n');
        writer
            .write_all(line.as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|e| Error::Connection(format!("the core went away: {e}")))
    }

    /// Call any protocol method.
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        lock(&self.waiting).insert(id, tx);
        let mut request = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        request["params"] = params;
        let sent = self.write(&request);
        if let Err(error) = sent {
            lock(&self.waiting).remove(&id);
            return Err(error);
        }
        let answer = rx.recv_timeout(self.timeout);
        lock(&self.waiting).remove(&id);
        let answer = answer.map_err(|_| Error::Connection(format!("no answer to {method}")))?;
        match answer.get("error") {
            Some(error) => Err(Error::from_json(error)),
            None => Ok(answer.get("result").cloned().unwrap_or(Value::Null)),
        }
    }

    /// The next unrequested message, waiting up to `timeout` (`None`: until
    /// one arrives or the connection closes). `None` back means closed or
    /// timed out.
    pub fn next(&self, timeout: Option<Duration>) -> Option<Incoming> {
        let incoming = lock(&self.incoming);
        match timeout {
            Some(t) => incoming.recv_timeout(t).ok(),
            None => incoming.recv().ok(),
        }
    }

    /// Answer an [`Incoming::Invoke`].
    pub fn answer(&self, id: &Value, outcome: std::result::Result<Value, AppError>) -> Result<()> {
        let message = match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": {
                "code": -32000, "message": error.message, "data": { "code": error.code },
            } }),
        };
        self.write(&message)
    }

    // ---- typed helpers ----

    pub fn put(&self, collection: &str, id: &str, doc: &Value, rev: Option<u64>) -> Result<Value> {
        let mut params = json!({ "collection": collection, "id": id, "doc": doc });
        if let Some(rev) = rev {
            params["rev"] = json!(rev);
        }
        self.call("storage.put", params)
    }

    /// The document, or `None` if there is none.
    pub fn get(&self, collection: &str, id: &str) -> Result<Option<Value>> {
        let found = self.call("storage.get", json!({ "collection": collection, "id": id }))?;
        Ok(if found.is_null() { None } else { Some(found) })
    }

    pub fn publish(&self, event: &str, payload: &Value) -> Result<()> {
        self.call(
            "events.publish",
            json!({ "event": event, "payload": payload }),
        )
        .map(drop)
    }

    pub fn subscribe(&self, pattern: &str) -> Result<()> {
        self.call("events.subscribe", json!({ "pattern": pattern }))
            .map(drop)
    }

    pub fn notify(&self, title: &str, body: &str) -> Result<()> {
        self.call(
            "notifications.post",
            json!({ "title": title, "body": body }),
        )
        .map(drop)
    }

    pub fn index(&self, id: &str, title: &str, text: &str, shared: bool) -> Result<()> {
        let visibility = if shared { "shared" } else { "private" };
        self.call(
            "search.index",
            json!({ "id": id, "title": title, "text": text, "visibility": visibility }),
        )
        .map(drop)
    }

    pub fn search(&self, query: &str) -> Result<Vec<Value>> {
        let found = self.call("search.query", json!({ "query": query }))?;
        Ok(found["hits"].as_array().cloned().unwrap_or_default())
    }

    pub fn schedule(
        &self,
        name: &str,
        delay_ms: u64,
        every_ms: Option<u64>,
        payload: &Value,
    ) -> Result<()> {
        let mut params = json!({ "name": name, "delay_ms": delay_ms, "payload": payload });
        if let Some(every) = every_ms {
            params["every_ms"] = json!(every);
        }
        self.call("schedule.add", params).map(drop)
    }

    /// Call a method another application provides. `app_unavailable` is a
    /// normal answer: that application is optional.
    pub fn call_app(&self, app: &str, method: &str, params: Value) -> Result<Value> {
        let mut request = json!({ "app": app, "method": method });
        request["params"] = params;
        self.call("apps.call", request)
    }

    pub fn prefs(&self) -> Result<Value> {
        self.call("prefs.get", json!({}))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = lock(&self.writer).shutdown(std::net::Shutdown::Both);
    }
}

/// An error this application answers an [`Incoming::Invoke`] with. The caller
/// receives it as `remote_error`, with this inside `data.error`.
#[derive(Debug, Clone)]
pub struct AppError {
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn new(code: &str, message: &str) -> Self {
        AppError {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}

fn read_loop(stream: TcpStream, waiting: &Waiting, incoming: &mpsc::Sender<Incoming>) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let is_reply = message.get("method").is_none();
        if is_reply {
            if let Some(id) = message.get("id").and_then(Value::as_u64) {
                if let Some(waiter) = lock(waiting).remove(&id) {
                    let _ = waiter.send(message);
                }
            }
        } else if let Some(item) = classify(&message) {
            if incoming.send(item).is_err() {
                break;
            }
        }
    }
    // Closed: wake every caller still waiting.
    lock(waiting).clear();
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn incoming_messages_are_classified() {
        let invoke = json!({ "jsonrpc": "2.0", "id": "i3", "method": "invoke",
            "params": { "from": "org.a.b", "method": "greet", "params": { "name": "Ada" } } });
        match classify(&invoke) {
            Some(Incoming::Invoke {
                id,
                from,
                method,
                params,
            }) => {
                assert_eq!(
                    (id, from.as_str(), method.as_str()),
                    (json!("i3"), "org.a.b", "greet")
                );
                assert_eq!(params["name"], "Ada");
            }
            other => panic!("{other:?}"),
        }
        let job = json!({ "jsonrpc": "2.0", "method": "job.due", "params": { "name": "n", "payload": 1, "scheduled_at": 5 } });
        assert!(matches!(
            classify(&job),
            Some(Incoming::JobDue {
                scheduled_at: 5,
                ..
            })
        ));
        assert!(matches!(
            classify(&json!({ "method": "shutdown" })),
            Some(Incoming::Shutdown)
        ));
        assert!(
            classify(&json!({ "method": "something.new" })).is_none(),
            "unknown is ignored"
        );
    }

    #[test]
    fn errors_expose_the_stable_code_and_the_missing_capability() {
        let error = Error::from_json(&json!({ "code": -32002, "message": "no",
            "data": { "code": "forbidden", "capability": "storage" } }));
        assert_eq!(error.code(), "forbidden");
        assert_eq!(error.missing_capability(), Some("storage"));
        assert_eq!(Error::Connection("x".into()).code(), "connection");
    }

    #[test]
    fn a_missing_core_is_a_connection_error() {
        let home = std::env::temp_dir().join("eon-office-sdk-no-such-home");
        let error = endpoint(&home).err().map(|e| e.code().to_owned());
        assert_eq!(error.as_deref(), Some("connection"));
    }
}
