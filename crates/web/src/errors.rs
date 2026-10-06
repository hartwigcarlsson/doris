//! Swedish messages for the API's stable error codes.

/// The text to show for a failed API call.
pub fn describe(status: &tonic::Status) -> String {
    message(status.message()).to_owned()
}

/// The text for an error code found in the browser, before any API call.
pub fn describe_code(code: &str) -> String {
    message(code).to_owned()
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
        "invalid_tax_table" => "Välj tabell 29–42 och kolumn 1–6.",
        "invalid_tax_percent" => "Procentsatsen måste vara 0–100.",
        "tax_required" => "Ange skatt eller en skatteinställning för den anställda.",
        "tax_table_unavailable" => {
            "Skattetabellen kunde inte hämtas från Skatteverket. Försök igen senare eller skriv in skatten för hand."
        }
        "invalid_account_number" => "Kontonumret ska vara fyra siffror, 1000–8999.",
        "invalid_account_name" => "Kontonamnet måste vara 1–100 tecken.",
        "account_exists" => "Kontot finns redan i kontoplanen.",
        "account_not_found" => "Kontot finns inte i kontoplanen.",
        "account_inactive" => {
            "Kontot är inaktivt. Aktivera det i kontoplanen eller välj ett annat."
        }
        "invalid_token_name" => "Namnet måste vara 1–100 tecken.",
        "invalid_token_expiry" => "Välj en sista giltig dag från i dag och högst ett år fram.",
        "invalid_token_grants" => "Ge token minst en behörighet.",
        "api_token_not_found" => "Token finns inte.",
        "missing_scope" => "Token saknar behörighet för det här.",
        "token_not_allowed" => "Det här kan inte göras med en token.",
        "invalid_voucher_text" => "Texten måste vara 1–200 tecken.",
        "invalid_voucher_lines" => "En verifikation ska ha 2–100 rader.",
        "invalid_amount" => "Varje rad ska ha ett belopp i antingen debet eller kredit.",
        "voucher_unbalanced" => "Debet och kredit måste vara lika stora.",
        "voucher_date_in_future" => "Datumet kan inte vara i framtiden.",
        "voucher_date_before_first_fiscal_year" => {
            "Datumet ligger före företagets första räkenskapsår."
        }
        "correction_date_outside_fiscal_year" => {
            "Rättelsen ska dateras inom samma räkenskapsår som verifikationen."
        }
        "voucher_not_found" => "Verifikationen finns inte.",
        "already_corrected" => "Verifikationen är redan rättad.",
        "cannot_correct_correction" => {
            "En rättelse kan inte rättas. Bokför en ny verifikation i stället."
        }
        "not_balance_sheet_account" => {
            "Ingående balanser får bara finnas på balanskonton, 1000–2999."
        }
        "duplicate_account" => "Varje konto får bara förekomma en gång.",
        "opening_balances_unbalanced" => {
            "De ingående balanserna måste balansera: debet och kredit ska vara lika stora."
        }
        "invalid_reason" => "Anledningen måste vara 1–200 tecken.",
        "fiscal_year_not_found" => "Räkenskapsåret finns inte.",
        "fiscal_year_closed" => {
            "Räkenskapsåret är stängt. Bokför i ett öppet år eller öppna året igen."
        }
        "fiscal_year_open" => "Räkenskapsåret är redan öppet.",
        "fiscal_year_not_ended" => "Räkenskapsåret är inte slut än.",
        "previous_fiscal_year_open" => "Stäng föregående räkenskapsår först.",
        "later_fiscal_year_closed" => "Öppna det senare räkenskapsåret först.",
        "invalid_date" => "Ange ett giltigt datum.",
        "unsupported_attachment_type" => "Underlaget måste vara en PDF, JPEG eller PNG.",
        "invalid_attachment_name" => "Filnamnet är ogiltigt.",
        "empty_attachment" => "Filen är tom.",
        "attachment_too_large" => "Underlaget är för stort (högst 10 MB per fil och 20 MB totalt).",
        "duplicate_attachment" => "Underlaget finns redan på verifikationen.",
        "attachment_not_found" => "Underlaget hittades inte.",
        "popup_blocked" => {
            "Webbläsaren blockerade det nya fönstret. Tillåt popup-fönster för Doris och försök igen."
        }
        "invalid_personal_identity_number" => "Personnumret är ogiltigt (ÅÅÅÅMMDD-NNNN).",
        "invalid_employee_name" => "Ange ett namn (högst 100 tecken).",
        "invalid_salary" => "Lönen måste vara större än noll.",
        "invalid_salary_account" => "Välj ett lönekonto.",
        "invalid_tax" => "Skatten får inte vara negativ eller större än bruttolönen.",
        "empty_payroll_run" => "Välj minst en anställd.",
        "duplicate_payroll_run_line" => "Samma anställd finns två gånger i körningen.",
        "duplicate_employee" => "Det finns redan en anställd med det personnumret.",
        "employee_inactive" => "Den anställda är inaktiverad.",
        "employee_not_found" => "Den anställda hittades inte.",
        "payroll_run_not_found" => "Lönekörningen hittades inte.",
        "payroll_run_not_open" => "Lönekörningen är färdigställd. Öppna den för att ändra.",
        "payroll_run_not_finalized" => "Lönekörningen är inte färdigställd.",
        "payroll_run_booked" => "Lönekörningen är bokförd. Backa bokföringen först.",
        "payroll_run_not_booked" => "Lönekörningen är inte bokförd.",
        "payroll_run_not_due" => "Lönekörningen kan inte bokföras före utbetalningsdagen.",
        "payroll_run_outdated" => {
            "Avgifterna har ändrats sedan körningen färdigställdes. Öppna och färdigställ den igen."
        }
        "invalid_name" => "Namnet måste vara 1–200 tecken.",
        "invalid_vat_number" => {
            "Ange ett giltigt momsregistreringsnummer, till exempel SE556016068001."
        }
        "invalid_payment_terms" => "Betalningsvillkoret ska vara 0–365 dagar.",
        "invalid_bankgiro" => "Ange ett giltigt bankgironummer (7–8 siffror).",
        "invalid_plusgiro" => "Ange ett giltigt plusgironummer (2–8 siffror).",
        "invalid_iban" => "Ange ett giltigt IBAN-nummer.",
        "invalid_bic" => "Ange en giltig BIC (8 eller 11 tecken).",
        "customer_not_found" => "Kunden finns inte.",
        "supplier_not_found" => "Leverantören finns inte.",
        "invalid_period" => "Ogiltig period.",
        "invalid_agi_contact" => {
            "Ange namn (högst 50 tecken), telefon (högst 20 tecken) och en giltig e-postadress."
        }
        "agi_contact_missing" => "Spara en kontaktperson först.",
        "agi_period_empty" => "Det finns inga bokförda löner den månaden.",
        "agi_unchanged" => "Månaden är redan inlämnad och har inte ändrats.",
        "agi_file_outdated" => "Filen är inaktuell, ladda ner den igen.",
        "supplier_invoice_not_found" => "Leverantörsfakturan finns inte.",
        "supplier_inactive" => "Leverantören är inaktiv. Aktivera den eller välj en annan.",
        "invalid_invoice_number" => "Fakturanumret måste vara 1–50 tecken.",
        "duplicate_supplier_invoice" => "Den här fakturan från leverantören är redan registrerad.",
        "invalid_due_date" => "Förfallodatumet kan inte vara före fakturadatumet.",
        "invalid_reference" => "OCR/meddelande får vara högst 50 tecken.",
        "invalid_invoice_lines" => "En faktura ska ha 1–50 rader med belopp över noll.",
        "invalid_vat_rate" => "Momssatsen ska vara 25, 12, 6 eller 0 %.",
        "invalid_vat_amount" => "Momsen får skilja högst 1 kr från den uträknade.",
        "invalid_payment_account" => "Betalkontot ska vara ett konto i 1900–1999.",
        "supplier_invoice_paid" => "Fakturan är redan betald.",
        "supplier_invoice_not_paid" => "Fakturan är inte betald.",
        "supplier_invoice_cancelled" => "Fakturan är makulerad.",
        "invalid_invoice_account" => {
            "Raderna kan inte bokföras på reskontrakontot (1510/2440) eller ett momskonto."
        }
        "customer_invoice_not_found" => "Kundfakturan finns inte.",
        "customer_inactive" => "Kunden är inaktiv. Aktivera den eller välj en annan.",
        "duplicate_customer_invoice" => {
            "Fakturanumret är redan använt. Ett utfärdat nummer återanvänds aldrig, inte heller efter makulering."
        }
        "customer_invoice_paid" => "Fakturan är redan betald.",
        "customer_invoice_not_paid" => "Fakturan är inte betald.",
        "customer_invoice_cancelled" => "Fakturan är makulerad.",
        "invalid_vat_box" => "Välj en ruta som finns på momsdeklarationen.",
        "invalid_vat_period" => "Den redovisningsperioden finns inte.",
        "vat_period_not_ended" => "Perioden har inte tagit slut än.",
        "vat_period_locked" => {
            "Redovisningsperioden kan inte ändras när en deklaration för året är inlämnad."
        }
        "vat_return_outdated" => {
            "Bokföringen har ändrats sedan deklarationen visades. Ladda om sidan och kontrollera den igen."
        }
        "vat_return_unchanged" => "Perioden är redan inlämnad och har inte ändrats.",
        "vat_not_registered" => "Företaget är inte momsregistrerat det räkenskapsåret.",
        _ => "Något gick fel. Försök igen.",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn api_token_codes_have_swedish_messages() {
        for code in [
            "invalid_token_name",
            "invalid_token_expiry",
            "invalid_token_grants",
            "api_token_not_found",
            "missing_scope",
            "token_not_allowed",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
    }

    #[test]
    fn agi_codes_have_swedish_messages() {
        assert_eq!(message("invalid_period"), "Ogiltig period.");
        assert_eq!(
            message("invalid_agi_contact"),
            "Ange namn (högst 50 tecken), telefon (högst 20 tecken) och en giltig e-postadress."
        );
        assert_eq!(
            message("agi_contact_missing"),
            "Spara en kontaktperson först."
        );
        assert_eq!(
            message("agi_period_empty"),
            "Det finns inga bokförda löner den månaden."
        );
        assert_eq!(
            message("agi_unchanged"),
            "Månaden är redan inlämnad och har inte ändrats."
        );
        assert_eq!(
            message("agi_file_outdated"),
            "Filen är inaktuell, ladda ner den igen."
        );
    }

    #[test]
    fn tax_table_codes_have_swedish_messages() {
        assert_eq!(
            message("invalid_tax_table"),
            "Välj tabell 29–42 och kolumn 1–6."
        );
        assert_eq!(
            message("invalid_tax_percent"),
            "Procentsatsen måste vara 0–100."
        );
        assert_eq!(
            message("tax_required"),
            "Ange skatt eller en skatteinställning för den anställda."
        );
        assert_eq!(
            message("tax_table_unavailable"),
            "Skattetabellen kunde inte hämtas från Skatteverket. Försök igen senare eller skriv in skatten för hand."
        );
    }

    use super::message;
    #[test]
    fn closing_codes_have_swedish_messages() {
        for code in [
            "not_balance_sheet_account",
            "duplicate_account",
            "opening_balances_unbalanced",
            "invalid_reason",
            "fiscal_year_not_found",
            "fiscal_year_closed",
            "fiscal_year_open",
            "fiscal_year_not_ended",
            "previous_fiscal_year_open",
            "later_fiscal_year_closed",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(
            message("fiscal_year_closed"),
            "Räkenskapsåret är stängt. Bokför i ett öppet år eller öppna året igen."
        );
    }
    #[test]
    fn ledger_codes_have_swedish_messages() {
        for code in [
            "invalid_account_number",
            "invalid_account_name",
            "account_exists",
            "account_not_found",
            "account_inactive",
            "invalid_voucher_text",
            "invalid_voucher_lines",
            "invalid_amount",
            "voucher_unbalanced",
            "voucher_date_in_future",
            "voucher_date_before_first_fiscal_year",
            "correction_date_outside_fiscal_year",
            "voucher_not_found",
            "already_corrected",
            "cannot_correct_correction",
            "invalid_date",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(
            message("invalid_voucher_lines"),
            "En verifikation ska ha 2–100 rader."
        );
    }

    #[test]
    fn invoicing_codes_have_swedish_messages() {
        for code in [
            "invalid_name",
            "invalid_vat_number",
            "invalid_payment_terms",
            "invalid_bankgiro",
            "invalid_plusgiro",
            "invalid_iban",
            "invalid_bic",
            "customer_not_found",
            "supplier_not_found",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(
            message("invalid_payment_terms"),
            "Betalningsvillkoret ska vara 0–365 dagar."
        );
    }

    #[test]
    fn supplier_invoice_codes_have_swedish_messages() {
        for code in [
            "supplier_invoice_not_found",
            "supplier_inactive",
            "invalid_invoice_number",
            "duplicate_supplier_invoice",
            "invalid_due_date",
            "invalid_reference",
            "invalid_invoice_lines",
            "invalid_vat_rate",
            "invalid_vat_amount",
            "invalid_invoice_account",
            "invalid_payment_account",
            "supplier_invoice_paid",
            "supplier_invoice_not_paid",
            "supplier_invoice_cancelled",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
    }

    #[test]
    fn customer_invoice_codes_have_swedish_messages() {
        for code in [
            "customer_invoice_not_found",
            "customer_inactive",
            "duplicate_customer_invoice",
            "customer_invoice_paid",
            "customer_invoice_not_paid",
            "customer_invoice_cancelled",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(
            message("invalid_invoice_account"),
            "Raderna kan inte bokföras på reskontrakontot (1510/2440) eller ett momskonto."
        );
    }

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

    #[test]
    fn attachment_codes_have_swedish_messages() {
        for code in [
            "unsupported_attachment_type",
            "invalid_attachment_name",
            "empty_attachment",
            "attachment_too_large",
            "duplicate_attachment",
            "attachment_not_found",
            "popup_blocked",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(
            message("unsupported_attachment_type"),
            "Underlaget måste vara en PDF, JPEG eller PNG."
        );
    }

    #[test]
    fn payroll_codes_have_swedish_messages() {
        for code in [
            "invalid_personal_identity_number",
            "invalid_employee_name",
            "invalid_salary",
            "invalid_salary_account",
            "invalid_tax",
            "empty_payroll_run",
            "duplicate_payroll_run_line",
            "duplicate_employee",
            "employee_inactive",
            "employee_not_found",
            "payroll_run_not_found",
            "payroll_run_not_open",
            "payroll_run_not_finalized",
            "payroll_run_booked",
            "payroll_run_not_booked",
            "payroll_run_not_due",
            "payroll_run_outdated",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(
            message("payroll_run_not_due"),
            "Lönekörningen kan inte bokföras före utbetalningsdagen."
        );
    }

    #[test]
    fn vat_codes_have_swedish_messages() {
        for code in [
            "invalid_vat_box",
            "invalid_vat_period",
            "vat_period_not_ended",
            "vat_period_locked",
            "vat_return_outdated",
            "vat_return_unchanged",
            "vat_not_registered",
        ] {
            assert_ne!(message(code), message("something_else"), "{code}");
        }
    }
}
