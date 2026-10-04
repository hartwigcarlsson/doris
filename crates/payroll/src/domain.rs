//! Pure payroll rules: employees, payroll runs and arbetsgivaravgifter.
//! No I/O, no clock.

use jiff::civil::{Date, date};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

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
    #[error("an employee with this personnummer exists")]
    DuplicateEmployee,
    #[error("no such employee")]
    EmployeeNotFound,
    #[error("the employee is inactive")]
    EmployeeInactive,
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

/// Full arbetsgivaravgift, in basis points of the gross salary.
pub const FULL_RATE: u32 = 3142;
/// Only ålderspensionsavgift: for those who were 67 when the year began.
pub const OLD_AGE_RATE: u32 = 1021;
/// The temporary reduction for 19–23-year-olds, on the first 25 000 kr a month.
pub const YOUTH_RATE: u32 = 2081;
const YOUTH_CAP: i64 = 2_500_000;
const YOUTH_FROM: Date = date(2026, 4, 1);
const YOUTH_UNTIL: Date = date(2027, 9, 30);

/// Arbetsgivaravgift on `gross` paid on `pay_date` to someone born in
/// `birth_year`, after `earlier_gross_same_month` already paid to them in
/// that calendar month. Returns the rate (for the youth reduction, the
/// rate under the cap) and the fee in öre, half an öre rounded up.
///
/// Skatteverket, "Arbetsgivaravgifter" (2026).
// ponytail: the rules live in code from 2026; a changed rate needs a new
// release. A table of rates by date pays off only if they change more
// often than Doris is released.
pub fn employer_fee(
    birth_year: i16,
    pay_date: Date,
    gross: i64,
    earlier_gross_same_month: i64,
) -> (u32, i64) {
    let year = pay_date.year();
    let (rate, under_cap) = if birth_year <= 1937 {
        (0, gross)
    } else if birth_year <= year - 68 {
        (OLD_AGE_RATE, gross)
    } else if (year - 23..=year - 19).contains(&birth_year)
        && (YOUTH_FROM..=YOUTH_UNTIL).contains(&pay_date)
    {
        let left = (YOUTH_CAP - earlier_gross_same_month).max(0);
        (YOUTH_RATE, gross.min(left))
    } else {
        (FULL_RATE, gross)
    };
    let over_cap = gross - under_cap;
    let fee = (under_cap * i64::from(rate) + over_cap * i64::from(FULL_RATE) + 5_000) / 10_000;
    (rate, fee)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PayrollEvent {
    EmployeeAdded {
        employee_id: Uuid,
        name: EmployeeName,
        personal_identity_number: PersonalIdentityNumber,
        monthly_salary: i64,
        salary_account: SalaryAccount,
    },
    EmployeeUpdated {
        employee_id: Uuid,
        name: EmployeeName,
        monthly_salary: i64,
        salary_account: SalaryAccount,
    },
    EmployeeDeactivated {
        employee_id: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Employee {
    pub id: Uuid,
    pub name: EmployeeName,
    pub personal_identity_number: PersonalIdentityNumber,
    pub monthly_salary: i64,
    pub salary_account: SalaryAccount,
    pub active: bool,
}

/// A voucher a payroll run was booked as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BookedVoucher {
    pub fiscal_year_start: Date,
    pub number: u32,
}

/// Filled in with the payroll run events.
#[derive(Debug, Clone, PartialEq)]
pub struct PayrollRun {
    pub id: Uuid,
}

/// A company's employees and payroll runs. `reversed` holds the ledger
/// vouchers that have been corrected (rättade): a run booked as one of
/// them is no longer booked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Payroll {
    pub employees: Vec<Employee>,
    pub runs: Vec<PayrollRun>,
    pub reversed: HashSet<BookedVoucher>,
}

impl Payroll {
    pub fn from_events(events: &[PayrollEvent], reversed: HashSet<BookedVoucher>) -> Self {
        let mut payroll = Self {
            reversed,
            ..Self::default()
        };
        for event in events {
            payroll.apply(event);
        }
        payroll
    }

    pub fn apply(&mut self, event: &PayrollEvent) {
        match event.clone() {
            PayrollEvent::EmployeeAdded {
                employee_id,
                name,
                personal_identity_number,
                monthly_salary,
                salary_account,
            } => self.employees.push(Employee {
                id: employee_id,
                name,
                personal_identity_number,
                monthly_salary,
                salary_account,
                active: true,
            }),
            PayrollEvent::EmployeeUpdated {
                employee_id,
                name,
                monthly_salary,
                salary_account,
            } => {
                if let Some(e) = self.employee_mut(employee_id) {
                    e.name = name;
                    e.monthly_salary = monthly_salary;
                    e.salary_account = salary_account;
                }
            }
            PayrollEvent::EmployeeDeactivated { employee_id } => {
                if let Some(e) = self.employee_mut(employee_id) {
                    e.active = false;
                }
            }
        }
    }

    pub fn employee(&self, id: Uuid) -> Option<&Employee> {
        self.employees.iter().find(|e| e.id == id)
    }

    fn employee_mut(&mut self, id: Uuid) -> Option<&mut Employee> {
        self.employees.iter_mut().find(|e| e.id == id)
    }

    fn active_employee(&self, id: Uuid) -> Result<&Employee, DomainError> {
        match self.employee(id) {
            None => Err(DomainError::EmployeeNotFound),
            Some(e) if !e.active => Err(DomainError::EmployeeInactive),
            Some(e) => Ok(e),
        }
    }
}

fn check_salary(ore: i64) -> Result<i64, DomainError> {
    if (1..=MAX_AMOUNT).contains(&ore) {
        Ok(ore)
    } else {
        Err(DomainError::InvalidSalary)
    }
}

#[derive(Debug, Clone)]
pub struct AddEmployee {
    pub employee_id: Uuid,
    pub name: EmployeeName,
    pub personal_identity_number: PersonalIdentityNumber,
    pub monthly_salary: i64,
    pub salary_account: SalaryAccount,
}

/// A personnummer is unique in the company, also among inactive
/// employees; the `employees` projection's UNIQUE backs this up.
pub fn add_employee(payroll: &Payroll, cmd: AddEmployee) -> Result<Vec<PayrollEvent>, DomainError> {
    check_salary(cmd.monthly_salary)?;
    if payroll
        .employees
        .iter()
        .any(|e| e.personal_identity_number == cmd.personal_identity_number)
    {
        return Err(DomainError::DuplicateEmployee);
    }
    Ok(vec![PayrollEvent::EmployeeAdded {
        employee_id: cmd.employee_id,
        name: cmd.name,
        personal_identity_number: cmd.personal_identity_number,
        monthly_salary: cmd.monthly_salary,
        salary_account: cmd.salary_account,
    }])
}

#[derive(Debug, Clone)]
pub struct UpdateEmployee {
    pub employee_id: Uuid,
    pub name: EmployeeName,
    pub monthly_salary: i64,
    pub salary_account: SalaryAccount,
}

/// The personnummer never changes: a wrong one is a new employee.
pub fn update_employee(
    payroll: &Payroll,
    cmd: UpdateEmployee,
) -> Result<Vec<PayrollEvent>, DomainError> {
    check_salary(cmd.monthly_salary)?;
    let e = payroll.active_employee(cmd.employee_id)?;
    if (&e.name, e.monthly_salary, e.salary_account)
        == (&cmd.name, cmd.monthly_salary, cmd.salary_account)
    {
        return Ok(vec![]);
    }
    Ok(vec![PayrollEvent::EmployeeUpdated {
        employee_id: cmd.employee_id,
        name: cmd.name,
        monthly_salary: cmd.monthly_salary,
        salary_account: cmd.salary_account,
    }])
}

pub fn deactivate_employee(
    payroll: &Payroll,
    employee_id: Uuid,
) -> Result<Vec<PayrollEvent>, DomainError> {
    let e = payroll
        .employee(employee_id)
        .ok_or(DomainError::EmployeeNotFound)?;
    if !e.active {
        return Ok(vec![]);
    }
    Ok(vec![PayrollEvent::EmployeeDeactivated { employee_id }])
}
