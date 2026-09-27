//! Audit log: every state change, with what it was before.
//!
//! # What this is for
//!
//! Every change Lanes makes is logged with the target app, the old value and
//! the new value. When something behaves oddly weeks from now, this log is the
//! difference between a five-minute fix and guesswork.
//!
//! It matters more than it would for most programs: Windows persists per-app
//! routing and volume against the **application**, so changes Lanes makes
//! **outlive Lanes itself** — uninstalling without restoring would leave every
//! assignment in force. This log is how someone works out what happened and
//! why their audio is where it is.
//!
//! # Why this is not `tracing`
//!
//! `tracing` is the right tool for *diagnostic* logging — spans, levels,
//! filtering, "what was the code doing". This is a different thing: an audit
//! trail of state changes, which is structured data, not prose.
//!
//! Writing it as JSON Lines means it can be queried (`what happened to
//! Discord.exe last Tuesday`) rather than grepped and eyeballed. Burying
//! `old: 0.25, new: 1.0` inside a formatted sentence would make the most
//! useful part of the record the hardest to use.
//!
//! Diagnostic output from the core goes to stdout and stderr, which a terminal
//! sees when Lanes is started from one (see `console`).

use std::fs::OpenOptions;
use std::io::Write;

use serde::{Deserialize, Serialize};

use crate::paths;

/// One recorded change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// ISO-8601 local time with offset, so a log read in a year is unambiguous.
    pub at: String,
    /// What was acted on — usually an executable name.
    pub target: String,
    /// Which property changed: `volume`, `mute`, `endpoint`.
    pub field: String,
    /// Value before the change. `None` means "was not set".
    pub old: Option<String>,
    /// Value after. `None` means "cleared".
    pub new: Option<String>,
    /// Why the app did this — `first-run`, `restore`, `user`, `rule`.
    pub reason: String,
    /// Whether it actually worked.
    pub ok: bool,
    /// Error text when it did not.
    pub error: Option<String>,
}

pub fn now_iso() -> String {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    // Local time where possible, UTC otherwise. `now_local` fails on some
    // configurations, and a log with a timestamp in the wrong zone is worse
    // than one that says plainly it is UTC — so the fallback is labelled.
    match OffsetDateTime::now_local() {
        Ok(t) => t.format(&Rfc3339).unwrap_or_else(|_| "unknown".into()),
        Err(_) => OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map(|s| format!("{s} (UTC)"))
            .unwrap_or_else(|_| "unknown".into()),
    }
}

fn today_stamp() -> String {
    use time::OffsetDateTime;
    let t = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    format!("{:04}-{:02}-{:02}", t.year(), t.month() as u8, t.day())
}

/// Append an entry. One file per day, so the log rolls without any pruning
/// logic and a day's activity is trivially findable.
///
/// **Never fails the caller.** A change that succeeded must not be reported as
/// failed because the log could not be written, and a change that is being
/// undone must not be blocked by a full disk. Logging problems go to stderr.
pub fn record(entry: &Entry) {
    if let Err(e) = try_record(entry) {
        eprintln!("warning: could not write audit log: {e}");
    }
}

fn try_record(entry: &Entry) -> std::io::Result<()> {
    let dir = paths::logs_dir();
    paths::ensure_dir(&dir)?;

    let path = dir.join(format!("audit-{}.jsonl", today_stamp()));
    let line = serde_json::to_string(entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

/// Record a successful change.
pub fn change(target: &str, field: &str, old: Option<String>, new: Option<String>, reason: &str) {
    record(&Entry {
        at: now_iso(),
        target: target.to_string(),
        field: field.to_string(),
        old,
        new,
        reason: reason.to_string(),
        ok: true,
        error: None,
    });
}

/// Record an attempted change that failed.
///
/// Failures are logged as deliberately as successes: "we tried to move Discord
/// and Windows refused" is exactly the kind of thing that is impossible to
/// reconstruct later from a log that only records what worked.
pub fn failure(target: &str, field: &str, attempted: Option<String>, reason: &str, error: &str) {
    record(&Entry {
        at: now_iso(),
        target: target.to_string(),
        field: field.to_string(),
        old: None,
        new: attempted,
        reason: reason.to_string(),
        ok: false,
        error: Some(error.to_string()),
    });
}
