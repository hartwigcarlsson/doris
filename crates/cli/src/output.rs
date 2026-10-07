//! What a command prints: text for people, one JSON value with --json,
//! and an exit code that says what to fix.

use doris_proto::messages::message;
use serde_json::{Value, json};
use std::io::Write;

/// A command that did not succeed.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub code: String,
    pub message: String,
    pub exit: i32,
}

impl Failure {
    pub fn new(code: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message(code).to_owned(),
            exit: exit_code(code),
        }
    }

    /// A usage error with a message of its own (which flag, which value).
    pub fn usage(text: impl Into<String>) -> Self {
        Self {
            code: "usage".into(),
            message: text.into(),
            exit: 2,
        }
    }

    /// The server's refusal: its stable code, the shared Swedish text.
    /// Not reaching Doris is `connection_failed`; any other reply that is
    /// not a code is `internal`.
    pub fn from_status(status: &tonic::Status) -> Self {
        let message = status.message();
        let is_code = !message.is_empty()
            && message
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if is_code {
            return Self::new(message);
        }
        match status.code() {
            tonic::Code::Unavailable | tonic::Code::Unknown => Self::new("connection_failed"),
            _ => Self::new("internal"),
        }
    }
}

/// 1: the books refused; 2: fix the arguments; 3: fix the token or connection.
pub fn exit_code(code: &str) -> i32 {
    match code {
        "usage" | "company_ambiguous" => 2,
        "missing_token" | "missing_url" | "insecure_url" | "not_signed_in"
        | "connection_failed" => 3,
        _ => 1,
    }
}

/// Where a command writes, and how.
pub struct Output<'a> {
    pub json: bool,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

impl Output<'_> {
    /// Prints `value` with --json, else `text`.
    pub fn print(&mut self, value: Value, text: &str) {
        if self.json {
            let _ = writeln!(self.out, "{value}");
        } else {
            let _ = write!(self.out, "{text}");
        }
    }

    /// Prints a failure and returns its exit code.
    pub fn fail(&mut self, failure: &Failure) -> i32 {
        if self.json {
            let error = json!({ "error": { "code": failure.code, "message": failure.message } });
            let _ = writeln!(self.out, "{error}");
        } else {
            let _ = writeln!(self.err, "{}", failure.message);
        }
        failure.exit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_tell_what_to_fix() {
        for (code, exit) in [
            ("voucher_unbalanced", 1),
            ("fiscal_year_closed", 1),
            ("missing_scope", 1),
            ("company_not_found", 1),
            ("usage", 2),
            ("company_ambiguous", 2),
            ("missing_token", 3),
            ("missing_url", 3),
            ("insecure_url", 3),
            ("not_signed_in", 3),
            ("connection_failed", 3),
        ] {
            assert_eq!(exit_code(code), exit, "{code}");
        }
    }

    #[test]
    fn statuses_are_told_apart() {
        use tonic::{Code, Status};
        let internal = Failure::from_status(&Status::new(
            Code::ResourceExhausted,
            "message length too large",
        ));
        assert_eq!((internal.code.as_str(), internal.exit), ("internal", 1));
        let down = Failure::from_status(&Status::unavailable("error trying to connect"));
        assert_eq!((down.code.as_str(), down.exit), ("connection_failed", 3));
        let unknown = Failure::from_status(&Status::unknown(""));
        assert_eq!(unknown.code, "connection_failed");
        let refused = Failure::from_status(&Status::invalid_argument("voucher_unbalanced"));
        assert_eq!(
            (refused.code.as_str(), refused.exit),
            ("voucher_unbalanced", 1)
        );
    }
}
