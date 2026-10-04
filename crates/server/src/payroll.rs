//! `doris.payroll.v1.PayrollService`: maps gRPC calls onto `doris_payroll`.
//! Every call needs a session, and a company the caller isn't a member of
//! looks exactly like one that doesn't exist. Personnummer and names are
//! personal data and never logged.

use crate::grpc::{now, signed_in_user, today};
use crate::skatteverket::TaxTables;
use doris_ledger::domain::VoucherLine;
use doris_payroll::agi::{AgiChange, AgiMonth, AgiStatus, Period};
use doris_payroll::domain::{
    BookedVoucher, DomainError, DraftLine, Employee, PayrollRunDraft, PayrollRunLine,
    PayrollRunStatus,
};
use doris_payroll::tax::{TaxBasis, TaxSetting};
use doris_payroll::{AgiMonthSummary, Error, NewEmployee, PayrollRunView};
use doris_proto::ledger::v1 as lpb;
use doris_proto::payroll::v1 as pb;
use doris_proto::payroll::v1::payroll_service_server::PayrollService;
use jiff::civil::Date;
use sqlx::SqlitePool;
use std::collections::HashMap;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub struct PayrollApi {
    pool: SqlitePool,
    tax_tables: TaxTables,
}

impl PayrollApi {
    pub fn new(pool: SqlitePool, tax_tables: TaxTables) -> Self {
        Self { pool, tax_tables }
    }

    /// Runs `call`; when the pay date's tax table isn't stored, fetches it
    /// from Skatteverket, stores it and runs `call` once more. The fetch
    /// happens outside any write transaction.
    async fn with_tax_table<T, F, Fut>(&self, call: F) -> Result<T, Status>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = doris_payroll::Result<T>>,
    {
        match call().await {
            Err(Error::Domain(DomainError::TaxTableMissing(year))) => {
                let table = self.tax_tables.fetch(year).await.map_err(|reason| {
                    tracing::warn!("tax table: {reason}");
                    Status::unavailable("tax_table_unavailable")
                })?;
                doris_payroll::store_tax_table(&self.pool, &table)
                    .await
                    .map_err(status)?;
                call().await.map_err(status)
            }
            result => result.map_err(status),
        }
    }

    /// The signed-in user and the company id they asked about. Membership
    /// is checked by `doris_payroll` itself.
    async fn caller<T>(
        &self,
        request: &Request<T>,
        company_id: &str,
    ) -> Result<(Uuid, Uuid), Status> {
        let user = signed_in_user(&self.pool, request).await?;
        let company: Uuid = company_id
            .parse()
            .map_err(|_| Status::not_found("company_not_found"))?;
        Ok((company, user.id))
    }

    /// Employee names by id, for lines that carry only the id.
    async fn names(&self, company: Uuid, user: Uuid) -> Result<HashMap<Uuid, String>, Status> {
        Ok(doris_payroll::list_employees(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(|e| (e.id, e.name.as_str().to_owned()))
            .collect())
    }
}

#[tonic::async_trait]
impl PayrollService for PayrollApi {
    async fn list_employees(
        &self,
        request: Request<pb::ListEmployeesRequest>,
    ) -> Result<Response<pb::ListEmployeesResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let employees = doris_payroll::list_employees(&self.pool, company, user)
            .await
            .map_err(status)?
            .iter()
            .map(employee_message)
            .collect();
        Ok(Response::new(pb::ListEmployeesResponse { employees }))
    }

