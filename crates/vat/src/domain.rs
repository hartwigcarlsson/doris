//! Pure VAT rules: boxes, the settlement voucher, status and decisions.

use doris_ledger::VatAccountTotal;
use doris_ledger::vat_box::{Side, VatBox};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// An account's saldo (debit − credit, öre) over a period, with its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSaldo {
    pub account: u16,
    pub vat_box: VatBox,
    pub saldo: i64,
}

/// What is declared: whole kronor per box that is not zero, and box 49.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Boxes {
    pub amounts: Vec<(VatBox, i64)>,
    pub vat_due: i64,
}

impl Boxes {
    /// Kronor in box `n`, 0 when empty.
    pub fn get(&self, n: u8) -> i64 {
        self.amounts
            .iter()
            .find(|(b, _)| b.get() == n)
            .map_or(0, |(_, kr)| *kr)
    }
}

const OUTPUT_VAT: [u8; 9] = [10, 11, 12, 30, 31, 32, 60, 61, 62];

pub fn saldos(totals: &[VatAccountTotal]) -> Vec<AccountSaldo> {
    totals
        .iter()
        .map(|t| AccountSaldo { account: t.number, vat_box: t.vat_box, saldo: t.saldo })
        .collect()
}

/// An account's saldo as its box counts it: positive for sales and
/// output VAT on the credit side, purchases and input VAT on the debit side.
pub fn signed(a: &AccountSaldo) -> i64 {
    match a.vat_box.side() {
        Side::Debit => a.saldo,
        Side::Credit => -a.saldo,
    }
}

/// Each box summed in öre, then the öre struck off toward zero, as
/// Skatteverket asks. Box 49 comes from the rounded boxes, as Skatteverket
/// checks it.
pub fn boxes(accounts: &[AccountSaldo]) -> Boxes {
    let mut ore = BTreeMap::<VatBox, i64>::new();
    for a in accounts {
        *ore.entry(a.vat_box).or_default() += signed(a);
    }
    let amounts: Vec<(VatBox, i64)> = ore
        .into_iter()
        .map(|(b, o)| (b, o / 100))
        .filter(|(_, kr)| *kr != 0)
        .collect();
    let mut boxes = Boxes { amounts, vat_due: 0 };
    boxes.vat_due = OUTPUT_VAT.iter().map(|&n| boxes.get(n)).sum::<i64>() - boxes.get(48);
    boxes
}

/// Box 49 in öre before rounding: the VAT the books hold.
pub fn booked_vat(accounts: &[AccountSaldo]) -> i64 {
    accounts
        .iter()
        .filter(|a| a.vat_box.is_vat())
        .map(|a| -a.saldo)
        .sum()
}

/// Hex SHA-256 of what a submission would record, so marking a period
/// submitted is refused when the books changed after it was shown.
pub fn fingerprint(period_end: Date, accounts: &[AccountSaldo]) -> String {
    let json = serde_json::to_vec(&(period_end, accounts)).expect("plain data serializes");
    Sha256::digest(json).iter().map(|b| format!("{b:02x}")).collect()
}
