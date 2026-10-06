use doris_ledger::vat_box::VatBox;
use doris_vat::domain::{AccountSaldo, booked_vat, boxes, fingerprint};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn a(account: u16, vat_box: u32, saldo: i64) -> AccountSaldo {
    AccountSaldo { account, vat_box: VatBox::parse(vat_box).unwrap(), saldo }
}

#[test]
fn boxes_count_sales_as_credits_and_purchases_as_debits_in_whole_kronor() {
    let accounts = [
        a(2611, 10, -9_500_037),   // output VAT 25 %
        a(2640, 48, 2_461_099),    // input VAT
        a(3001, 5, -38_000_000),
        a(3002, 5, -3_230_040),
        a(4535, 21, 480_000),      // EU service purchase
        a(2614, 30, -120_000),     // its output VAT
        a(2645, 48, 120_000),      // and its input VAT
    ];
    let b = boxes(&accounts);
    assert_eq!(
        [b.get(5), b.get(10), b.get(21), b.get(30), b.get(48)],
        [412_300, 95_000, 4_800, 1_200, 25_810]
    );
    // 49 from the rounded boxes: 95 000 + 1 200 − 25 810.
    assert_eq!(b.vat_due, 70_390);
    assert_eq!(b.get(6), 0);
    assert!(b.amounts.iter().all(|(_, kr)| *kr != 0));
    // Exactly booked: 95 000,37 + 1 200,00 − 25 810,99 = 70 389,38.
    assert_eq!(booked_vat(&accounts), 7_038_938);
}

#[test]
fn ore_are_struck_off_toward_zero_also_for_negative_boxes() {
    // More credited than sold: a debit saldo on sales and output VAT.
    let b = boxes(&[a(3001, 5, 100_099), a(2611, 10, 25_075)]);
    assert_eq!((b.get(5), b.get(10), b.vat_due), (-1_000, -250, -250));
}

#[test]
fn the_fingerprint_follows_the_accounts_and_their_boxes() {
    let one = fingerprint(d("2026-09-30"), &[a(2611, 10, -100)]);
    assert_eq!(one.len(), 64);
    assert_eq!(one, fingerprint(d("2026-09-30"), &[a(2611, 10, -100)]));
    assert_ne!(one, fingerprint(d("2026-09-30"), &[a(2611, 10, -101)]));
    assert_ne!(one, fingerprint(d("2026-09-30"), &[a(2611, 11, -100)]));
    assert_ne!(one, fingerprint(d("2026-06-30"), &[a(2611, 10, -100)]));
}