    async fn add_employee(
        &self,
        request: Request<pb::AddEmployeeRequest>,
    ) -> Result<Response<pb::AddEmployeeResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let new = NewEmployee {
            name: &req.name,
            personal_identity_number: &req.personal_identity_number,
            monthly_salary: req.monthly_salary,
            salary_account: req.salary_account,
            tax: tax_setting(req.tax)?,
        };
        let employee_id = doris_payroll::add_employee(&self.pool, company, user, new)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AddEmployeeResponse {
            employee_id: employee_id.to_string(),
        }))
    }

    async fn update_employee(
        &self,
        request: Request<pb::UpdateEmployeeRequest>,
    ) -> Result<Response<pb::UpdateEmployeeResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_payroll::update_employee(
            &self.pool,
            company,
            user,
            employee_id(&req.employee_id)?,
            &req.name,
            req.monthly_salary,
            req.salary_account,
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::UpdateEmployeeResponse {}))
    }

    async fn set_employee_tax(
        &self,
        request: Request<pb::SetEmployeeTaxRequest>,
    ) -> Result<Response<pb::SetEmployeeTaxResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        // A setting can be changed, not removed: none is not a valid input.
        let tax =
            tax_setting(req.tax)?.ok_or_else(|| Status::invalid_argument("invalid_tax_table"))?;
        doris_payroll::set_employee_tax(
            &self.pool,
            company,
            user,
            employee_id(&req.employee_id)?,
            tax,
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::SetEmployeeTaxResponse {}))
    }

    async fn deactivate_employee(
        &self,
        request: Request<pb::DeactivateEmployeeRequest>,
    ) -> Result<Response<pb::DeactivateEmployeeResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let id = employee_id(&request.get_ref().employee_id)?;
        doris_payroll::deactivate_employee(&self.pool, company, user, id)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::DeactivateEmployeeResponse {}))
    }

    async fn preview_payroll_run(
        &self,
        request: Request<pb::PreviewPayrollRunRequest>,
    ) -> Result<Response<pb::PreviewPayrollRunResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let draft = draft(request.into_inner().draft)?;
        let preview = self
            .with_tax_table(|| {
                doris_payroll::preview_payroll_run(&self.pool, company, user, draft.clone())
            })
            .await?;
        let names = self.names(company, user).await?;
        Ok(Response::new(pb::PreviewPayrollRunResponse {
            text: preview.text,
            lines: preview
                .lines
                .iter()
                .map(|l| {
                    locked_line_message(l, names.get(&l.employee_id).cloned().unwrap_or_default())
                })
                .collect(),
            voucher_lines: doris_payroll::domain::voucher_lines(&preview.lines)
                .iter()
                .map(voucher_line_message)
                .collect(),
        }))
    }

    async fn create_payroll_run(
        &self,
        request: Request<pb::CreatePayrollRunRequest>,
    ) -> Result<Response<pb::CreatePayrollRunResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let draft = draft(request.into_inner().draft)?;
        let id = doris_payroll::create_payroll_run(&self.pool, company, user, draft)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::CreatePayrollRunResponse {
            payroll_run_id: id.to_string(),
        }))
    }

    async fn update_payroll_run(
        &self,
        request: Request<pb::UpdatePayrollRunRequest>,
    ) -> Result<Response<pb::UpdatePayrollRunResponse>, Status> {
        let run = request.get_ref().run.clone().unwrap_or_default();
        let (company, user) = self.caller(&request, &run.company_id).await?;
        let draft = draft(request.into_inner().draft)?;
        doris_payroll::update_payroll_run(
            &self.pool,
            company,
            user,
            run_id(&run.payroll_run_id)?,
            draft,
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::UpdatePayrollRunResponse {}))
    }

    async fn finalize_payroll_run(
        &self,
        request: Request<pb::PayrollRunRef>,
    ) -> Result<Response<pb::FinalizePayrollRunResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let run = run_id(&request.get_ref().payroll_run_id)?;
        self.with_tax_table(|| doris_payroll::finalize_payroll_run(&self.pool, company, user, run))
            .await?;
        Ok(Response::new(pb::FinalizePayrollRunResponse {}))
    }

    async fn reopen_payroll_run(
        &self,
        request: Request<pb::PayrollRunRef>,
    ) -> Result<Response<pb::ReopenPayrollRunResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let run = run_id(&request.get_ref().payroll_run_id)?;
        doris_payroll::reopen_payroll_run(&self.pool, company, user, run)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::ReopenPayrollRunResponse {}))
    }

    async fn book_payroll_run(
        &self,
        request: Request<pb::PayrollRunRef>,
    ) -> Result<Response<pb::VoucherRef>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let run = run_id(&request.get_ref().payroll_run_id)?;
        let booked = doris_payroll::book_payroll_run(&self.pool, company, user, run, today())
            .await
            .map_err(status)?;
        Ok(Response::new(voucher_message(booked)))
    }

    async fn unbook_payroll_run(
        &self,
        request: Request<pb::PayrollRunRef>,
    ) -> Result<Response<pb::VoucherRef>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let run = run_id(&request.get_ref().payroll_run_id)?;
        let correction = doris_payroll::unbook_payroll_run(&self.pool, company, user, run, today())
            .await
            .map_err(status)?;
        Ok(Response::new(voucher_message(correction)))
    }

    async fn get_payroll_run(
        &self,
        request: Request<pb::PayrollRunRef>,
    ) -> Result<Response<pb::PayrollRun>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let run = run_id(&request.get_ref().payroll_run_id)?;
        let view = doris_payroll::get_payroll_run(&self.pool, company, user, run)
            .await
            .map_err(status)?;
        Ok(Response::new(run_message(view)))
    }

    async fn list_payroll_runs(
        &self,
        request: Request<pb::ListPayrollRunsRequest>,
    ) -> Result<Response<pb::ListPayrollRunsResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let payroll_runs = doris_payroll::list_payroll_runs(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(run_message)
            .collect();
        Ok(Response::new(pb::ListPayrollRunsResponse { payroll_runs }))
    }

    async fn get_agi_contact(
        &self,
        request: Request<pb::GetAgiContactRequest>,
    ) -> Result<Response<pb::AgiContact>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let contact = doris_payroll::agi_contact(&self.pool, company, user)
            .await
            .map_err(status)?
            .map(|c| pb::AgiContact {
                name: c.name,
                phone: c.phone,
                email: c.email,
            })
            .unwrap_or_default();
        Ok(Response::new(contact))
    }

    async fn set_agi_contact(
        &self,
        request: Request<pb::SetAgiContactRequest>,
    ) -> Result<Response<pb::SetAgiContactResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let c = request.into_inner().contact.unwrap_or_default();
        doris_payroll::set_agi_contact(&self.pool, company, user, &c.name, &c.phone, &c.email)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::SetAgiContactResponse {}))
    }

    async fn list_agi_months(
        &self,
        request: Request<pb::ListAgiMonthsRequest>,
    ) -> Result<Response<pb::ListAgiMonthsResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let months = doris_payroll::agi_months(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(summary_message)
            .collect();
        Ok(Response::new(pb::ListAgiMonthsResponse { months }))
    }

    async fn get_agi_month(
        &self,
        request: Request<pb::AgiMonthRef>,
    ) -> Result<Response<pb::AgiMonth>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let period = period(&request.get_ref().period)?;
        let month = doris_payroll::agi_month(&self.pool, company, user, period)
            .await
            .map_err(status)?;
        let employees = doris_payroll::list_employees(&self.pool, company, user)
            .await
            .map_err(status)?;
        Ok(Response::new(month_message(month, &employees)))
    }

    async fn export_agi_file(
        &self,
        request: Request<pb::AgiMonthRef>,
    ) -> Result<Response<pb::AgiFile>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let period = period(&request.get_ref().period)?;
        let (file_name, xml) = doris_payroll::agi_file(&self.pool, company, user, period, now())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AgiFile { file_name, xml }))
    }

    async fn mark_agi_submitted(
        &self,
        request: Request<pb::AgiMonthRef>,
    ) -> Result<Response<pb::MarkAgiSubmittedResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let period = period(&request.get_ref().period)?;
        doris_payroll::submit_agi_month(&self.pool, company, user, period)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::MarkAgiSubmittedResponse {}))
    }
}

