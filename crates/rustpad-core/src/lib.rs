//! Toolkit-independent core of RustPad.
//!
//! Nothing in here knows about GTK. The GUI crate drives these modules and
//! stays a thin layer, which keeps documents, recovery storage, configuration
//! and theme resolution unit-testable on their own.

pub mod config;
pub mod desktop;
pub mod files;
pub mod storage;
pub mod watch;
