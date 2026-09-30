//! Pure company rules: value types, events, state and decisions. No I/O.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("invalid organisationsnummer")]
    InvalidOrgNr,
    #[error("company name must be 1-200 characters")]
    InvalidCompanyName,
    #[error("address fields must be at most 200 characters")]
    InvalidAddress,
    #[error("fiscal year breaks BFL 3 kap.")]
    InvalidFiscalYear,
    #[error("not a member of the company")]
    NotMember,
}

/// Organisationsnummer: 10 digits, no hyphen. For an enskild firma it is the
/// owner's personnummer, so it is personal data and must never be logged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrgNr(String);

/// Redacted: the number may be a personnummer and must never reach a log.
impl std::fmt::Debug for OrgNr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OrgNr(<redacted>)")
    }
}

impl OrgNr {
    /// Accepts `NNNNNN-NNNN`, spaces, and the 12-digit form with a century
    /// (`16`, `19` or `20`) in front.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let digits: String = raw.chars().filter(|c| !matches!(c, '-' | ' ')).collect();
        // is_ascii first: slicing by byte index must not split a char.
        let digits = match digits.len() {
            12 if digits.is_ascii() && ["16", "19", "20"].contains(&&digits[..2]) => {
                digits[2..].to_owned()
            }
            _ => digits,
        };
        if digits.len() == 10 && digits.bytes().all(|b| b.is_ascii_digit()) && luhn(&digits) {
            Ok(Self(digits))
        } else {
            Err(DomainError::InvalidOrgNr)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn formatted(&self) -> String {
        format!("{}-{}", &self.0[..6], &self.0[6..])
    }

    /// Organisationsnummer have 2 or more as their third digit; a lower
    /// digit is a birth month, so this is a person's personnummer.
    pub fn is_personal_identity_number(&self) -> bool {
        self.0.as_bytes()[2] < b'2'
    }
}

/// The Luhn check (weights 2,1,2,1…) over all ten digits.
fn luhn(digits: &str) -> bool {
    let sum: u32 = digits
        .bytes()
        .enumerate()
        .map(|(i, b)| {
            let d = u32::from(b - b'0') * if i % 2 == 0 { 2 } else { 1 };
            if d > 9 { d - 9 } else { d }
        })
        .sum();
    sum.is_multiple_of(10)
}
