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
    /// Extra fields for the JSON error object.
    pub details: Option<Value>,
}

impl Failure {
    pub fn new(code: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message(code).to_owned(),
            exit: exit_code(code),
            details: None,
        }
    }

    /// A usage error with a message of its own (which flag, which value).
    pub fn usage(text: impl Into<String>) -> Self {
        Self {
            code: "usage".into(),
            message: text.into(),
            exit: 2,
            details: None,
        }
    }

    /// `{"error":{"code","message",…details}}`, as `--json` prints it.
    pub fn to_json(&self) -> Value {
        let mut error = json!({ "code": self.code, "message": self.message });
        if let (Some(Value::Object(extra)), Some(object)) = (&self.details, error.as_object_mut()) {
            object.extend(extra.clone());
        }
        json!({ "error": error })
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
        "missing_token" | "missing_url" | "insecure_url" | "bad_url" | "not_signed_in"
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
            let _ = writeln!(self.out, "{}", failure.to_json());
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
            ("bad_url", 3),
            ("dry_run_unsupported", 1),
        ] {
            assert_eq!(exit_code(code), exit, "{code}");
        }
    }

    #[test]
    fn details_join_the_json_error() {
        let mut f = Failure::new("dry_run_unsupported");
        f.details = Some(json!({"number": 3}));
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut o = Output {
            json: true,
            out: &mut out,
            err: &mut err,
        };
        assert_eq!(o.fail(&f), 1);
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["error"]["number"], 3);
        assert_eq!(v["error"]["code"], "dry_run_unsupported");
    }

    #[test]
    fn a_failure_as_json_carries_code_message_and_details() {
        let mut f = Failure::new("dry_run_unsupported");
        f.details = Some(json!({"number": 3}));

        let v = f.to_json();

        assert_eq!(v["error"]["code"], "dry_run_unsupported");
        assert_eq!(v["error"]["message"], f.message.as_str());
        assert_eq!(v["error"]["number"], 3);
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
