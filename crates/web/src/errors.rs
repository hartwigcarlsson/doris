//! Swedish messages for the API's stable error codes (the table lives in
//! `doris_proto::messages`, shared with doris-cli).

use doris_proto::messages::message;

/// The text to show for a failed API call.
pub fn describe(status: &tonic::Status) -> String {
    message(status.message()).to_owned()
}

/// The text for an error code found in the browser, before any API call.
pub fn describe_code(code: &str) -> String {
    message(code).to_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn codes_come_from_the_shared_table() {
        assert_eq!(
            super::describe_code("not_signed_in"),
            "Du är inte inloggad."
        );
    }
}
