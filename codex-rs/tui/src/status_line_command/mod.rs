//! Local external status-line command support.
//!
//! This module deliberately separates the serialized formatter contract, output
//! parsing, and async request ownership. Process spawning is supplied by the
//! local contained-process runner so remote workspace execution can never leak
//! into this local-only path. Cleanup strength is platform-specific: a Windows
//! Job Object kills the whole tree, while Unix terminates the formatter's
//! process group on a best-effort basis (a formatter that calls `setsid()` can
//! escape; daemonizing formatters are unsupported).

pub(crate) mod parser;
pub(crate) mod process;
pub(crate) mod runner;
pub(crate) mod wire;
