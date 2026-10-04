//! Pure rules for the customer and supplier registers. No I/O.

use doris_company::domain::{OrgNr, luhn};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("name must be 1-200 characters")]
    InvalidName,
    #[error("invalid organisationsnummer")]
    InvalidOrgNr,
    #[error("address fields must be at most 200 characters")]
    InvalidAddress,
    #[error("invalid email address")]
    InvalidEmail,
    #[error("invalid momsregistreringsnummer")]
    InvalidVatNumber,
    #[error("payment terms must be 0-365 days")]
    InvalidPaymentTerms,
    #[error("invalid bankgiro number")]
    InvalidBankgiro,
    #[error("invalid plusgiro number")]
    InvalidPlusgiro,
    #[error("invalid IBAN")]
    InvalidIban,
    #[error("invalid BIC")]
    InvalidBic,
    #[error("customer not found")]
    CustomerNotFound,
    #[error("supplier not found")]
    SupplierNotFound,
}

/// Spaces and hyphens removed, letters upper case: how numbers are pasted.
fn compact(raw: &str) -> String {
    raw.chars()
        .filter(|c| !matches!(c, ' ' | '-'))
        .collect::<String>()
        .to_ascii_uppercase()
}

fn upper_alnum(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

fn upper_letters(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_uppercase())
}

fn digits(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PartyName(String);

impl PartyName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let name = raw.trim();
        if (1..=200).contains(&name.chars().count()) {
            Ok(Self(name.to_owned()))
        } else {
            Err(DomainError::InvalidName)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Personal data: never logged, so `Debug` is redacted.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Email(String);

impl std::fmt::Debug for Email {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Email(<redacted>)")
    }
}

impl Email {
    /// The same rule as `doris_identity::domain::Email`, which this crate
    /// does not depend on.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let email = raw.trim().to_lowercase();
        let well_formed = match email.split_once('@') {
            Some((local, domain)) => {
                !local.is_empty()
                    && !domain.contains('@')
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
            }
            None => false,
        };
        if well_formed && email.len() <= 254 && !email.contains(char::is_whitespace) {
            Ok(Self(email))
        } else {
            Err(DomainError::InvalidEmail)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Momsregistreringsnummer. The format is checked, but the number is not
/// looked up in VIES.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VatNumber(String);

impl VatNumber {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let vat = compact(raw);
        // is_ascii first: slicing by byte index must not split a char.
        let valid = vat.is_ascii()
            && (4..=14).contains(&vat.len())
            && upper_letters(&vat[..2])
            && upper_alnum(&vat[2..])
            && (&vat[..2] != "SE"
                || (vat.len() == 14
                    && vat.ends_with("01")
                    && digits(&vat[2..12])
                    && OrgNr::parse(&vat[2..12]).is_ok()));
        if valid {
            Ok(Self(vat))
        } else {
            Err(DomainError::InvalidVatNumber)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaymentTerms(u16);

impl PaymentTerms {
    pub fn new(days: u32) -> Result<Self, DomainError> {
        match u16::try_from(days) {
            Ok(days) if days <= 365 => Ok(Self(days)),
            _ => Err(DomainError::InvalidPaymentTerms),
        }
    }

    pub fn get(self) -> u32 {
        self.0.into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bankgiro(String);

impl Bankgiro {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let bg = compact(raw);
        if (7..=8).contains(&bg.len()) && digits(&bg) && luhn(&bg) {
            Ok(Self(bg))
        } else {
            Err(DomainError::InvalidBankgiro)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `NNN-NNNN` or `NNNN-NNNN`.
    pub fn formatted(&self) -> String {
        let (head, tail) = self.0.split_at(self.0.len() - 4);
        format!("{head}-{tail}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Plusgiro(String);

impl Plusgiro {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let pg = compact(raw);
        if (2..=8).contains(&pg.len()) && digits(&pg) && luhn(&pg) {
            Ok(Self(pg))
        } else {
            Err(DomainError::InvalidPlusgiro)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A hyphen before the check digit.
    pub fn formatted(&self) -> String {
        let (head, tail) = self.0.split_at(self.0.len() - 1);
        format!("{head}-{tail}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Iban(String);

impl Iban {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let iban = compact(raw);
        let valid = iban.is_ascii()
            && (15..=34).contains(&iban.len())
            && upper_letters(&iban[..2])
            && digits(&iban[2..4])
            && upper_alnum(&iban[4..])
            && mod_97(&iban) == Some(1);
        if valid {
            Ok(Self(iban))
        } else {
            Err(DomainError::InvalidIban)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Groups of four.
    pub fn formatted(&self) -> String {
        let chars: Vec<char> = self.0.chars().collect();
        chars
            .chunks(4)
            .map(|group| group.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// ISO 13616: the first four characters moved to the end, letters as
/// 10..=35, the whole number mod 97, digit by digit so nothing overflows.
fn mod_97(iban: &str) -> Option<u32> {
    iban[4..]
        .chars()
        .chain(iban[..4].chars())
        .try_fold(0u32, |acc, c| {
            let d = c.to_digit(36)?;
            Some(if d < 10 {
                (acc * 10 + d) % 97
            } else {
                (acc * 100 + d) % 97
            })
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bic(String);

impl Bic {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let bic = compact(raw);
        let valid = bic.is_ascii()
            && matches!(bic.len(), 8 | 11)
            && upper_letters(&bic[..6])
            && upper_alnum(&bic[6..]);
        if valid {
            Ok(Self(bic))
        } else {
            Err(DomainError::InvalidBic)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
