//! Companies a user keeps the books for, event-sourced into SQLite.
//!
//! Every write runs in one IMMEDIATE transaction: load state, decide, append,
//! project. A UNIQUE violation in a projection rolls the whole write back.

pub mod domain;
