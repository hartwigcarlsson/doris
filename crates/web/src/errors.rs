//! Swedish messages for the API's stable error codes.

/// The text to show for a failed API call.
pub fn describe(status: &tonic::Status) -> String {
    message(status.message()).to_owned()
}

fn message(code: &str) -> &'static str {
    match code {
        "invalid_email" => "Ange en giltig e-postadress.",
        "invalid_display_name" => "Namnet måste vara 1–100 tecken.",
        "invalid_passkey_name" => "Passkeyns namn måste vara 1–64 tecken.",
        "invitation_required" => "Du behöver en inbjudan för att registrera dig.",
        "invitation_expired" => "Inbjudan har gått ut. Be om en ny.",
        "invitation_already_used" => "Inbjudan har redan använts.",
        "invitation_email_mismatch" => "E-postadressen matchar inte inbjudan.",
        "invitation_not_found" => "Inbjudan finns inte eller har redan använts.",
        "already_exists" => "E-postadressen är redan registrerad eller inbjuden.",
        "duplicate_passkey" => "Den här passkeyn är redan registrerad.",
        "ceremony_expired" => "Tiden gick ut. Försök igen.",
        "login_failed" => "Inloggningen misslyckades.",
        "credential_rejected" => "Passkeyn godkändes inte. Försök igen.",
        "not_signed_in" => "Du är inte inloggad.",
        "not_admin" => "Du saknar behörighet.",
        _ => "Något gick fel. Försök igen.",
    }
}

#[cfg(test)]
mod tests {
    use super::message;

    #[test]
    fn known_codes_have_their_own_message_and_unknown_ones_a_generic_one() {
        assert_eq!(message("login_failed"), "Inloggningen misslyckades.");
        assert_eq!(message("not_admin"), "Du saknar behörighet.");
        assert_eq!(message("internal"), "Något gick fel. Försök igen.");
        assert_eq!(message("something_new"), "Något gick fel. Försök igen.");
    }
}
