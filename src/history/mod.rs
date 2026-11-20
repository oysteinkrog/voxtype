//! Dictation history (#209): a record of every dictation, kept as plain
//! JSONL day files.
//!
//! - [`entry`]: what one dictation records, one JSON line.
//! - [`store`]: the day files, their permissions, atomic rewrites,
//!   compression, retention and the entry cap.
//!
//! The file format and the command names follow upstream voxtype v1.3.1.
//! Its search index, focused-app lookup and rule suggestions are not ported.

pub mod entry;
pub mod store;

pub use entry::{edits_between, Edit, EndedBy, HistoryEntry, Outcome, OutputInfo, Trigger};
pub use store::{DayFile, HistoryStore};
