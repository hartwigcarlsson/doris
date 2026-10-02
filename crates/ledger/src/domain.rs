//! Pure ledger rules: the chart of accounts and vouchers. No I/O, no clock.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("account number must be 1000-8999")]
    InvalidAccountNumber,
    #[error("account name must be 1-100 characters")]
    InvalidAccountName,
    #[error("account already exists")]
    AccountExists,
    #[error("no such account")]
    AccountNotFound,
    #[error("account is inactive")]
    AccountInactive,
    #[error("voucher text must be 1-200 characters")]
    InvalidVoucherText,
    #[error("a voucher has 2-100 lines")]
    InvalidVoucherLines,
    #[error("each line has exactly one of debit and credit, at most 10^13 öre")]
    InvalidAmount,
    #[error("debit and credit differ")]
    VoucherUnbalanced,
    #[error("voucher date is in the future")]
    VoucherDateInFuture,
    #[error("voucher date is before the first fiscal year")]
    VoucherDateBeforeFirstFiscalYear,
    #[error("correction date is outside the voucher's fiscal year")]
    CorrectionDateOutsideFiscalYear,
    #[error("no such voucher")]
    VoucherNotFound,
    #[error("voucher is already corrected")]
    AlreadyCorrected,
    #[error("a correction cannot be corrected")]
    CannotCorrectCorrection,
}

/// A BAS account number: four digits, 1000-8999.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountNumber(u16);

impl AccountNumber {
    pub fn parse(raw: u32) -> Result<Self, DomainError> {
        match raw {
            1000..=8999 => Ok(Self(raw as u16)),
            _ => Err(DomainError::InvalidAccountNumber),
        }
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountName(String);

impl AccountName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let name = raw.trim();
        if (1..=100).contains(&name.chars().count()) {
            Ok(Self(name.to_owned()))
        } else {
            Err(DomainError::InvalidAccountName)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartAccount {
    pub number: AccountNumber,
    pub name: AccountName,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChartEvent {
    /// The whole starting chart, spelled out so the history reads the same
    /// even after the built-in selection changes.
    ChartSeeded {
        accounts: Vec<ChartAccount>,
    },
    AccountAdded {
        number: AccountNumber,
        name: AccountName,
    },
    AccountRenamed {
        number: AccountNumber,
        name: AccountName,
    },
    AccountDeactivated {
        number: AccountNumber,
    },
    AccountReactivated {
        number: AccountNumber,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub number: AccountNumber,
    pub name: AccountName,
    pub active: bool,
}

/// A company's chart of accounts. Accounts are never removed, only
/// deactivated.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chart {
    accounts: BTreeMap<AccountNumber, Account>,
}

impl Chart {
    pub fn from_events(events: &[ChartEvent]) -> Self {
        let mut chart = Self::default();
        for event in events {
            chart.apply(event);
        }
        chart
    }

    pub fn apply(&mut self, event: &ChartEvent) {
        match event.clone() {
            ChartEvent::ChartSeeded { accounts } => {
                for ChartAccount { number, name } in accounts {
                    self.accounts.insert(
                        number,
                        Account {
                            number,
                            name,
                            active: true,
                        },
                    );
                }
            }
            ChartEvent::AccountAdded { number, name } => {
                self.accounts.insert(
                    number,
                    Account {
                        number,
                        name,
                        active: true,
                    },
                );
            }
            ChartEvent::AccountRenamed { number, name } => {
                if let Some(account) = self.accounts.get_mut(&number) {
                    account.name = name;
                }
            }
            ChartEvent::AccountDeactivated { number } => self.set_active(number, false),
            ChartEvent::AccountReactivated { number } => self.set_active(number, true),
        }
    }

    fn set_active(&mut self, number: AccountNumber, active: bool) {
        if let Some(account) = self.accounts.get_mut(&number) {
            account.active = active;
        }
    }

    pub fn get(&self, number: AccountNumber) -> Option<&Account> {
        self.accounts.get(&number)
    }

    /// By number.
    pub fn accounts(&self) -> impl Iterator<Item = &Account> {
        self.accounts.values()
    }
}

/// The built-in BAS selection every company starts with.
pub fn seed_chart() -> ChartEvent {
    ChartEvent::ChartSeeded {
        accounts: crate::bas::ACCOUNTS
            .iter()
            .map(|&(number, name)| ChartAccount {
                number: AccountNumber::parse(number.into()).expect("BAS numbers are valid"),
                name: AccountName::parse(name).expect("BAS names are valid"),
            })
            .collect(),
    }
}

pub fn add_account(
    chart: &Chart,
    number: AccountNumber,
    name: AccountName,
) -> Result<Vec<ChartEvent>, DomainError> {
    if chart.get(number).is_some() {
        return Err(DomainError::AccountExists);
    }
    Ok(vec![ChartEvent::AccountAdded { number, name }])
}

/// Renaming to the current name yields no events.
pub fn rename_account(
    chart: &Chart,
    number: AccountNumber,
    name: AccountName,
) -> Result<Vec<ChartEvent>, DomainError> {
    let account = chart.get(number).ok_or(DomainError::AccountNotFound)?;
    if account.name == name {
        return Ok(vec![]);
    }
    Ok(vec![ChartEvent::AccountRenamed { number, name }])
}

/// Idempotent: an account already in the wanted state yields no events.
pub fn set_account_active(
    chart: &Chart,
    number: AccountNumber,
    active: bool,
) -> Result<Vec<ChartEvent>, DomainError> {
    let account = chart.get(number).ok_or(DomainError::AccountNotFound)?;
    Ok(match (account.active, active) {
        (true, false) => vec![ChartEvent::AccountDeactivated { number }],
        (false, true) => vec![ChartEvent::AccountReactivated { number }],
        _ => vec![],
    })
}
