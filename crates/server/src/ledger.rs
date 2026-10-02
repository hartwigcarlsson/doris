//! `doris.ledger.v1.LedgerService`: maps gRPC calls onto `doris_ledger`.
//! Every call needs a session, and a company the caller isn't a member of
//! looks exactly like one that doesn't exist.

use crate::grpc::{signed_in_user, today};
use doris_ledger::Error;
use doris_ledger::domain::{DomainError, RecordVoucher, Voucher, VoucherLine};
use doris_proto::ledger::v1 as pb;
use doris_proto::ledger::v1::ledger_service_server::LedgerService;
use jiff::civil::Date;
use sqlx::SqlitePool;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub struct LedgerApi {
    pool: SqlitePool,
}

impl LedgerApi {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The signed-in user and the company id they asked about. Membership
    /// is checked by `doris_ledger` itself.
    async fn caller<T>(
        &self,
        request: &Request<T>,
        company_id: &str,
    ) -> Result<(Uuid, Uuid), Status> {
        let user = signed_in_user(&self.pool, request).await?;
        let company: Uuid = company_id.parse().map_err(|_| company_not_found())?;
        Ok((company, user.id))
    }
}

#[tonic::async_trait]
impl LedgerService for LedgerApi {
    async fn list_accounts(
        &self,
        request: Request<pb::ListAccountsRequest>,
    ) -> Result<Response<pb::ListAccountsResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let accounts = doris_ledger::list_accounts(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(|a| pb::Account {
                number: a.number.get().into(),
                name: a.name.as_str().to_owned(),
                active: a.active,
            })
            .collect();
        Ok(Response::new(pb::ListAccountsResponse { accounts }))
    }

    async fn add_account(
        &self,
        request: Request<pb::AddAccountRequest>,
    ) -> Result<Response<pb::AddAccountResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_ledger::add_account(&self.pool, company, user, req.number, &req.name)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AddAccountResponse {}))
    }

    async fn rename_account(
        &self,
        request: Request<pb::RenameAccountRequest>,
    ) -> Result<Response<pb::RenameAccountResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_ledger::rename_account(&self.pool, company, user, req.number, &req.name)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::RenameAccountResponse {}))
    }

    async fn set_account_active(
        &self,
        request: Request<pb::SetAccountActiveRequest>,
    ) -> Result<Response<pb::SetAccountActiveResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_ledger::set_account_active(&self.pool, company, user, req.number, req.active)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::SetAccountActiveResponse {}))
    }

    async fn list_fiscal_years(
        &self,
        request: Request<pb::ListFiscalYearsRequest>,
    ) -> Result<Response<pb::ListFiscalYearsResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let fiscal_years = doris_ledger::list_fiscal_years(&self.pool, company, user, today())
            .await
            .map_err(status)?
            .into_iter()
            .map(|y| pb::FiscalYear {
                start: y.fiscal_year.start.to_string(),
                end: y.fiscal_year.end.to_string(),
                closed: y.closed,
            })
            .collect();
        Ok(Response::new(pb::ListFiscalYearsResponse { fiscal_years }))
    }

    async fn record_voucher(
        &self,
        request: Request<pb::RecordVoucherRequest>,
    ) -> Result<Response<pb::RecordVoucherResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let cmd = RecordVoucher {
            date: date(&req.date)?,
            text: req.text,
            lines: domain_lines(&req.lines)?,
        };
        let booked = doris_ledger::record_voucher(&self.pool, company, user, cmd, today())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::RecordVoucherResponse {
            fiscal_year_start: booked.fiscal_year_start.to_string(),
            number: booked.number,
        }))
    }

    async fn correct_voucher(
        &self,
        request: Request<pb::CorrectVoucherRequest>,
    ) -> Result<Response<pb::CorrectVoucherResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let booked = doris_ledger::correct_voucher(
            &self.pool,
            company,
            user,
            date(&req.fiscal_year_start)?,
            req.number,
            date(&req.date)?,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::CorrectVoucherResponse {
            number: booked.number,
        }))
    }

    async fn list_vouchers(
        &self,
        request: Request<pb::ListVouchersRequest>,
    ) -> Result<Response<pb::ListVouchersResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let fiscal_year_start = date(&request.get_ref().fiscal_year_start)?;
        let vouchers = doris_ledger::list_vouchers(&self.pool, company, user, fiscal_year_start)
            .await
            .map_err(status)?
            .into_iter()
            .map(voucher_message)
            .collect();
        Ok(Response::new(pb::ListVouchersResponse { vouchers }))
    }

    async fn get_trial_balance(
        &self,
        request: Request<pb::GetTrialBalanceRequest>,
    ) -> Result<Response<pb::GetTrialBalanceResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let fiscal_year_start = date(&request.get_ref().fiscal_year_start)?;
        let rows = doris_ledger::trial_balance(&self.pool, company, user, fiscal_year_start)
            .await
            .map_err(status)?
            .into_iter()
            .map(|r| pb::TrialBalanceRow {
                account: r.account,
                name: r.name,
                debit: r.debit,
                credit: r.credit,
                opening: r.opening,
            })
            .collect();
        Ok(Response::new(pb::GetTrialBalanceResponse { rows }))
    }

    async fn get_account_ledger(
        &self,
        request: Request<pb::GetAccountLedgerRequest>,
    ) -> Result<Response<pb::GetAccountLedgerResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.get_ref();
        let fiscal_year_start = date(&req.fiscal_year_start)?;
        let ledger =
            doris_ledger::account_ledger(&self.pool, company, user, fiscal_year_start, req.account)
                .await
                .map_err(status)?;
        let entries = ledger
            .entries
            .into_iter()
            .map(|e| pb::LedgerEntry {
                date: e.date.to_string(),
                number: e.number,
                text: e.text,
                debit: e.debit,
                credit: e.credit,
                balance: e.balance,
            })
            .collect();
        Ok(Response::new(pb::GetAccountLedgerResponse {
            entries,
            opening: ledger.opening,
        }))
    }

    async fn get_opening_balances(
        &self,
        request: Request<pb::GetOpeningBalancesRequest>,
    ) -> Result<Response<pb::GetOpeningBalancesResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let lines = doris_ledger::opening_balances(&self.pool, company, user)
            .await
            .map_err(status)?
            .iter()
            .map(line_message)
            .collect();
        Ok(Response::new(pb::GetOpeningBalancesResponse { lines }))
    }

    async fn set_opening_balances(
        &self,
        request: Request<pb::SetOpeningBalancesRequest>,
    ) -> Result<Response<pb::SetOpeningBalancesResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let lines = domain_lines(&request.get_ref().lines)?;
        doris_ledger::set_opening_balances(&self.pool, company, user, lines)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::SetOpeningBalancesResponse {}))
    }

    async fn close_fiscal_year(
        &self,
        request: Request<pb::CloseFiscalYearRequest>,
    ) -> Result<Response<pb::CloseFiscalYearResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let start = date(&request.get_ref().fiscal_year_start)?;
        let result_voucher =
            doris_ledger::close_fiscal_year(&self.pool, company, user, start, today())
                .await
                .map_err(status)?;
        Ok(Response::new(pb::CloseFiscalYearResponse {
            result_voucher: result_voucher.unwrap_or(0),
        }))
    }

    async fn reopen_fiscal_year(
        &self,
        request: Request<pb::ReopenFiscalYearRequest>,
    ) -> Result<Response<pb::ReopenFiscalYearResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.get_ref();
        let start = date(&req.fiscal_year_start)?;
        doris_ledger::reopen_fiscal_year(&self.pool, company, user, start, &req.reason, today())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::ReopenFiscalYearResponse {}))
    }
}

