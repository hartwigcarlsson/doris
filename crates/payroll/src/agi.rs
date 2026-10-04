//! Arbetsgivardeklaration på individnivå (AGI): what a month declares to
//! Skatteverket, computed from booked payroll runs, and the file that
//! carries it. Pure: no I/O, no clock.
//!
//! Skatteverket, "Teknisk beskrivning 1.1.18.2 för arbetsgivardeklaration".

use crate::domain::DomainError;
use jiff::civil::{Date, date};
use serde::{Deserialize, Serialize};

/// A redovisningsperiod: the pay date's year and month, `ÅÅÅÅMM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Period(u32);

impl Period {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let raw = raw.trim();
        if raw.len() != 6 || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return Err(DomainError::InvalidPeriod);
        }
        let value: u32 = raw.parse().map_err(|_| DomainError::InvalidPeriod)?;
        let (year, month) = (value / 100, value % 100);
        if year < 1900 || !(1..=12).contains(&month) {
            return Err(DomainError::InvalidPeriod);
        }
        Ok(Self(value))
    }

    pub fn of(day: Date) -> Self {
        Self(day.year() as u32 * 100 + day.month() as u32)
    }

    pub fn first_day(self) -> Date {
        date((self.0 / 100) as i16, (self.0 % 100) as i8, 1)
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for Period {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Who Skatteverket may contact about the company's AGI. The file needs a
/// name, a phone number and an e-mail address (schema types TEXT50, TEXT20
/// and EPOST).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgiContact {
    pub name: String,
    pub phone: String,
    pub email: String,
}

impl AgiContact {
    pub fn parse(name: &str, phone: &str, email: &str) -> Result<Self, DomainError> {
        let (name, phone, email) = (name.trim(), phone.trim(), email.trim());
        // TEXT50/TEXT20: not empty, not only whitespace (trimmed), no < or >.
        let text =
            |s: &str, max: usize| (1..=max).contains(&s.chars().count()) && !s.contains(['<', '>']);
        if text(name, 50) && text(phone, 20) && is_email(email) {
            Ok(Self {
                name: name.to_owned(),
                phone: phone.to_owned(),
                email: email.to_owned(),
            })
        } else {
            Err(DomainError::InvalidAgiContact)
        }
    }
}

/// Skatteverket's EPOST pattern, 5–254 characters:
/// `[a-zA-Z0-9_]+([-+.'][a-zA-Z0-9_]+)*@[a-zA-Z0-9_]+([-.][a-zA-Z0-9_]+)*\.[a-zA-Z0-9_]+([-.][a-zA-Z0-9_]+)*`
fn is_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    (5..=254).contains(&s.chars().count())
        && words(local, &['-', '+', '.', '\'']).is_some()
        && words(domain, &['-', '.']).is_some_and(|dots| dots >= 1)
}

/// `s` as runs of `[A-Za-z0-9_]` joined by single separators from `seps`,
/// starting and ending with a run; returns how many separators were dots.
fn words(s: &str, seps: &[char]) -> Option<usize> {
    let (mut dots, mut in_word) = (0, false);
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            in_word = true;
        } else if in_word && seps.contains(&c) {
            in_word = false;
            dots += usize::from(c == '.');
        } else {
            return None;
        }
    }
    in_word.then_some(dots)
}
