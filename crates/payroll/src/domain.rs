//! Pure payroll rules: employees, payroll runs and arbetsgivaravgifter.
//! No I/O, no clock.

use doris_ledger::domain::{AccountNumber, RecordVoucher, VoucherLine};
use jiff::civil::{Date, date};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
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
    #[error("tax must be 0 up to the gross salary")]
    InvalidTax,
    #[error("text must be at most 200 characters")]
    InvalidText,
    #[error("a payroll run needs at least one employee")]
    EmptyPayrollRun,
    #[error("an employee appears twice in the payroll run")]
    DuplicatePayrollRunLine,
    #[error("no such payroll run")]
    PayrollRunNotFound,
    #[error("the payroll run is finalized")]
    PayrollRunNotOpen,
    #[error("the payroll run is not finalized")]
    PayrollRunNotFinalized,
    #[error("the payroll run is booked")]
    PayrollRunBooked,
    #[error("the payroll run is not booked")]
    PayrollRunNotBooked,
    #[error("the pay date has not come")]
    PayrollRunNotDue,
    #[error("the fees changed since the payroll run was finalized")]
    PayrollRunOutdated,
    #[error("tax table must be 29-42 and column 1-6")]
    InvalidTaxTable,
    #[error("tax percentage must be 0-100")]
    InvalidTaxPercent,
    #[error("the line needs a tax or the employee a tax setting")]
    TaxRequired,
    /// The year's table isn't stored yet; the server fetches it.
    #[error("no tax table for {0}")]
    TaxTableMissing(i16),
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
    /// A new, open run.
    PayrollRunCreated {
        payroll_run_id: Uuid,
        draft: PayrollRunDraft,
    },
    /// The open run's contents replaced.
    PayrollRunUpdated {
        payroll_run_id: Uuid,
        draft: PayrollRunDraft,
    },
    /// Amounts, fees and accounts locked; may precede the pay date.
    PayrollRunFinalized {
        payroll_run_id: Uuid,
        lines: Vec<PayrollRunLine>,
    },
    /// Open again; the locked lines are dropped.
    PayrollRunReopened {
        payroll_run_id: Uuid,
    },
    /// Booked as a voucher, on or after the pay date.
    PayrollRunBooked {
        payroll_run_id: Uuid,
        voucher: BookedVoucher,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PayrollRunDraft {
    pub pay_date: Date,
    pub text: String,
    pub lines: Vec<DraftLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftLine {
    pub employee_id: Uuid,
    pub gross: i64,
    pub tax: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayrollRunLine {
    pub employee_id: Uuid,
    /// Copied from the employee when finalized, so a later change of
    /// account doesn't move the booking.
    pub salary_account: SalaryAccount,
    pub gross: i64,
    pub tax: i64,
    /// Basis points of the part under the youth cap, see `employer_fee`.
    pub fee_rate: u32,
    pub fee: i64,
    pub net: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PayrollRun {
    pub id: Uuid,
    pub draft: PayrollRunDraft,
    /// Set while finalized (and booked).
    pub lines: Option<Vec<PayrollRunLine>>,
    /// Every booking ever made, oldest first.
    pub bookings: Vec<BookedVoucher>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayrollRunStatus {
    Open,
    Finalized,
    /// The voucher in force.
    Booked(BookedVoucher),
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
            PayrollEvent::PayrollRunCreated {
                payroll_run_id,
                draft,
            } => self.runs.push(PayrollRun {
                id: payroll_run_id,
                draft,
                lines: None,
                bookings: vec![],
            }),
            PayrollEvent::PayrollRunUpdated {
                payroll_run_id,
                draft,
            } => {
                if let Some(run) = self.run_mut(payroll_run_id) {
                    run.draft = draft;
                }
            }
            PayrollEvent::PayrollRunFinalized {
                payroll_run_id,
                lines,
            } => {
                if let Some(run) = self.run_mut(payroll_run_id) {
                    run.lines = Some(lines);
                }
            }
            PayrollEvent::PayrollRunReopened { payroll_run_id } => {
                if let Some(run) = self.run_mut(payroll_run_id) {
                    run.lines = None;
                }
            }
            PayrollEvent::PayrollRunBooked {
                payroll_run_id,
                voucher,
            } => {
                if let Some(run) = self.run_mut(payroll_run_id) {
                    run.bookings.push(voucher);
                }
            }
        }
    }

    pub fn employee(&self, id: Uuid) -> Option<&Employee> {
        self.employees.iter().find(|e| e.id == id)
    }

    pub fn run(&self, id: Uuid) -> Option<&PayrollRun> {
        self.runs.iter().find(|r| r.id == id)
    }

    fn run_mut(&mut self, id: Uuid) -> Option<&mut PayrollRun> {
        self.runs.iter_mut().find(|r| r.id == id)
    }

    /// Booked while the latest booking's voucher is not corrected.
    pub fn status(&self, run: &PayrollRun) -> PayrollRunStatus {
        match run.bookings.last() {
            Some(voucher) if !self.reversed.contains(voucher) => PayrollRunStatus::Booked(*voucher),
            _ if run.lines.is_some() => PayrollRunStatus::Finalized,
            _ => PayrollRunStatus::Open,
        }
    }

    /// Gross paid to `employee_id` by booked runs other than `except` with
    /// a pay date in `pay_date`'s calendar month: what the youth cap counts.
    // ponytail: only booked runs count, and a booking whose fee would
    // change is refused (PayrollRunOutdated); fine while several runs a
    // month for an employee under 24 are rare.
    fn booked_gross(&self, employee_id: Uuid, pay_date: Date, except: Option<Uuid>) -> i64 {
        let same_month = |d: Date| (d.year(), d.month()) == (pay_date.year(), pay_date.month());
        self.runs
            .iter()
            .filter(|r| Some(r.id) != except && same_month(r.draft.pay_date))
            .filter(|r| matches!(self.status(r), PayrollRunStatus::Booked(_)))
            .flat_map(|r| r.lines.iter().flatten())
            .filter(|l| l.employee_id == employee_id)
            .map(|l| l.gross)
            .sum()
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

const MONTHS: [&str; 12] = [
    "januari",
    "februari",
    "mars",
    "april",
    "maj",
    "juni",
    "juli",
    "augusti",
    "september",
    "oktober",
    "november",
    "december",
];

/// The draft as it will be stored: lines checked, text trimmed or, when
/// empty, "Lön {månad} {år}". The pay date may be in the future.
pub fn validate_draft(
    payroll: &Payroll,
    draft: PayrollRunDraft,
) -> Result<PayrollRunDraft, DomainError> {
    if draft.lines.is_empty() {
        return Err(DomainError::EmptyPayrollRun);
    }
    let mut seen = HashSet::new();
    for line in &draft.lines {
        if !seen.insert(line.employee_id) {
            return Err(DomainError::DuplicatePayrollRunLine);
        }
        payroll.active_employee(line.employee_id)?;
        check_salary(line.gross)?;
        if !(0..=line.gross).contains(&line.tax) {
            return Err(DomainError::InvalidTax);
        }
    }
    let text = match draft.text.trim() {
        "" => format!(
            "Lön {} {}",
            MONTHS[draft.pay_date.month() as usize - 1],
            draft.pay_date.year()
        ),
        // The ledger's limit for a voucher text, refused before booking.
        text if text.chars().count() > 200 => return Err(DomainError::InvalidText),
        text => text.to_owned(),
    };
    Ok(PayrollRunDraft { text, ..draft })
}

/// A line's fee and net pay, the youth cap counting runs other than
/// `except`.
fn line(
    payroll: &Payroll,
    except: Option<Uuid>,
    pay_date: Date,
    employee: &Employee,
    gross: i64,
    tax: i64,
) -> PayrollRunLine {
    let earlier = payroll.booked_gross(employee.id, pay_date, except);
    let birth_year = employee.personal_identity_number.birth_year();
    let (fee_rate, fee) = employer_fee(birth_year, pay_date, gross, earlier);
    PayrollRunLine {
        employee_id: employee.id,
        salary_account: employee.salary_account,
        gross,
        tax,
        fee_rate,
        fee,
        net: gross - tax,
    }
}

/// The lines `draft` would lock if run `run_id` (or a new run, `None`)
/// were finalized now.
pub fn compute_lines(
    payroll: &Payroll,
    run_id: Option<Uuid>,
    draft: &PayrollRunDraft,
) -> Result<Vec<PayrollRunLine>, DomainError> {
    let draft = validate_draft(payroll, draft.clone())?;
    Ok(draft
        .lines
        .iter()
        .map(|l| {
            let employee = payroll.employee(l.employee_id).expect("validated");
            line(payroll, run_id, draft.pay_date, employee, l.gross, l.tax)
        })
        .collect())
}

/// The payroll voucher: gross on each salary account, tax on 2710, net
/// pay from 1930, the fee on 7510 against 2731. Zero lines are left out.
pub fn voucher_lines(lines: &[PayrollRunLine]) -> Vec<VoucherLine> {
    let mut salaries = BTreeMap::<u32, i64>::new();
    for l in lines {
        *salaries.entry(l.salary_account.get()).or_default() += l.gross;
    }
    let sum = |amount: fn(&PayrollRunLine) -> i64| lines.iter().map(amount).sum::<i64>();
    let (tax, net, fee) = (sum(|l| l.tax), sum(|l| l.net), sum(|l| l.fee));
    salaries
        .into_iter()
        .map(|(account, gross)| (account, gross, 0))
        .chain([
            (2710, 0, tax),
            (1930, 0, net),
            (7510, fee, 0),
            (2731, 0, fee),
        ])
        .filter(|&(_, debit, credit)| debit != 0 || credit != 0)
        .map(|(account, debit, credit)| VoucherLine {
            account: AccountNumber::parse(account).expect("payroll accounts are in range"),
            debit,
            credit,
        })
        .collect()
}

fn find_run(payroll: &Payroll, id: Uuid) -> Result<&PayrollRun, DomainError> {
    payroll.run(id).ok_or(DomainError::PayrollRunNotFound)
}

fn open_run(payroll: &Payroll, id: Uuid) -> Result<&PayrollRun, DomainError> {
    let run = find_run(payroll, id)?;
    match payroll.status(run) {
        PayrollRunStatus::Open => Ok(run),
        PayrollRunStatus::Finalized => Err(DomainError::PayrollRunNotOpen),
        PayrollRunStatus::Booked(_) => Err(DomainError::PayrollRunBooked),
    }
}

fn finalized_run(payroll: &Payroll, id: Uuid) -> Result<&PayrollRun, DomainError> {
    let run = find_run(payroll, id)?;
    match payroll.status(run) {
        PayrollRunStatus::Open => Err(DomainError::PayrollRunNotFinalized),
        PayrollRunStatus::Finalized => Ok(run),
        PayrollRunStatus::Booked(_) => Err(DomainError::PayrollRunBooked),
    }
}

pub fn create_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
    draft: PayrollRunDraft,
) -> Result<PayrollEvent, DomainError> {
    Ok(PayrollEvent::PayrollRunCreated {
        payroll_run_id,
        draft: validate_draft(payroll, draft)?,
    })
}

pub fn update_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
    draft: PayrollRunDraft,
) -> Result<PayrollEvent, DomainError> {
    open_run(payroll, payroll_run_id)?;
    Ok(PayrollEvent::PayrollRunUpdated {
        payroll_run_id,
        draft: validate_draft(payroll, draft)?,
    })
}

pub fn finalize_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
) -> Result<PayrollEvent, DomainError> {
    let run = open_run(payroll, payroll_run_id)?;
    Ok(PayrollEvent::PayrollRunFinalized {
        payroll_run_id,
        lines: compute_lines(payroll, Some(payroll_run_id), &run.draft)?,
    })
}

pub fn reopen_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
) -> Result<PayrollEvent, DomainError> {
    finalized_run(payroll, payroll_run_id)?;
    Ok(PayrollEvent::PayrollRunReopened { payroll_run_id })
}

/// The voucher a finalized run books on `today`, no earlier than its pay
/// date. The locked lines are booked as they are; only their fees are
/// checked again, since another run booked or reversed meanwhile can move
/// the youth cap.
pub fn book_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
    today: Date,
) -> Result<RecordVoucher, DomainError> {
    let run = finalized_run(payroll, payroll_run_id)?;
    let pay_date = run.draft.pay_date;
    if pay_date > today {
        return Err(DomainError::PayrollRunNotDue);
    }
    let locked = run.lines.as_deref().expect("a finalized run has lines");
    let fees_hold = locked.iter().all(|l| {
        let employee = payroll
            .employee(l.employee_id)
            .expect("a run's employees exist");
        let fresh = line(
            payroll,
            Some(payroll_run_id),
            pay_date,
            employee,
            l.gross,
            l.tax,
        );
        (fresh.fee_rate, fresh.fee) == (l.fee_rate, l.fee)
    });
    if !fees_hold {
        return Err(DomainError::PayrollRunOutdated);
    }
    Ok(RecordVoucher {
        date: pay_date,
        text: run.draft.text.clone(),
        lines: voucher_lines(locked),
    })
}

pub fn booked(payroll_run_id: Uuid, voucher: BookedVoucher) -> PayrollEvent {
    PayrollEvent::PayrollRunBooked {
        payroll_run_id,
        voucher,
    }
}

/// The voucher to correct to take a booked run back to Färdigställd.
pub fn unbook_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
) -> Result<BookedVoucher, DomainError> {
    match payroll.status(find_run(payroll, payroll_run_id)?) {
        PayrollRunStatus::Booked(voucher) => Ok(voucher),
        _ => Err(DomainError::PayrollRunNotBooked),
    }
}
