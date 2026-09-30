//! `doris.company.v1.CompanyService`: maps gRPC calls onto `doris_company`.
//! Every call needs a session, and a company the caller isn't a member of
//! looks exactly like one that doesn't exist.

use crate::grpc::{self, signed_in_user};
use doris_company::domain::{AccountingMethod, Address, Company, DomainError, LegalForm};
use doris_company::{Error, NewCompany};
use doris_proto::company::v1 as pb;
use doris_proto::company::v1::company_service_server::CompanyService;
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use sqlx::SqlitePool;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub struct CompanyApi {
    pool: SqlitePool,
}

impl CompanyApi {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The company, if the signed-in user is a member of it.
    async fn member_company<T>(
        &self,
        request: &Request<T>,
        company_id: &str,
    ) -> Result<(Company, Uuid), Status> {
        let user = signed_in_user(&self.pool, request).await?;
        let id: Uuid = company_id.parse().map_err(|_| company_not_found())?;
        let company = doris_company::get_company(&self.pool, id, user.id)
            .await
            .map_err(status)?;
        Ok((company, user.id))
    }
}

#[tonic::async_trait]
impl CompanyService for CompanyApi {
    async fn lookup_company(
        &self,
        request: Request<pb::LookupCompanyRequest>,
    ) -> Result<Response<pb::LookupCompanyResponse>, Status> {
        signed_in_user(&self.pool, &request).await?;
        Err(Status::failed_precondition("lookup_unavailable"))
    }

    async fn create_company(
        &self,
        request: Request<pb::CreateCompanyRequest>,
    ) -> Result<Response<pb::CreateCompanyResponse>, Status> {
        let user = signed_in_user(&self.pool, &request).await?;
        let req = request.into_inner();
        let address = req.address.clone().unwrap_or_default();
        let input = NewCompany {
            org_nr: &req.org_nr,
            name: &req.name,
            legal_form: legal_form_from(req.legal_form())?,
            street: &address.street,
            postal_code: &address.postal_code,
            city: &address.city,
            fiscal_year_start: date(&req.fiscal_year_start)?,
            fiscal_year_end: date(&req.fiscal_year_end)?,
            accounting_method: accounting_method_from(req.accounting_method())?,
        };
        let id = doris_company::register_company(&self.pool, user.id, input)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::CreateCompanyResponse {
            company_id: id.to_string(),
        }))
    }

    async fn list_companies(
        &self,
        request: Request<pb::ListCompaniesRequest>,
    ) -> Result<Response<pb::ListCompaniesResponse>, Status> {
        let user = signed_in_user(&self.pool, &request).await?;
        let companies = doris_company::list_companies(&self.pool, user.id)
            .await
            .map_err(status)?
            .into_iter()
            .map(|c| pb::CompanySummary {
                id: c.id.to_string(),
                org_nr: format!("{}-{}", &c.org_nr[..6], &c.org_nr[6..]),
                name: c.name,
            })
            .collect();
        Ok(Response::new(pb::ListCompaniesResponse { companies }))
    }

    async fn get_company(
        &self,
        request: Request<pb::GetCompanyRequest>,
    ) -> Result<Response<pb::Company>, Status> {
        let (company, _) = self
            .member_company(&request, &request.get_ref().company_id)
            .await?;
        // ponytail: "today" in UTC, so the fiscal year flips up to 2 hours
        // late at New Year in Sweden; use Europe/Stockholm once the image ships tzdata.
        let today = Timestamp::now().to_zoned(TimeZone::UTC).date();
        let year = company.first_fiscal_year.containing(today);
        Ok(Response::new(pb::Company {
            id: company.id.to_string(),
            org_nr: company.org_nr.formatted(),
            name: company.name.as_str().to_owned(),
            legal_form: legal_form_message(company.legal_form) as i32,
            address: Some(address_message(&company.address)),
            accounting_method: match company.accounting_method {
                AccountingMethod::Cash => pb::AccountingMethod::Cash,
                AccountingMethod::Invoice => pb::AccountingMethod::Invoice,
            } as i32,
            fiscal_year_start: year.start.to_string(),
            fiscal_year_end: year.end.to_string(),
        }))
    }

    async fn add_member(
        &self,
        request: Request<pb::AddMemberRequest>,
    ) -> Result<Response<pb::AddMemberResponse>, Status> {
        // Access first, so a non-member can't probe which emails exist.
        let (company, actor) = self
            .member_company(&request, &request.get_ref().company_id)
            .await?;
        let member = doris_identity::find_user_by_email(&self.pool, &request.get_ref().email)
            .await
            .map_err(grpc::status)?
            .ok_or_else(|| Status::not_found("user_not_found"))?;
        doris_company::add_member(&self.pool, company.id, actor, member.id)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AddMemberResponse {}))
    }

    async fn list_members(
        &self,
        request: Request<pb::ListMembersRequest>,
    ) -> Result<Response<pb::ListMembersResponse>, Status> {
        let (company, _) = self
            .member_company(&request, &request.get_ref().company_id)
            .await?;
        let mut members = Vec::with_capacity(company.members.len());
        for id in company.members {
            if let Some(user) = doris_identity::get_user(&self.pool, id)
                .await
                .map_err(grpc::status)?
            {
                members.push(pb::Member {
                    display_name: user.display_name.as_str().to_owned(),
                    email: user.email.as_str().to_owned(),
                });
            }
        }
        Ok(Response::new(pb::ListMembersResponse { members }))
    }
}

