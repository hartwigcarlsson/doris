//! The boxes (rutor) of Skatteverket's momsdeklaration (SKV 4700) and the
//! box each BAS account goes in by default. Pure.

use crate::domain::DomainError;
use serde::{Deserialize, Serialize};

/// Every box an account can be in, in the order of Skatteverket's file.
/// 49 is computed and is never an account's.
pub const BOXES: [u8; 28] = [
    5, 6, 7, 8, 20, 21, 22, 23, 24, 50, 35, 36, 37, 38, 39, 40, 41, 42, 10, 11, 12, 30, 31, 32,
    60, 61, 62, 48,
];

/// A box on the momsdeklaration, stored as its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct VatBox(u8);

/// The side of an account's saldo (debit − credit) a box counts as positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Debit,
    Credit,
}

impl VatBox {
    pub fn parse(n: u32) -> Result<Self, DomainError> {
        u8::try_from(n)
            .ok()
            .filter(|b| BOXES.contains(b))
            .map(Self)
            .ok_or(DomainError::InvalidVatBox)
    }

    pub fn get(self) -> u8 {
        self.0
    }

    /// Purchases and input VAT are debits; sales and output VAT credits.
    pub fn side(self) -> Side {
        match self.0 {
            20..=24 | 48 | 50 => Side::Debit,
            _ => Side::Credit,
        }
    }

    /// The VAT itself: the settlement moves these accounts to 2650.
    pub fn is_vat(self) -> bool {
        matches!(self.0, 10..=12 | 30..=32 | 60..=62 | 48)
    }
}

impl TryFrom<u32> for VatBox {
    type Error = DomainError;
    fn try_from(n: u32) -> Result<Self, DomainError> {
        Self::parse(n)
    }
}

impl From<VatBox> for u32 {
    fn from(b: VatBox) -> u32 {
        b.0.into()
    }
}

/// The box BAS puts `account` in, looked up by number so it also holds for
/// accounts added later.
pub fn default_vat_box(account: u16) -> Option<VatBox> {
    let n = match account {
        3001..=3003 | 3106 => 5,
        3004 => 42,
        3108 => 35,
        3305 => 40,
        3308 => 39,
        2611 => 10,
        2621 => 11,
        2631 => 12,
        4515..=4517 => 20,
        4535..=4537 => 21,
        4531..=4533 => 22,
        4415..=4417 => 23,
        4425..=4427 => 24,
        2614 => 30,
        2624 => 31,
        2634 => 32,
        4545..=4547 => 50,
        2615 => 60,
        2625 => 61,
        2635 => 62,
        2640 | 2641 | 2645 | 2647 => 48,
        _ => return None,
    };
    Some(VatBox::parse(n).expect("default boxes are on the form"))
}