fn employee_message(e: &Employee) -> pb::Employee {
    pb::Employee {
        id: e.id.to_string(),
        name: e.name.as_str().to_owned(),
        personal_identity_number: e.personal_identity_number.formatted(),
        monthly_salary: e.monthly_salary,
        salary_account: e.salary_account.get(),
        active: e.active,
        tax: e.tax.map(tax_setting_message),
    }
}

fn locked_line_message(l: &PayrollRunLine, employee_name: String) -> pb::PayrollRunLine {
    pb::PayrollRunLine {
        employee_id: l.employee_id.to_string(),
        employee_name,
        gross: l.gross,
        tax: Some(l.tax),
        tax_basis: Some(tax_basis_message(l.tax_basis)),
        salary_account: l.salary_account.get(),
        fee_rate: l.fee_rate,
        fee: l.fee,
        net: l.net,
    }
}

fn voucher_line_message(l: &VoucherLine) -> lpb::VoucherLine {
    lpb::VoucherLine {
        account: l.account.get().into(),
        debit: l.debit,
        credit: l.credit,
    }
}

fn voucher_message(v: BookedVoucher) -> pb::VoucherRef {
    pb::VoucherRef {
        fiscal_year_start: v.fiscal_year_start.to_string(),
        number: v.number,
    }
}

