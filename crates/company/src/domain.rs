//! Pure company rules: value types, events, state and decisions. No I/O.

use jiff::Span;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

/// The Luhn check over all digits, weights 1,2,1,2… from the right. Used
/// for organisationsnummer, bankgiro and plusgiro.
pub fn luhn(digits: &str) -> bool {
    let sum: u32 = digits
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, b)| {
            let d = u32::from(b - b'0') * if i % 2 == 0 { 1 } else { 2 };
            if d > 9 { d - 9 } else { d }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn bounded_text(raw: &str, max_chars: usize) -> Option<String> {
    let text = raw.trim();
    (!text.is_empty() && text.chars().count() <= max_chars).then(|| text.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CompanyName(String);

impl CompanyName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        bounded_text(raw, 200)
            .map(Self)
            .ok_or(DomainError::InvalidCompanyName)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Postal address. Every part is optional: Bolagsverket often has only
/// postnummer and postort.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub street: Option<String>,
    pub postal_code: Option<String>,
    pub city: Option<String>,
}

impl Address {
    pub fn parse(street: &str, postal_code: &str, city: &str) -> Result<Self, DomainError> {
        let part = |raw: &str| {
            let text = raw.trim();
            match text.chars().count() {
                0 => Ok(None),
                1..=200 => Ok(Some(text.to_owned())),
                _ => Err(DomainError::InvalidAddress),
            }
        };
        Ok(Self {
            street: part(street)?,
            postal_code: part(postal_code)?,
            city: part(city)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalForm {
    Aktiebolag,
    Handelsbolag,
    Kommanditbolag,
    EnskildFirma,
    EkonomiskForening,
    IdeellForening,
    Stiftelse,
    Other,
}

impl LegalForm {
    pub fn as_str(self) -> &'static str {
        match self {
            LegalForm::Aktiebolag => "aktiebolag",
            LegalForm::Handelsbolag => "handelsbolag",
            LegalForm::Kommanditbolag => "kommanditbolag",
            LegalForm::EnskildFirma => "enskild_firma",
            LegalForm::EkonomiskForening => "ekonomisk_forening",
            LegalForm::IdeellForening => "ideell_forening",
            LegalForm::Stiftelse => "stiftelse",
            LegalForm::Other => "other",
        }
    }

    /// BFL 3 kap. 1 §: a natural person and a handelsbolag must use the
    /// calendar year. (The exception for handelsbolag owned by legal persons
    /// is not supported yet.)
    fn requires_calendar_year(self) -> bool {
        matches!(
            self,
            LegalForm::EnskildFirma | LegalForm::Handelsbolag | LegalForm::Kommanditbolag
        )
    }
}

/// Kontantmetoden (BFL 5 kap. 2 §, net sales up to 3 MSEK) or faktureringsmetoden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountingMethod {
    Cash,
    Invoice,
}

impl AccountingMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountingMethod::Cash => "cash",
            AccountingMethod::Invoice => "invoice",
        }
    }
}

/// A räkenskapsår: from the first day of a month to the last day of a month.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiscalYear {
    pub start: Date,
    pub end: Date,
}

impl FiscalYear {
    /// The company's first räkenskapsår (BFL 3 kap. 1 och 3 §§): 1-18
    /// months, whole months, and ending 31 December where the legal form
    /// requires the calendar year.
    pub fn first(start: Date, end: Date, legal_form: LegalForm) -> Result<Self, DomainError> {
        let months = (i32::from(end.year()) * 12 + i32::from(end.month()))
            - (i32::from(start.year()) * 12 + i32::from(start.month()))
            + 1;
        let valid = start == start.first_of_month()
            && end == end.last_of_month()
            && (1..=18).contains(&months)
            && (!legal_form.requires_calendar_year() || end.month() == 12);
        if valid {
            Ok(Self { start, end })
        } else {
            Err(DomainError::InvalidFiscalYear)
        }
    }

    /// The following räkenskapsår: 12 months, ending in the same month.
    pub fn next(&self) -> Self {
        let start = self
            .end
            .tomorrow()
            .expect("fiscal years are far from the date limits");
        let end = start
            .checked_add(Span::new().months(11))
            .expect("fiscal years are far from the date limits")
            .last_of_month();
        Self { start, end }
    }

    /// The räkenskapsår `day` falls in, counting from this (first) one.
    /// Days before the first year give the first year.
    pub fn containing(&self, day: Date) -> Self {
        let mut year = *self;
        while day > year.end {
            year = year.next();
        }
        year
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CompanyEvent {
    CompanyRegistered {
        company_id: Uuid,
        org_nr: OrgNr,
        name: CompanyName,
        legal_form: LegalForm,
        address: Address,
        first_fiscal_year: FiscalYear,
        accounting_method: AccountingMethod,
        created_by: Uuid,
    },
    MemberAdded {
        user_id: Uuid,
        added_by: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Company {
    pub id: Uuid,
    pub org_nr: OrgNr,
    pub name: CompanyName,
    pub legal_form: LegalForm,
    pub address: Address,
    pub first_fiscal_year: FiscalYear,
    pub accounting_method: AccountingMethod,
    pub members: Vec<Uuid>,
}

impl Company {
    pub fn from_events(events: &[CompanyEvent]) -> Option<Self> {
        let mut company = None;
        for event in events {
            match event.clone() {
                CompanyEvent::CompanyRegistered {
                    company_id,
                    org_nr,
                    name,
                    legal_form,
                    address,
                    first_fiscal_year,
                    accounting_method,
                    ..
                } => {
                    company = Some(Company {
                        id: company_id,
                        org_nr,
                        name,
                        legal_form,
                        address,
                        first_fiscal_year,
                        accounting_method,
                        members: vec![],
                    });
                }
                CompanyEvent::MemberAdded { user_id, .. } => {
                    if let Some(c) = company.as_mut() {
                        c.members.push(user_id);
                    }
                }
            }
        }
        company
    }

    pub fn is_member(&self, user_id: Uuid) -> bool {
        self.members.contains(&user_id)
    }
}

#[derive(Debug, Clone)]
pub struct RegisterCompany {
    pub company_id: Uuid,
    pub org_nr: OrgNr,
    pub name: CompanyName,
    pub legal_form: LegalForm,
    pub address: Address,
    pub first_fiscal_year: FiscalYear,
    pub accounting_method: AccountingMethod,
}

/// The creator becomes the first member, so membership has one source.
pub fn register_company(cmd: RegisterCompany, created_by: Uuid) -> Vec<CompanyEvent> {
    vec![
        CompanyEvent::CompanyRegistered {
            company_id: cmd.company_id,
            org_nr: cmd.org_nr,
            name: cmd.name,
            legal_form: cmd.legal_form,
            address: cmd.address,
            first_fiscal_year: cmd.first_fiscal_year,
            accounting_method: cmd.accounting_method,
            created_by,
        },
        CompanyEvent::MemberAdded {
            user_id: created_by,
            added_by: created_by,
        },
    ]
}

/// Idempotent: adding an existing member yields no events.
pub fn add_member(
    company: &Company,
    actor: Uuid,
    user_id: Uuid,
) -> Result<Vec<CompanyEvent>, DomainError> {
    if !company.is_member(actor) {
        return Err(DomainError::NotMember);
    }
    if company.is_member(user_id) {
        return Ok(vec![]);
    }
    Ok(vec![CompanyEvent::MemberAdded {
        user_id,
        added_by: actor,
    }])
}
