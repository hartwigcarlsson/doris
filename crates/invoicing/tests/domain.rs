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

fn customer_form(name: &str) -> CustomerForm<'_> {
    CustomerForm {
        name,
        org_nr: "556016-0680",
        vat_number: "",
        street: "Storgatan 1",
        postal_code: "111 22",
        city: "Stockholm",
        email: "",
        payment_terms: 30,
    }
}

fn customer(name: &str) -> CustomerDetails {
    CustomerDetails::parse(&customer_form(name)).unwrap()
}

fn supplier_form(name: &str) -> SupplierForm<'_> {
    SupplierForm {
        name,
        org_nr: "",
        vat_number: "",
        street: "",
        postal_code: "",
        city: "",
        email: "",
        bankgiro: "",
        plusgiro: "",
        iban: "",
        bic: "",
    }
}

#[test]
fn customer_details_parse_every_field() {
    let details = CustomerDetails::parse(&CustomerForm {
        email: "Ekonomi@Kund.se",
        vat_number: "SE556016068001",
        ..customer_form(" Kund AB ")
    })
    .unwrap();
    assert_eq!(details.name.as_str(), "Kund AB");
    assert_eq!(details.org_nr.unwrap().as_str(), "5560160680");
    assert_eq!(details.vat_number.unwrap().as_str(), "SE556016068001");
    assert_eq!(details.address.city.as_deref(), Some("Stockholm"));
    assert_eq!(details.email.unwrap().as_str(), "ekonomi@kund.se");
    assert_eq!(details.payment_terms.get(), 30);
}

#[test]
fn blank_optional_fields_are_none() {
    let details = CustomerDetails::parse(&CustomerForm {
        org_nr: "  ",
        vat_number: " ",
        street: " ",
        postal_code: "",
        city: "",
        email: "  ",
        ..customer_form("Privatperson")
    })
    .unwrap();
    assert_eq!(details.org_nr, None);
    assert_eq!(details.vat_number, None);
    assert_eq!(details.address.street, None);
    assert_eq!(details.email, None);

    let supplier = SupplierDetails::parse(&SupplierForm {
        bankgiro: " ",
        ..supplier_form("Leverantör AB")
    })
    .unwrap();
    assert_eq!((supplier.bankgiro, supplier.iban), (None, None));
}

#[test]
fn each_bad_field_gives_its_own_error() {
    let bad = |form: CustomerForm| CustomerDetails::parse(&form).unwrap_err();
    let long = "å".repeat(201);
    assert_eq!(bad(customer_form("")), DomainError::InvalidName);
    for (form, error) in [
        (
            CustomerForm {
                org_nr: "556016-0681",
                ..customer_form("K")
            },
            DomainError::InvalidOrgNr,
        ),
        (
            CustomerForm {
                vat_number: "SE1",
                ..customer_form("K")
            },
            DomainError::InvalidVatNumber,
        ),
        (
            CustomerForm {
                city: &long,
                ..customer_form("K")
            },
            DomainError::InvalidAddress,
        ),
        (
            CustomerForm {
                email: "kund",
                ..customer_form("K")
            },
            DomainError::InvalidEmail,
        ),
        (
            CustomerForm {
                payment_terms: 366,
                ..customer_form("K")
            },
            DomainError::InvalidPaymentTerms,
        ),
    ] {
        assert_eq!(bad(form), error);
    }

    let supplier = |form: SupplierForm| SupplierDetails::parse(&form).unwrap_err();
    for (form, error) in [
        (
            SupplierForm {
                bankgiro: "1",
                ..supplier_form("L")
            },
            DomainError::InvalidBankgiro,
        ),
        (
            SupplierForm {
                plusgiro: "1",
                ..supplier_form("L")
            },
            DomainError::InvalidPlusgiro,
        ),
        (
            SupplierForm {
                iban: "SE1",
                ..supplier_form("L")
            },
            DomainError::InvalidIban,
        ),
        (
            SupplierForm {
                bic: "X",
                ..supplier_form("L")
            },
            DomainError::InvalidBic,
        ),
    ] {
        assert_eq!(supplier(form), error);
    }
}

#[test]
fn numbers_run_1_to_n() {
    let mut register = Register::default();
    for expected in 1..=3 {
        let changes = add(&register, customer("Kund")).unwrap();
        assert_eq!(changes[0].number(), expected);
        changes.into_iter().for_each(|c| register.apply(c));
    }
    assert_eq!(
        register.parties().map(|p| p.number).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[test]
fn an_update_replaces_every_detail_and_the_same_details_are_a_no_op() {
    let register = Register::from_changes([Change::Added {
        number: 1,
        details: customer("Gamla AB"),
    }]);
    assert_eq!(
        update(&register, 1, customer("Nya AB")).unwrap(),
        [Change::Updated {
            number: 1,
            details: customer("Nya AB")
        }]
    );
    assert_eq!(update(&register, 1, customer("Gamla AB")).unwrap(), []);
}

#[test]
fn deactivating_and_reactivating_are_idempotent() {
    let mut register = Register::from_changes([Change::Added {
        number: 1,
        details: customer("K"),
    }]);
    assert_eq!(set_active(&register, 1, true).unwrap(), []);
    let off = set_active(&register, 1, false).unwrap();
    assert_eq!(off, [Change::Deactivated { number: 1 }]);
    off.into_iter().for_each(|c| register.apply(c));
    assert!(!register.get(1).unwrap().active);
    assert_eq!(set_active(&register, 1, false).unwrap(), []);
    assert_eq!(
        set_active(&register, 1, true).unwrap(),
        [Change::Reactivated { number: 1 }]
    );
}

#[test]
fn unknown_numbers_are_not_found_per_register() {
    let customers: Register<CustomerDetails> = Register::default();
    assert_eq!(
        update(&customers, 7, customer("K")),
        Err(DomainError::CustomerNotFound)
    );
    assert_eq!(
        set_active(&customers, 7, false),
        Err(DomainError::CustomerNotFound)
    );
    let suppliers: Register<SupplierDetails> = Register::default();
    assert_eq!(
        set_active(&suppliers, 7, false),
        Err(DomainError::SupplierNotFound)
    );
}

#[test]
fn stored_events_read_as_customer_and_supplier_events() {
    let event = CustomerEvent::from(Change::Added {
        number: 1,
        details: customer("K"),
    });
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "CustomerAdded");
    assert_eq!(json["number"], 1);
    assert_eq!(json["details"]["org_nr"], "5560160680");
    let back: Change<CustomerDetails> = serde_json::from_value::<CustomerEvent>(json)
        .unwrap()
        .into();
    assert_eq!(
        back,
        Change::Added {
            number: 1,
            details: customer("K")
        }
    );
    let event = SupplierEvent::from(Change::<SupplierDetails>::Deactivated { number: 2 });
    assert_eq!(
        serde_json::to_value(&event).unwrap()["type"],
        "SupplierDeactivated"
    );
}
