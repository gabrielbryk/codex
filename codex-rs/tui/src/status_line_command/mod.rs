//! Local external status-line command support.
//!
//! This module deliberately separates the serialized formatter contract, output
//! parsing, and async request ownership. Process spawning is supplied by the
//! platform process-tree runner so remote workspace execution can never leak
//! into this local-only path.

pub(crate) mod parser;
pub(crate) mod process;
pub(crate) mod runner;
pub(crate) mod wire;
