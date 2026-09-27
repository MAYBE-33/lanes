//! The local API, and the reason the UI is honest.
//!
//! # Everything goes through it
//!
//! The mixer window is a client of exactly this surface, with no privileged
//! path into the core. That is what makes the Stream Deck plugin a second
//! client with no extra backend work, and it is why a script or another tool
//! can do anything the window can. The contract is [`protocol`]; `docs/api.md`
//! describes it for people.
//!
//! # Why there is no async runtime
//!
//! **This uses blocking `tungstenite` on dedicated threads**, not `tokio`, and
//! the reason is COM, not preference.
//!
//! Every Windows audio call in this project must happen on the thread that
//! initialised the COM apartment. We use a single-threaded apartment
//! deliberately — the undocumented `IAudioPolicyConfigFactory` is sensitive to
//! apartment model, and STA is what the reference implementation uses. An async
//! runtime that moves futures between worker threads would violate that, and
//! the failure mode is not a clean error: it is `RPC_E_WRONG_THREAD` at best,
//! and silent misbehaviour at worst.
//!
//! So: **one thread owns COM and all state**, and does every audio operation.
//! Connections live on their own threads and talk to it over channels. With one
//! UI window and one Stream Deck plugin there is no concurrency pressure an
//! async runtime would relieve, and this arrangement makes the threading rule
//! impossible to break by accident rather than merely documented.
//!
//! `tokio` remains the right answer if this ever needs to serve many clients.
//! It does not.
//!
//! # Loopback only
//!
//! The listener binds `127.0.0.1` and never `0.0.0.0`. A shared token from
//! `config.json` is checked on every command — enough to stop other local
//! processes poking it casually. Requests from web pages are refused outright;
//! see `server::browser_refusal`.

pub mod oneshot;
pub mod protocol;
pub mod server;
pub mod state;

pub use server::serve;
