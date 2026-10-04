//! Employees and payroll runs (lönekörningar) of a company, event-sourced
//! into SQLite. A run is booked as a ledger voucher in the same
//! transaction as its event.

pub mod domain;
