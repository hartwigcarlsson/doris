//! `doris.vat.v1.VatService`: maps gRPC calls onto `doris_vat`. Every call
//! needs a session, and a company the caller isn't a member of looks
//! exactly like one that doesn't exist. An org nr can be a personnummer:
//! never logged.

use crate::grpc::{signed_in_user, today};
use doris_proto::vat::v1 as pb;
use doris_proto::vat::v1::vat_service_server::VatService;
use doris_vat::domain::{DomainError, VatStatus};
use doris_vat::period::VatPeriodKind;
use doris_vat::{Error, PeriodSummary};
use jiff::civil::{Date, date};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub struct VatApi {
    pool: SqlitePool,
}

impl VatApi {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

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
}

#[tonic::async_trait]
impl VatService for VatApi {
    async fn set_vat_period(
        &self,
        request: Request<pb::SetVatPeriodRequest>,
    ) -> Result<Response<pb::SetVatPeriodResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let kind = match pb::VatPeriodKind::try_from(req.kind) {
            Ok(pb::VatPeriodKind::Monthly) => VatPeriodKind::Monthly,
            Ok(pb::VatPeriodKind::Quarterly) => VatPeriodKind::Quarterly,
            Ok(pb::VatPeriodKind::Yearly) => VatPeriodKind::Yearly,
            Ok(pb::VatPeriodKind::NotRegistered) => VatPeriodKind::NotRegistered,
            _ => return Err(domain_status(DomainError::InvalidVatPeriod)),
        };
        doris_vat::set_vat_period(
            &self.pool,
            company,
            user,
            crate::ledger::date(&req.fiscal_year_start)?,
            kind,
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::SetVatPeriodResponse {}))
    }

    async fn list_vat_returns(
        &self,
        request: Request<pb::ListVatReturnsRequest>,
    ) -> Result<Response<pb::ListVatReturnsResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let start = crate::ledger::date(&request.get_ref().fiscal_year_start)?;
        let year = doris_vat::list_vat_returns(&self.pool, company, user, start, today())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::ListVatReturnsResponse {
            kind: kind_message(year.kind) as i32,
            locked: year.locked,
            periods: year.periods.iter().map(summary_message).collect(),
        }))
    }

    async fn get_vat_return(
        &self,
        request: Request<pb::VatReturnRef>,
    ) -> Result<Response<pb::VatReturn>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let end = period_end(&request.get_ref().period)?;
        let view = doris_vat::get_vat_return(&self.pool, company, user, end, today())
            .await
            .map_err(status)?;
        // Identity knows the names; once per person.
        let mut names = HashMap::<Uuid, String>::new();
        for id in view.submissions.iter().filter_map(|s| s.submitted_by) {
            if let Entry::Vacant(entry) = names.entry(id) {
                let name = doris_identity::get_user(&self.pool, id)
                    .await
                    .map_err(crate::grpc::status)?
                    .map(|u| u.display_name.as_str().to_owned())
                    .unwrap_or_default();
                entry.insert(name);
            }
        }
        let mut boxes: Vec<pb::VatBoxAmount> = Vec::new();
        for t in &view.totals {
            let account = pb::VatBoxAccount {
                number: t.number.into(),
                name: t.name.clone(),
                amount: doris_vat::domain::signed(&doris_vat::domain::AccountSaldo {
                    account: t.number,
                    vat_box: t.vat_box,
                    saldo: t.saldo,
                }),
            };
            let n = u32::from(t.vat_box);
            match boxes.iter_mut().find(|b| b.r#box == n) {
                Some(b) => b.accounts.push(account),
                None => boxes.push(pb::VatBoxAmount {
                    r#box: n,
                    amount: view.boxes.get(t.vat_box.get()),
                    accounts: vec![account],
                }),
            }
        }
        boxes.sort_by_key(|b| b.r#box);
        Ok(Response::new(pb::VatReturn {
            summary: Some(summary_message(&view.summary)),
            kind: kind_message(view.kind) as i32,
            org_nr: view.org_nr,
            vat_number: view.vat_number,
            boxes,
            booked_vat: view.booked_vat,
            fingerprint: view.fingerprint,
            submissions: view
                .submissions
                .iter()
                .map(|s| pb::VatSubmission {
                    submitted_at: s.submitted_at.clone(),
                    submitted_by_name: s
                        .submitted_by
                        .and_then(|id| names.get(&id).cloned())
                        .unwrap_or_default(),
                    fiscal_year_start: s
                        .submission
                        .voucher
                        .map(|v| v.fiscal_year_start.to_string())
                        .unwrap_or_default(),
                    voucher_number: s.submission.voucher.map_or(0, |v| v.number),
                    corrected: s.corrected,
                })
                .collect(),
        }))
    }

    async fn export_vat_file(
        &self,
        request: Request<pb::VatReturnRef>,
    ) -> Result<Response<pb::VatFile>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let end = period_end(&request.get_ref().period)?;
        let (file_name, content, fingerprint) =
            doris_vat::export_vat_file(&self.pool, company, user, end, today())
                .await
                .map_err(status)?;
        Ok(Response::new(pb::VatFile {
            file_name,
            content,
            fingerprint,
        }))
    }

    async fn mark_vat_return_submitted(
        &self,
        request: Request<pb::MarkVatReturnSubmittedRequest>,
    ) -> Result<Response<pb::MarkVatReturnSubmittedResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let voucher = doris_vat::mark_vat_return_submitted(
            &self.pool,
            company,
            user,
            period_end(&req.period)?,
            &req.fingerprint,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::MarkVatReturnSubmittedResponse {
            fiscal_year_start: voucher
                .map(|v| v.fiscal_year_start.to_string())
                .unwrap_or_default(),
            voucher_number: voucher.map_or(0, |v| v.number),
        }))
    }
}

