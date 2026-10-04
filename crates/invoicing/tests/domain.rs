use doris_invoicing::domain::*;

#[test]
fn names_are_trimmed_and_1_to_200_characters() {
    assert_eq!(PartyName::parse("  Kund AB ").unwrap().as_str(), "Kund AB");
    assert!(PartyName::parse(&"å".repeat(200)).is_ok());
    for bad in ["", "  ", &"å".repeat(201)] {
        assert_eq!(PartyName::parse(bad), Err(DomainError::InvalidName));
    }
}

#[test]
fn emails_are_lowercased_and_need_a_local_part_and_a_domain() {
    assert_eq!(
        Email::parse(" Ekonomi@Kund.SE ").unwrap().as_str(),
        "ekonomi@kund.se"
    );
    for bad in [
        "kund.se",
        "@kund.se",
        "a@b@kund.se",
        "a@kund",
        "a b@kund.se",
    ] {
        assert_eq!(Email::parse(bad), Err(DomainError::InvalidEmail), "{bad}");
    }
    assert_eq!(
        format!("{:?}", Email::parse("a@kund.se").unwrap()),
        "Email(<redacted>)"
    );
}

#[test]
fn vat_numbers_are_two_letters_and_2_to_12_characters_and_swedish_ones_end_in_01() {
    assert_eq!(
        VatNumber::parse("SE556016068001").unwrap().as_str(),
        "SE556016068001"
    );
    assert_eq!(
        VatNumber::parse("de 123456789").unwrap().as_str(),
        "DE123456789"
    );
    for bad in [
        "SE5560160680",   // no 01
        "SE556016068101", // bad Luhn
        "SE55601606800",  // too short
        "S1234",
        "DE1",
        "DE1234567890123",
        "DE12_34",
    ] {
        assert_eq!(
            VatNumber::parse(bad),
            Err(DomainError::InvalidVatNumber),
            "{bad}"
        );
    }
}

#[test]
fn payment_terms_are_0_to_365_days() {
    assert_eq!(PaymentTerms::new(0).unwrap().get(), 0);
    assert_eq!(PaymentTerms::new(365).unwrap().get(), 365);
    assert_eq!(
        PaymentTerms::new(366),
        Err(DomainError::InvalidPaymentTerms)
    );
}

#[test]
fn bankgiro_is_7_or_8_digits_with_a_luhn_check_digit() {
    let bg = Bankgiro::parse("5050-1055").unwrap();
    assert_eq!(
        (bg.as_str(), bg.formatted().as_str()),
        ("50501055", "5050-1055")
    );
    assert_eq!(Bankgiro::parse("1234566").unwrap().formatted(), "123-4566");
    for bad in ["5050-1056", "123456", "123456789", "12a4566"] {
        assert_eq!(
            Bankgiro::parse(bad),
            Err(DomainError::InvalidBankgiro),
            "{bad}"
        );
    }
}

#[test]
fn plusgiro_is_2_to_8_digits_with_a_luhn_check_digit() {
    assert_eq!(Plusgiro::parse("1-8").unwrap().formatted(), "1-8");
    assert_eq!(
        Plusgiro::parse("12 34 56 7-4").unwrap().formatted(),
        "1234567-4"
    );
    for bad in ["1-9", "8", "123456789"] {
        assert_eq!(
            Plusgiro::parse(bad),
            Err(DomainError::InvalidPlusgiro),
            "{bad}"
        );
    }
}

#[test]
fn iban_passes_the_mod_97_check() {
    let iban = Iban::parse("SE45 5000 0000 0583 9825 7466").unwrap();
    assert_eq!(iban.as_str(), "SE4550000000058398257466");
    assert_eq!(iban.formatted(), "SE45 5000 0000 0583 9825 7466");
    for bad in [
        "SE46 5000 0000 0583 9825 7466",
        "SE45",
        "1145 5000 0000 0583 9825 7466",
    ] {
        assert_eq!(Iban::parse(bad), Err(DomainError::InvalidIban), "{bad}");
    }
}

#[test]
fn bic_is_8_or_11_characters() {
    assert_eq!(Bic::parse("ESSESESS").unwrap().as_str(), "ESSESESS");
    assert_eq!(Bic::parse("ESSESESSXXX").unwrap().as_str(), "ESSESESSXXX");
    for bad in ["ESSESES", "ESSESESSX", "1SSESESS", "ESSE1ESS"] {
        assert_eq!(Bic::parse(bad), Err(DomainError::InvalidBic), "{bad}");
    }
}

#[test]
fn values_are_normalised_and_non_ascii_never_panics() {
    assert!(Iban::parse("se45 5000 0000 0583 9825 7466").is_ok());
    assert_eq!(Bic::parse(" essesess ").unwrap().as_str(), "ESSESESS");
    assert_eq!(
        VatNumber::parse("SE 556016-0680 01").unwrap().as_str(),
        "SE556016068001"
    );
    assert_eq!(
        Iban::parse("SÉ45 5000 0000 0583 9825 7466"),
        Err(DomainError::InvalidIban)
    );
    assert_eq!(Bic::parse("ÉSSESESS"), Err(DomainError::InvalidBic));
    assert_eq!(
        VatNumber::parse("É1234"),
        Err(DomainError::InvalidVatNumber)
    );
    assert_eq!(
        Bankgiro::parse("5050-105å"),
        Err(DomainError::InvalidBankgiro)
    );
}
