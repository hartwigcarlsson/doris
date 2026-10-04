//! Pure rules for the customer and supplier registers. No I/O.

use doris_company::domain::{Address, OrgNr, luhn};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    #[error("invoice number must be 1-50 characters")]
    InvalidInvoiceNumber,
    #[error("this supplier's invoice is already registered")]
    DuplicateSupplierInvoice,
    #[error("due date is before the invoice date")]
    InvalidDueDate,
    #[error("payment reference must be at most 50 characters")]
    InvalidReference,
    #[error("an invoice has 1-50 lines, each above 0")]
    InvalidInvoiceLines,
    #[error("VAT rate must be 25, 12, 6 or 0 %")]
    InvalidVatRate,
    #[error("VAT may differ from the computed amount by at most 1 krona")]
    InvalidVatAmount,
    #[error("an invoice line cannot book 2440 or a VAT account")]
    InvalidInvoiceAccount,
    #[error("payment account must be 1900-1999")]
    InvalidPaymentAccount,
    #[error("supplier is inactive")]
    SupplierInactive,
    #[error("no such supplier invoice")]
    SupplierInvoiceNotFound,
    #[error("supplier invoice is paid")]
    SupplierInvoicePaid,
    #[error("supplier invoice is not paid")]
    SupplierInvoiceNotPaid,
    #[error("supplier invoice is cancelled")]
    SupplierInvoiceCancelled,
    #[error("reason must be 1-200 characters")]
    InvalidReason,
    #[error("invoice date is in the future")]
    InvoiceDateInFuture,
    #[error("customer is inactive")]
    CustomerInactive,
    #[error("no such customer invoice")]
    CustomerInvoiceNotFound,
    #[error("this invoice number is already used")]
    DuplicateCustomerInvoice,
    #[error("customer invoice is paid")]
    CustomerInvoicePaid,
    #[error("customer invoice is not paid")]
    CustomerInvoiceNotPaid,
    #[error("customer invoice is cancelled")]
    CustomerInvoiceCancelled,
}

/// Whitespace (also non-breaking spaces from PDFs) and hyphens removed,
/// letters upper case: how numbers are pasted.
fn compact(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
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

/// A customer as typed into the form; blank optional fields become `None`.
/// No `Debug`: the org.nr may be a personnummer.
#[derive(Clone, Copy)]
pub struct CustomerForm<'a> {
    pub name: &'a str,
    pub org_nr: &'a str,
    pub vat_number: &'a str,
    pub street: &'a str,
    pub postal_code: &'a str,
    pub city: &'a str,
    pub email: &'a str,
    pub payment_terms: u32,
}

