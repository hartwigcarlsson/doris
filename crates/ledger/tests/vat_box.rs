use doris_ledger::domain::DomainError;
use doris_ledger::vat_box::{BOXES, Side, VatBox, default_vat_box};

fn b(n: u32) -> VatBox {
    VatBox::parse(n).unwrap()
}

#[test]
fn only_the_forms_boxes_parse_and_49_is_computed() {
    for n in [5, 8, 10, 24, 42, 48, 50, 62] {
        assert_eq!(b(n).get() as u32, n);
    }
    for n in [0, 1, 9, 13, 25, 49, 63, 300] {
        assert_eq!(VatBox::parse(n), Err(DomainError::InvalidVatBox), "{n}");
    }
    assert_eq!(BOXES.len(), 28);
}

#[test]
fn boxes_know_their_side_and_whether_they_carry_vat() {
    for n in [20, 21, 22, 23, 24, 48, 50] {
        assert_eq!(b(n).side(), Side::Debit, "{n}");
    }
    for n in [5, 6, 7, 8, 10, 11, 12, 30, 31, 32, 35, 39, 42, 60, 61, 62] {
        assert_eq!(b(n).side(), Side::Credit, "{n}");
    }
    let vat: Vec<u8> = BOXES.iter().copied().filter(|&n| b(n.into()).is_vat()).collect();
    assert_eq!(vat, [10, 11, 12, 30, 31, 32, 60, 61, 62, 48]);
}

#[test]
fn bas_accounts_have_their_boxes() {
    let cases = [
        (3001, Some(5)), (3002, Some(5)), (3003, Some(5)), (3106, Some(5)),
        (3004, Some(42)), (3108, Some(35)), (3305, Some(40)), (3308, Some(39)),
        (2611, Some(10)), (2621, Some(11)), (2631, Some(12)),
        (4515, Some(20)), (4535, Some(21)), (4531, Some(22)), (4415, Some(23)), (4425, Some(24)),
        (2614, Some(30)), (2624, Some(31)), (2634, Some(32)),
        (4545, Some(50)), (2615, Some(60)), (2625, Some(61)), (2635, Some(62)),
        (2640, Some(48)), (2641, Some(48)), (2645, Some(48)), (2647, Some(48)),
        (2650, None), (3740, None), (1930, None), (4010, None),
    ];
    for (account, expected) in cases {
        assert_eq!(default_vat_box(account).map(VatBox::get), expected, "{account}");
    }
}

#[test]
fn a_box_is_stored_as_its_number() {
    assert_eq!(serde_json::to_string(&b(10)).unwrap(), "10");
    assert_eq!(serde_json::from_str::<VatBox>("48").unwrap(), b(48));
    assert!(serde_json::from_str::<VatBox>("49").is_err());
}