fn run_message(view: PayrollRunView) -> pb::PayrollRun {
    let voucher_lines = view
        .voucher_lines()
        .iter()
        .map(voucher_line_message)
        .collect();
    let (status, voucher) = match view.status {
        PayrollRunStatus::Open => (pb::PayrollRunStatus::Open, None),
        PayrollRunStatus::Finalized => (pb::PayrollRunStatus::Finalized, None),
        PayrollRunStatus::Booked(v) => (pb::PayrollRunStatus::Booked, Some(voucher_message(v))),
    };
    pb::PayrollRun {
        id: view.id.to_string(),
        pay_date: view.pay_date.to_string(),
        text: view.text,
        status: status as i32,
        lines: view
            .lines
            .into_iter()
            .map(|l| match &l.locked {
                Some(locked) => locked_line_message(locked, l.employee_name),
                None => pb::PayrollRunLine {
                    employee_id: l.employee_id.to_string(),
                    employee_name: l.employee_name,
                    gross: l.gross,
                    tax: l.tax,
                    ..Default::default()
                },
            })
            .collect(),
        voucher_lines,
        voucher,
    }
}

fn draft(message: Option<pb::PayrollRunDraft>) -> Result<PayrollRunDraft, Status> {
    let message = message.unwrap_or_default();
    Ok(PayrollRunDraft {
        pay_date: date(&message.pay_date)?,
        text: message.text,
        lines: message
            .lines
            .iter()
            .map(|l| {
                Ok(DraftLine {
                    employee_id: employee_id(&l.employee_id)?,
                    gross: l.gross,
                    tax: l.tax,
                })
            })
            .collect::<Result<_, Status>>()?,
    })
}

fn tax_setting(message: Option<pb::TaxSetting>) -> Result<Option<TaxSetting>, Status> {
    match message.and_then(|m| m.kind) {
        None => Ok(None),
        Some(pb::tax_setting::Kind::Table(t)) => TaxSetting::table(t.table, t.column)
            .map(Some)
            .map_err(domain_status),
        Some(pb::tax_setting::Kind::Percent(p)) => {
            TaxSetting::percent(p).map(Some).map_err(domain_status)
        }
    }
}

fn tax_setting_message(setting: TaxSetting) -> pb::TaxSetting {
    let kind = match setting {
        TaxSetting::Table { table, column } => pb::tax_setting::Kind::Table(pb::TableTax {
            table: table.into(),
            column: column.into(),
        }),
        TaxSetting::Percent { percent } => pb::tax_setting::Kind::Percent(percent.into()),
    };
    pb::TaxSetting { kind: Some(kind) }
}

fn tax_basis_message(basis: TaxBasis) -> pb::TaxBasis {
    let kind = match basis {
        TaxBasis::Table {
            year,
            table,
            column,
        } => pb::tax_basis::Kind::Table(pb::TableBasis {
            year: u32::try_from(year).unwrap_or_default(),
            table: table.into(),
            column: column.into(),
        }),
        TaxBasis::Percent { percent } => pb::tax_basis::Kind::Percent(percent.into()),
        TaxBasis::Manual => pb::tax_basis::Kind::Manual(true),
    };
    pb::TaxBasis { kind: Some(kind) }
}

fn date(raw: &str) -> Result<Date, Status> {
    raw.parse()
        .map_err(|_| Status::invalid_argument("invalid_date"))
}

/// A malformed id names no employee.
fn employee_id(raw: &str) -> Result<Uuid, Status> {
    raw.parse()
        .map_err(|_| Status::not_found("employee_not_found"))
}

fn run_id(raw: &str) -> Result<Uuid, Status> {
    raw.parse()
        .map_err(|_| Status::not_found("payroll_run_not_found"))
}

