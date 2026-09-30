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
        "invalid_org_nr" => "Ange ett giltigt organisationsnummer (10 siffror).",
        "invalid_company_name" => "Företagsnamnet måste vara 1–200 tecken.",
        "invalid_address" => "Adressfälten får vara högst 200 tecken.",
        "invalid_legal_form" => "Välj juridisk form.",
        "invalid_accounting_method" => "Välj bokföringsmetod.",
        "invalid_fiscal_year" => {
            "Räkenskapsåret ska börja den 1:a och sluta sista dagen i en månad, vara högst 18 månader, och för enskild firma och handelsbolag sluta 31 december."
        }
        "company_exists" => {
            "Företaget finns redan i Doris. Be någon som har tillgång att lägga till dig."
        }
        "company_not_found" => "Företaget finns inte eller så saknar du tillgång.",
        "user_not_found" => "Det finns ingen användare med den e-postadressen.",
        "lookup_unavailable" => {
            "Hämtning från Bolagsverket är inte konfigurerad. Fyll i uppgifterna själv."
        }
        "lookup_personal_number" => {
            "Enskilda firmor hämtas inte från Bolagsverket. Fyll i uppgifterna själv."
        }
        "lookup_not_found" => "Bolagsverket hittade inget företag med det numret.",
        "lookup_failed" => "Bolagsverket svarade inte. Försök igen eller fyll i uppgifterna själv.",
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

    #[test]
    fn company_codes_have_swedish_messages() {
        for code in [
            "invalid_org_nr",
            "invalid_company_name",
            "invalid_address",
            "invalid_legal_form",
            "invalid_accounting_method",
            "invalid_fiscal_year",
            "company_exists",
            "company_not_found",
            "user_not_found",
            "lookup_unavailable",
            "lookup_personal_number",
            "lookup_not_found",
            "lookup_failed",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
    }
}
