//! Send a single command to a core that is already running.
//!
//! # Why this exists
//!
//! Two things need one process to talk to another:
//!
//! - **Relaunching shows the running instance's window.** Starting a second
//!   copy — from the Start Menu, a desktop shortcut, or by double-clicking the
//!   executable while it sits in the tray — brings up the mixer rather than
//!   refusing, which is what someone who just clicked a shortcut expects.
//!
//! - **The uninstaller has to stop the app.** Windows will not delete a file a
//!   running process holds open, so an uninstall with the tray still running
//!   leaves the folder behind and the Run key pointing into it.
//!
//! # Why the local API rather than a mutex or a window message
//!
//! Because it already exists, is already the documented way to drive this
//! application, and is already what the mixer window and a Stream Deck plugin
//! use. A named pipe or a `WM_COPYDATA` handler would be a second control
//! surface with its own bugs, to do something the first one can already do.
//!
//! The cost is that this path depends on the port file and the token being
//! readable, and it says so plainly when they are not.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::config::Config;
use crate::paths;

/// How long to wait for the core to answer.
///
/// Short. Every caller here is interactive — a shortcut click, or an
/// uninstaller with a progress bar — and a core that has not replied in two
/// seconds is not going to.
const TIMEOUT: Duration = Duration::from_secs(2);

/// Why a one-shot command could not be delivered.
#[derive(Debug)]
pub enum Error {
    /// No port file, so nothing is running — or it never started.
    NotRunning,
    /// The port file is there but nothing answered on it.
    NoAnswer(std::io::Error),
    /// It answered and refused.
    Refused(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotRunning => write!(f, "Lanes does not appear to be running"),
            Error::NoAnswer(e) => write!(f, "could not reach the running instance: {e}"),
            Error::Refused(m) => write!(f, "the running instance refused: {m}"),
        }
    }
}

/// Send one command and wait for the reply.
///
/// `command` is the command name as it appears in `docs/api.md`.
pub fn send(command: &str) -> Result<(), Error> {
    let port = read_port().ok_or(Error::NotRunning)?;
    let token = Config::load().api_token.unwrap_or_default();

    let body = format!(r#"{{"command":"{command}","token":"{token}"}}"#);
    let request = format!(
        "POST / HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );

    let address = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));

    // A port file that nobody is listening on means the app is not running.
    //
    // Nothing deletes the port file on exit - a crash could not, so a clean
    // exit doing it would only make the leftover case rarer and harder to
    // reason about. The file is therefore a hint, not proof, and the connection
    // is what actually answers the question. Reporting "could not reach the
    // running instance" here would be describing a running instance that does
    // not exist.
    let mut stream = match TcpStream::connect_timeout(&address, TIMEOUT) {
        Ok(stream) => stream,
        Err(_) => return Err(Error::NotRunning),
    };
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));

    stream
        .write_all(request.as_bytes())
        .map_err(Error::NoAnswer)?;

    let mut response = String::new();
    // A short read is fine: everything needed is in the first packet, and a
    // command that quits the core will have its connection cut mid-reply.
    let _ = stream.read_to_string(&mut response);

    // `"ok":true` is the whole contract. Parsing the state back would mean
    // carrying the reply types into a path whose only question is whether the
    // command landed.
    if response.contains(r#""ok":true"#) {
        return Ok(());
    }

    // A quit races its own reply: the core stops before the socket drains, so
    // an empty response here means the command almost certainly worked.
    if response.is_empty() && command == "quit" {
        return Ok(());
    }

    Err(Error::Refused(summarise(&response)))
}

/// Ask a running instance to show its window. Returns false if none is running.
pub fn show_window() -> bool {
    send("show_window").is_ok()
}

fn read_port() -> Option<u16> {
    std::fs::read_to_string(paths::port_file())
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// The error message out of a JSON reply, without pulling in a parser.
fn summarise(response: &str) -> String {
    const KEY: &str = r#""message":""#;

    match response.find(KEY) {
        Some(at) => {
            let rest = &response[at + KEY.len()..];
            rest.split('"').next().unwrap_or(rest).to_string()
        }
        None if response.is_empty() => "no reply".to_string(),
        None => response.lines().next().unwrap_or("").to_string(),
    }
}