pub(crate) fn legal_form_message(form: LegalForm) -> pb::LegalForm {
    match form {
        LegalForm::Aktiebolag => pb::LegalForm::Aktiebolag,
        LegalForm::Handelsbolag => pb::LegalForm::Handelsbolag,
        LegalForm::Kommanditbolag => pb::LegalForm::Kommanditbolag,
        LegalForm::EnskildFirma => pb::LegalForm::EnskildFirma,
        LegalForm::EkonomiskForening => pb::LegalForm::EkonomiskForening,
        LegalForm::IdeellForening => pb::LegalForm::IdeellForening,
        LegalForm::Stiftelse => pb::LegalForm::Stiftelse,
        LegalForm::Other => pb::LegalForm::Other,
    }
}

fn legal_form_from(form: pb::LegalForm) -> Result<LegalForm, Status> {
    Ok(match form {
        pb::LegalForm::Unspecified => return Err(Status::invalid_argument("invalid_legal_form")),
        pb::LegalForm::Aktiebolag => LegalForm::Aktiebolag,
        pb::LegalForm::Handelsbolag => LegalForm::Handelsbolag,
        pb::LegalForm::Kommanditbolag => LegalForm::Kommanditbolag,
        pb::LegalForm::EnskildFirma => LegalForm::EnskildFirma,
        pb::LegalForm::EkonomiskForening => LegalForm::EkonomiskForening,
        pb::LegalForm::IdeellForening => LegalForm::IdeellForening,
        pb::LegalForm::Stiftelse => LegalForm::Stiftelse,
        pb::LegalForm::Other => LegalForm::Other,
    })
}

fn accounting_method_from(method: pb::AccountingMethod) -> Result<AccountingMethod, Status> {
    match method {
        pb::AccountingMethod::Unspecified => {
            Err(Status::invalid_argument("invalid_accounting_method"))
        }
        pb::AccountingMethod::Cash => Ok(AccountingMethod::Cash),
        pb::AccountingMethod::Invoice => Ok(AccountingMethod::Invoice),
    }
}

pub(crate) fn address_message(address: &Address) -> pb::Address {
    pb::Address {
        street: address.street.clone().unwrap_or_default(),
        postal_code: address.postal_code.clone().unwrap_or_default(),
        city: address.city.clone().unwrap_or_default(),
    }
}

fn date(raw: &str) -> Result<Date, Status> {
    raw.parse()
        .map_err(|_| Status::invalid_argument("invalid_fiscal_year"))
}

fn company_not_found() -> Status {
    Status::not_found("company_not_found")
}

pub(crate) fn domain_status(err: DomainError) -> Status {
    match err {
        DomainError::InvalidOrgNr => Status::invalid_argument("invalid_org_nr"),
        DomainError::InvalidCompanyName => Status::invalid_argument("invalid_company_name"),
        DomainError::InvalidAddress => Status::invalid_argument("invalid_address"),
        DomainError::InvalidFiscalYear => Status::invalid_argument("invalid_fiscal_year"),
        DomainError::NotMember => company_not_found(),
    }
}

fn status(err: Error) -> Status {
    match err {
        Error::Domain(err) => domain_status(err),
        Error::AlreadyExists => Status::already_exists("company_exists"),
        Error::NotFound => company_not_found(),
        Error::Store(err) => {
            tracing::error!("store: {err}");
            Status::internal("internal")
        }
    }
}
