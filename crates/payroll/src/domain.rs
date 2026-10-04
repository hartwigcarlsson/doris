//! Pure payroll rules: employees, payroll runs and arbetsgivaravgifter.
//! No I/O, no clock.

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// The largest amount on a voucher line (the ledger's limit), in öre.
pub const MAX_AMOUNT: i64 = 10_000_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("personnummer must be ÅÅÅÅMMDD-NNNN with a valid date and check digit")]
    InvalidPersonalIdentityNumber,
    #[error("employee name must be 1-100 characters")]
    InvalidEmployeeName,
    #[error("salary must be more than zero")]
    InvalidSalary,
    #[error("salary account must be 7010, 7210 or 7220")]
    InvalidSalaryAccount,
}

/// A personnummer or samordningsnummer, as twelve digits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalIdentityNumber(String);

impl PersonalIdentityNumber {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let invalid = DomainError::InvalidPersonalIdentityNumber;
        let raw = raw.trim();
        // ASCII first: the slicing below is by byte.
        if !raw.is_ascii() {
            return Err(invalid);
        }
        let digits = match raw.len() {
            12 => raw.to_owned(),
            13 if raw.as_bytes()[8] == b'-' => format!("{}{}", &raw[..8], &raw[9..]),
            _ => return Err(invalid),
        };
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid);
        }
        let number = |range: std::ops::Range<usize>| digits[range].parse::<i16>().unwrap();
        let day = number(6..8);
        // A samordningsnummer adds 60 to the day.
        let day = if day > 60 { day - 60 } else { day };
        let date = Date::new(number(0..4), number(4..6) as i8, day as i8);
        if date.is_err() || !doris_company::domain::luhn(&digits[2..]) {
            return Err(invalid);
        }
        Ok(Self(digits))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `ÅÅÅÅMMDD-NNNN`.
    pub fn formatted(&self) -> String {
        format!("{}-{}", &self.0[..8], &self.0[8..])
    }

    pub fn birth_year(&self) -> i16 {
        self.0[..4].parse().expect("parsed digits")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmployeeName(String);

impl EmployeeName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let name = raw.trim();
        match name.chars().count() {
            1..=100 => Ok(Self(name.to_owned())),
            _ => Err(DomainError::InvalidEmployeeName),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The BAS account an employee's gross salary is booked on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SalaryAccount(u16);

impl SalaryAccount {
    /// 7210 Löner till tjänstemän.
    pub const DEFAULT: Self = Self(7210);

    pub fn parse(raw: u32) -> Result<Self, DomainError> {
        match raw {
            7010 | 7210 | 7220 => Ok(Self(raw as u16)),
            _ => Err(DomainError::InvalidSalaryAccount),
        }
    }

    pub fn get(self) -> u32 {
        self.0.into()
    }
}