#[derive(Clone, Copy)]
pub struct SupplierForm<'a> {
    pub name: &'a str,
    pub org_nr: &'a str,
    pub vat_number: &'a str,
    pub street: &'a str,
    pub postal_code: &'a str,
    pub city: &'a str,
    pub email: &'a str,
    pub bankgiro: &'a str,
    pub plusgiro: &'a str,
    pub iban: &'a str,
    pub bic: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerDetails {
    pub name: PartyName,
    pub org_nr: Option<OrgNr>,
    pub vat_number: Option<VatNumber>,
    pub address: Address,
    pub email: Option<Email>,
    pub payment_terms: PaymentTerms,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierDetails {
    pub name: PartyName,
    pub org_nr: Option<OrgNr>,
    pub vat_number: Option<VatNumber>,
    pub address: Address,
    pub email: Option<Email>,
    pub bankgiro: Option<Bankgiro>,
    pub plusgiro: Option<Plusgiro>,
    pub iban: Option<Iban>,
    pub bic: Option<Bic>,
}

pub(crate) fn optional<T>(
    raw: &str,
    parse: impl FnOnce(&str) -> Result<T, DomainError>,
) -> Result<Option<T>, DomainError> {
    if raw.trim().is_empty() {
        Ok(None)
    } else {
        parse(raw).map(Some)
    }
}

/// Org.nr or personnummer: a private customer has the latter.
fn org_nr(raw: &str) -> Result<Option<OrgNr>, DomainError> {
    optional(raw, |raw| {
        OrgNr::parse(raw).map_err(|_| DomainError::InvalidOrgNr)
    })
}

fn address(street: &str, postal_code: &str, city: &str) -> Result<Address, DomainError> {
    Address::parse(street, postal_code, city).map_err(|_| DomainError::InvalidAddress)
}

impl CustomerDetails {
    pub fn parse(form: &CustomerForm) -> Result<Self, DomainError> {
        Ok(Self {
            name: PartyName::parse(form.name)?,
            org_nr: org_nr(form.org_nr)?,
            vat_number: optional(form.vat_number, VatNumber::parse)?,
            address: address(form.street, form.postal_code, form.city)?,
            email: optional(form.email, Email::parse)?,
            payment_terms: PaymentTerms::new(form.payment_terms)?,
        })
    }
}

impl SupplierDetails {
    pub fn parse(form: &SupplierForm) -> Result<Self, DomainError> {
        Ok(Self {
            name: PartyName::parse(form.name)?,
            org_nr: org_nr(form.org_nr)?,
            vat_number: optional(form.vat_number, VatNumber::parse)?,
            address: address(form.street, form.postal_code, form.city)?,
            email: optional(form.email, Email::parse)?,
            bankgiro: optional(form.bankgiro, Bankgiro::parse)?,
            plusgiro: optional(form.plusgiro, Plusgiro::parse)?,
            iban: optional(form.iban, Iban::parse)?,
            bic: optional(form.bic, Bic::parse)?,
        })
    }
}

/// What the register logic needs to know about one register's details.
pub trait PartyDetails: Clone + PartialEq {
    const NOT_FOUND: DomainError;
}

impl PartyDetails for CustomerDetails {
    const NOT_FOUND: DomainError = DomainError::CustomerNotFound;
}

impl PartyDetails for SupplierDetails {
    const NOT_FOUND: DomainError = DomainError::SupplierNotFound;
}

/// A change to a register. Stored as a [`CustomerEvent`] or a
/// [`SupplierEvent`], so the log names the register.
#[derive(Debug, Clone, PartialEq)]
pub enum Change<D> {
    Added {
        number: u32,
        details: D,
    },
    /// The full new set of details, not a diff.
    Updated {
        number: u32,
        details: D,
    },
    Deactivated {
        number: u32,
    },
    Reactivated {
        number: u32,
    },
}

impl<D> Change<D> {
    pub fn number(&self) -> u32 {
        match self {
            Change::Added { number, .. }
            | Change::Updated { number, .. }
            | Change::Deactivated { number }
            | Change::Reactivated { number } => *number,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CustomerEvent {
    CustomerAdded {
        number: u32,
        details: CustomerDetails,
    },
    CustomerUpdated {
        number: u32,
        details: CustomerDetails,
    },
    CustomerDeactivated {
        number: u32,
    },
    CustomerReactivated {
        number: u32,
    },
}

impl From<Change<CustomerDetails>> for CustomerEvent {
    fn from(change: Change<CustomerDetails>) -> Self {
        match change {
            Change::Added { number, details } => Self::CustomerAdded { number, details },
            Change::Updated { number, details } => Self::CustomerUpdated { number, details },
            Change::Deactivated { number } => Self::CustomerDeactivated { number },
            Change::Reactivated { number } => Self::CustomerReactivated { number },
        }
    }
}

impl From<CustomerEvent> for Change<CustomerDetails> {
    fn from(event: CustomerEvent) -> Self {
        match event {
            CustomerEvent::CustomerAdded { number, details } => Self::Added { number, details },
            CustomerEvent::CustomerUpdated { number, details } => Self::Updated { number, details },
            CustomerEvent::CustomerDeactivated { number } => Self::Deactivated { number },
            CustomerEvent::CustomerReactivated { number } => Self::Reactivated { number },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SupplierEvent {
    SupplierAdded {
        number: u32,
        details: SupplierDetails,
    },
    SupplierUpdated {
        number: u32,
        details: SupplierDetails,
    },
    SupplierDeactivated {
        number: u32,
    },
    SupplierReactivated {
        number: u32,
    },
}

impl From<Change<SupplierDetails>> for SupplierEvent {
    fn from(change: Change<SupplierDetails>) -> Self {
        match change {
            Change::Added { number, details } => Self::SupplierAdded { number, details },
            Change::Updated { number, details } => Self::SupplierUpdated { number, details },
            Change::Deactivated { number } => Self::SupplierDeactivated { number },
            Change::Reactivated { number } => Self::SupplierReactivated { number },
        }
    }
}

impl From<SupplierEvent> for Change<SupplierDetails> {
    fn from(event: SupplierEvent) -> Self {
        match event {
            SupplierEvent::SupplierAdded { number, details } => Self::Added { number, details },
            SupplierEvent::SupplierUpdated { number, details } => Self::Updated { number, details },
            SupplierEvent::SupplierDeactivated { number } => Self::Deactivated { number },
            SupplierEvent::SupplierReactivated { number } => Self::Reactivated { number },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Party<D> {
    pub number: u32,
    pub details: D,
    pub active: bool,
}

/// One company's customers or suppliers. Never removed, only deactivated.
#[derive(Debug, Clone, PartialEq)]
pub struct Register<D> {
    parties: BTreeMap<u32, Party<D>>,
}

impl<D> Default for Register<D> {
    fn default() -> Self {
        Self {
            parties: BTreeMap::new(),
        }
    }
}

impl<D> Register<D> {
    pub fn from_changes(changes: impl IntoIterator<Item = Change<D>>) -> Self {
        let mut register = Self::default();
        changes.into_iter().for_each(|c| register.apply(c));
        register
    }

    pub fn apply(&mut self, change: Change<D>) {
        match change {
            Change::Added { number, details } => {
                self.parties.insert(
                    number,
                    Party {
                        number,
                        details,
                        active: true,
                    },
                );
            }
            Change::Updated { number, details } => {
                if let Some(party) = self.parties.get_mut(&number) {
                    party.details = details;
                }
            }
            Change::Deactivated { number } => self.set_active(number, false),
            Change::Reactivated { number } => self.set_active(number, true),
        }
    }

    fn set_active(&mut self, number: u32, active: bool) {
        if let Some(party) = self.parties.get_mut(&number) {
            party.active = active;
        }
    }

    pub fn get(&self, number: u32) -> Option<&Party<D>> {
        self.parties.get(&number)
    }

    /// By number.
    pub fn parties(&self) -> impl Iterator<Item = &Party<D>> {
        self.parties.values()
    }
}

/// The next number: one more than the highest, so 1..=n without gaps.
pub fn add<D: PartyDetails>(
    register: &Register<D>,
    details: D,
) -> Result<Vec<Change<D>>, DomainError> {
    let number = register.parties.keys().next_back().map_or(1, |n| n + 1);
    Ok(vec![Change::Added { number, details }])
}

/// The same details yield no events.
pub fn update<D: PartyDetails>(
    register: &Register<D>,
    number: u32,
    details: D,
) -> Result<Vec<Change<D>>, DomainError> {
    let party = register.get(number).ok_or(D::NOT_FOUND)?;
    if party.details == details {
        return Ok(vec![]);
    }
    Ok(vec![Change::Updated { number, details }])
}

/// Idempotent: a party already in the wanted state yields no events.
pub fn set_active<D: PartyDetails>(
    register: &Register<D>,
    number: u32,
    active: bool,
) -> Result<Vec<Change<D>>, DomainError> {
    let party = register.get(number).ok_or(D::NOT_FOUND)?;
    Ok(match (party.active, active) {
        (true, false) => vec![Change::Deactivated { number }],
        (false, true) => vec![Change::Reactivated { number }],
        _ => vec![],
    })
}