/// "ÅÅÅÅMM" → the month's last day.
fn period_end(raw: &str) -> Result<Date, Status> {
    let invalid = || domain_status(DomainError::InvalidVatPeriod);
    if raw.len() != 6 || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let (year, month): (i16, i8) = (
        raw[..4].parse().map_err(|_| invalid())?,
        raw[4..].parse().map_err(|_| invalid())?,
    );
    if !(1..=12).contains(&month) || !(1900..=2999).contains(&year) {
        return Err(invalid());
    }
    Ok(date(year, month, 1).last_of_month())
}

fn kind_message(kind: VatPeriodKind) -> pb::VatPeriodKind {
    match kind {
        VatPeriodKind::Monthly => pb::VatPeriodKind::Monthly,
        VatPeriodKind::Quarterly => pb::VatPeriodKind::Quarterly,
        VatPeriodKind::Yearly => pb::VatPeriodKind::Yearly,
        VatPeriodKind::NotRegistered => pb::VatPeriodKind::NotRegistered,
    }
}

fn summary_message(s: &PeriodSummary) -> pb::VatPeriodSummary {
    pb::VatPeriodSummary {
        period: s.period.code(),
        start: s.period.start.to_string(),
        end: s.period.end.to_string(),
        label: s.period.label(),
        status: match s.status {
            VatStatus::InProgress => pb::VatStatus::InProgress,
            VatStatus::ToSubmit => pb::VatStatus::ToSubmit,
            VatStatus::Submitted => pb::VatStatus::Submitted,
            VatStatus::Changed => pb::VatStatus::Changed,
        } as i32,
        due_date: s.due_date.map(|d| d.to_string()).unwrap_or_default(),
        vat_due: s.vat_due,
    }
}

fn domain_status(err: DomainError) -> Status {
    use DomainError::*;
    match err {
        InvalidVatPeriod => Status::invalid_argument("invalid_vat_period"),
        VatPeriodNotEnded => Status::failed_precondition("vat_period_not_ended"),
        VatPeriodLocked => Status::failed_precondition("vat_period_locked"),
        VatReturnOutdated => Status::failed_precondition("vat_return_outdated"),
        VatReturnUnchanged => Status::failed_precondition("vat_return_unchanged"),
        VatNotRegistered => Status::failed_precondition("vat_not_registered"),
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