fn domain_status(err: DomainError) -> Status {
    use DomainError::*;
    match err {
        InvalidPersonalIdentityNumber => {
            Status::invalid_argument("invalid_personal_identity_number")
        }
        InvalidEmployeeName => Status::invalid_argument("invalid_employee_name"),
        InvalidSalary => Status::invalid_argument("invalid_salary"),
        InvalidSalaryAccount => Status::invalid_argument("invalid_salary_account"),
        InvalidTax => Status::invalid_argument("invalid_tax"),
        // The ledger's code: the text becomes the voucher's.
        InvalidText => Status::invalid_argument("invalid_voucher_text"),
        EmptyPayrollRun => Status::invalid_argument("empty_payroll_run"),
        DuplicatePayrollRunLine => Status::invalid_argument("duplicate_payroll_run_line"),
        InvalidTaxTable => Status::invalid_argument("invalid_tax_table"),
        InvalidTaxPercent => Status::invalid_argument("invalid_tax_percent"),
        TaxRequired => Status::invalid_argument("tax_required"),
        TaxTableMissing(_) => Status::unavailable("tax_table_unavailable"),
        InvalidPeriod => Status::invalid_argument("invalid_period"),
        InvalidAgiContact => Status::invalid_argument("invalid_agi_contact"),
        AgiContactMissing => Status::failed_precondition("agi_contact_missing"),
        AgiPeriodEmpty => Status::failed_precondition("agi_period_empty"),
        AgiUnchanged => Status::failed_precondition("agi_unchanged"),
        DuplicateEmployee => Status::failed_precondition("duplicate_employee"),
        EmployeeInactive => Status::failed_precondition("employee_inactive"),
        PayrollRunNotOpen => Status::failed_precondition("payroll_run_not_open"),
        PayrollRunNotFinalized => Status::failed_precondition("payroll_run_not_finalized"),
        PayrollRunBooked => Status::failed_precondition("payroll_run_booked"),
        PayrollRunNotBooked => Status::failed_precondition("payroll_run_not_booked"),
        PayrollRunNotDue => Status::failed_precondition("payroll_run_not_due"),
        PayrollRunOutdated => Status::failed_precondition("payroll_run_outdated"),
        EmployeeNotFound => Status::not_found("employee_not_found"),
        PayrollRunNotFound => Status::not_found("payroll_run_not_found"),
    }
}

fn status(err: Error) -> Status {
    match err {
        Error::Domain(err) => domain_status(err),
        Error::NotFound => Status::not_found("company_not_found"),
        Error::Ledger(err) => crate::ledger::status(err),
        Error::Store(err) => {
            // sqlx messages name columns, never values: no personal data.
            tracing::error!("store: {err}");
            Status::internal("internal")
        }
    }
}

fn period(raw: &str) -> Result<Period, Status> {
    Period::parse(raw).map_err(domain_status)
}

fn status_message(status: AgiStatus) -> pb::AgiStatus {
    match status {
        AgiStatus::NotSubmitted => pb::AgiStatus::NotSubmitted,
        AgiStatus::Submitted => pb::AgiStatus::Submitted,
        AgiStatus::Changed => pb::AgiStatus::Changed,
    }
}

fn summary_message(s: AgiMonthSummary) -> pb::AgiMonthSummary {
    pb::AgiMonthSummary {
        period: s.period.to_string(),
        gross: s.gross,
        tax_sum: s.tax_sum,
        fee_sum: s.fee_sum,
        status: status_message(s.status) as i32,
        submitted_at: s.submitted_at.unwrap_or_default(),
    }
}

fn month_message(month: AgiMonth, employees: &[Employee]) -> pb::AgiMonth {
    let line = |l: &doris_payroll::agi::AgiLine, change: pb::AgiChange| {
        let e = employees.iter().find(|e| e.id == l.employee_id);
        pb::AgiLine {
            employee_id: l.employee_id.to_string(),
            employee_name: e.map(|e| e.name.as_str().to_owned()).unwrap_or_default(),
            personal_identity_number: e
                .map(|e| e.personal_identity_number.formatted())
                .unwrap_or_default(),
            specification_number: l.specification_number,
            gross: l.gross,
            tax: l.tax,
            change: change as i32,
        }
    };
    let changed = |c: AgiChange| match c {
        AgiChange::New => pb::AgiChange::New,
        AgiChange::Changed => pb::AgiChange::Changed,
        AgiChange::Unchanged => pb::AgiChange::Unchanged,
    };
    let lines = month
        .lines
        .iter()
        .map(|(l, c)| line(l, changed(*c)))
        .chain(
            month
                .removed
                .iter()
                .map(|l| line(l, pb::AgiChange::Removed)),
        )
        .collect();
    pb::AgiMonth {
        summary: Some(pb::AgiMonthSummary {
            period: month.period.to_string(),
            gross: month.lines.iter().map(|(l, _)| l.gross).sum(),
            tax_sum: month.tax_sum,
            fee_sum: month.fee_sum,
            status: status_message(month.status) as i32,
            submitted_at: String::new(),
        }),
        lines,
        booked_fees: month.booked_fees,
    }
}
