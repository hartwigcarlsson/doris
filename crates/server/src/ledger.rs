//! `doris.ledger.v1.LedgerService`: maps gRPC calls onto `doris_ledger`.
//! Every call needs a session, and a company the caller isn't a member of
//! looks exactly like one that doesn't exist.

use crate::grpc::{signed_in_user, today};
use doris_ledger::Error;
use doris_ledger::domain::{Attachment, DomainError, RecordVoucher, Voucher, VoucherLine};
use doris_ledger::statements::{LineKind, StatementLine};
use doris_proto::ledger::v1 as pb;
use doris_proto::ledger::v1::ledger_service_server::LedgerService;
use jiff::civil::Date;
use sqlx::SqlitePool;
use tonic::{Request, Response, Status};
use uuid::Uuid;

/// Two 10 MiB underlag plus the rest of a `RecordVoucher`.
pub(crate) const MAX_REQUEST: usize = 21 << 20;
/// One 10 MiB underlag in a `GetAttachment` answer.
pub(crate) const MAX_RESPONSE: usize = 11 << 20;
/// The most underlag data one request may carry, all files together.
const MAX_ATTACHMENTS_PER_REQUEST: usize = 20 << 20;

pub struct LedgerApi {
    pub(crate) pool: SqlitePool,
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
        let attachments = new_attachments(req.attachments)?;
        let booked = doris_ledger::record_voucher_with_attachments(
            &self.pool,
            company,
            user,
            cmd,
            attachments,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::RecordVoucherResponse {
            fiscal_year_start: booked.fiscal_year_start.to_string(),
            number: booked.number,
        }))
    }

    async fn add_attachment(
        &self,
        request: Request<pb::AddAttachmentRequest>,
    ) -> Result<Response<pb::AddAttachmentResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        // A missing attachment arrives as an empty name and is refused as such.
        let new = req.attachment.unwrap_or_default();
        let added = doris_ledger::add_attachment(
            &self.pool,
            company,
            user,
            date(&req.fiscal_year_start)?,
            req.number,
            doris_ledger::NewAttachment {
                file_name: new.file_name,
                data: new.data,
            },
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::AddAttachmentResponse {
            attachment: Some(attachment_message(&added)),
        }))
    }

    async fn get_attachment(
        &self,
        request: Request<pb::GetAttachmentRequest>,
    ) -> Result<Response<pb::GetAttachmentResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.get_ref();
        let (attachment, data) = doris_ledger::get_attachment(
            &self.pool,
            company,
            user,
            date(&req.fiscal_year_start)?,
            req.number,
            &req.id,
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::GetAttachmentResponse {
            attachment: Some(attachment_message(&attachment)),
            data,
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

    async fn get_financial_statements(
        &self,
        request: Request<pb::GetFinancialStatementsRequest>,
    ) -> Result<Response<pb::GetFinancialStatementsResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let fiscal_year_start = date(&request.get_ref().fiscal_year_start)?;
        let statements = doris_ledger::financial_statements(
            &self.pool,
            company,
            user,
            fiscal_year_start,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::GetFinancialStatementsResponse {
            income_statement: statements.income.into_iter().map(statement_line).collect(),
            balance_sheet: statements.balance.into_iter().map(statement_line).collect(),
            previous_fiscal_year_start: statements
                .previous_fiscal_year_start
                .map(|start| start.to_string())
                .unwrap_or_default(),
            difference: statements.difference,
            previous_difference: statements.previous_difference,
        }))
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

/// The uploaded files, refused as a whole if together they are over the
/// per-request limit. Each file is checked by `doris_ledger`.
fn new_attachments(
    files: Vec<pb::NewAttachment>,
) -> Result<Vec<doris_ledger::NewAttachment>, Status> {
    if files.iter().map(|f| f.data.len()).sum::<usize>() > MAX_ATTACHMENTS_PER_REQUEST {
        return Err(domain_status(DomainError::AttachmentTooLarge));
    }
    Ok(files
        .into_iter()
        .map(|f| doris_ledger::NewAttachment {
            file_name: f.file_name,
            data: f.data,
        })
        .collect())
}

fn attachment_message(a: &Attachment) -> pb::Attachment {
    pb::Attachment {
        id: a.sha256.clone(),
        file_name: a.file_name.as_str().to_owned(),
        content_type: a.content_type.as_mime().to_owned(),
        size: a.size,
    }
}

fn voucher_message(v: Voucher) -> pb::Voucher {
    pb::Voucher {
        number: v.number,
        date: v.date.to_string(),
        text: v.text,
        lines: v.lines.iter().map(line_message).collect(),
        corrects: v.corrects.unwrap_or(0),
        corrected_by: v.corrected_by.unwrap_or(0),
        attachments: v.attachments.iter().map(attachment_message).collect(),
    }
}

fn statement_line(line: StatementLine) -> pb::StatementLine {
    let kind = match line.kind {
        LineKind::Heading => pb::StatementLineKind::Heading,
        LineKind::Item => pb::StatementLineKind::Item,
        LineKind::Subtotal => pb::StatementLineKind::Subtotal,
    };
    pb::StatementLine {
        label: line.label.into(),
        kind: kind as i32,
        amount: line.amount,
        previous: line.previous,
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
        UnsupportedAttachmentType => Status::invalid_argument("unsupported_attachment_type"),
        InvalidAttachmentName => Status::invalid_argument("invalid_attachment_name"),
        EmptyAttachment => Status::invalid_argument("empty_attachment"),
        AttachmentTooLarge => Status::invalid_argument("attachment_too_large"),
        DuplicateAttachment => Status::invalid_argument("duplicate_attachment"),
        AttachmentNotFound => Status::not_found("attachment_not_found"),
        Overflow => {
            tracing::error!("ledger: amount overflow");
            Status::internal("internal")
        }
    }
}

pub(crate) fn status(err: Error) -> Status {
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