fn line_message(l: &VoucherLine) -> pb::VoucherLine {
    pb::VoucherLine {
        account: l.account.get().into(),
        debit: l.debit,
        credit: l.credit,
    }
}

/// An out-of-range account is refused as `account_not_found`.
fn domain_lines(lines: &[pb::VoucherLine]) -> Result<Vec<VoucherLine>, Status> {
    lines
        .iter()
        .map(|l| VoucherLine::new(l.account, l.debit, l.credit))
        .collect::<Result<_, _>>()
        .map_err(domain_status)
}

fn voucher_message(v: Voucher) -> pb::Voucher {
    pb::Voucher {
        number: v.number,
        date: v.date.to_string(),
        text: v.text,
        lines: v.lines.iter().map(line_message).collect(),
        corrects: v.corrects.unwrap_or(0),
        corrected_by: v.corrected_by.unwrap_or(0),
    }
}

fn date(raw: &str) -> Result<Date, Status> {
    raw.parse()
        .map_err(|_| Status::invalid_argument("invalid_date"))
}

fn company_not_found() -> Status {
    Status::not_found("company_not_found")
}

fn domain_status(err: DomainError) -> Status {
    use DomainError::*;
    match err {
        InvalidAccountNumber => Status::invalid_argument("invalid_account_number"),
        InvalidAccountName => Status::invalid_argument("invalid_account_name"),
        AccountExists => Status::already_exists("account_exists"),
        AccountNotFound => Status::not_found("account_not_found"),
        AccountInactive => Status::failed_precondition("account_inactive"),
        InvalidVoucherText => Status::invalid_argument("invalid_voucher_text"),
        InvalidVoucherLines => Status::invalid_argument("invalid_voucher_lines"),
        InvalidAmount => Status::invalid_argument("invalid_amount"),
        VoucherUnbalanced => Status::invalid_argument("voucher_unbalanced"),
        VoucherDateInFuture => Status::invalid_argument("voucher_date_in_future"),
        VoucherDateBeforeFirstFiscalYear => {
            Status::invalid_argument("voucher_date_before_first_fiscal_year")
        }
        CorrectionDateOutsideFiscalYear => {
            Status::invalid_argument("correction_date_outside_fiscal_year")
        }
        VoucherNotFound => Status::not_found("voucher_not_found"),
        AlreadyCorrected => Status::failed_precondition("already_corrected"),
        CannotCorrectCorrection => Status::failed_precondition("cannot_correct_correction"),
        NotBalanceSheetAccount => Status::invalid_argument("not_balance_sheet_account"),
        DuplicateAccount => Status::invalid_argument("duplicate_account"),
        OpeningBalancesUnbalanced => Status::invalid_argument("opening_balances_unbalanced"),
        InvalidReason => Status::invalid_argument("invalid_reason"),
        FiscalYearNotFound => Status::not_found("fiscal_year_not_found"),
        FiscalYearClosed => Status::failed_precondition("fiscal_year_closed"),
        FiscalYearOpen => Status::failed_precondition("fiscal_year_open"),
        FiscalYearNotEnded => Status::failed_precondition("fiscal_year_not_ended"),
        PreviousFiscalYearOpen => Status::failed_precondition("previous_fiscal_year_open"),
        LaterFiscalYearClosed => Status::failed_precondition("later_fiscal_year_closed"),
        Overflow => {
            tracing::error!("ledger: amount overflow");
            Status::internal("internal")
        }
    }
}

fn status(err: Error) -> Status {
    match err {
        Error::Domain(err) => domain_status(err),
        Error::NotFound => company_not_found(),
        Error::Overflow => {
            tracing::error!("ledger: amount overflow");
            Status::internal("internal")
        }
        Error::Store(err) => {
            tracing::error!("store: {err}");
            Status::internal("internal")
        }
    }
}
