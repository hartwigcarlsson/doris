//! `doris.invoicing.v1.InvoicingService`: maps gRPC calls onto
//! `doris_invoicing`. Every call needs a session, and a company the caller
//! isn't a member of looks exactly like one that doesn't exist.

use crate::grpc::{signed_in_user, today};
use crate::ledger::{attachment_message, date, new_attachments};
use doris_company::domain::AccountingMethod;
use doris_invoicing::domain::{CustomerForm, DomainError, SupplierForm};
use doris_invoicing::supplier_invoices::{
    NewSupplierInvoice, Status as InvoiceStatus, SupplierInvoice,
};
use doris_invoicing::vat::InvoiceLine;
use doris_invoicing::{Customer, Error, Supplier};
use doris_proto::invoicing::v1 as pb;
use doris_proto::invoicing::v1::invoicing_service_server::InvoicingService;
use sqlx::SqlitePool;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub struct InvoicingApi {
    pool: SqlitePool,
}

impl InvoicingApi {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The signed-in user and the company id they asked about. Membership
    /// is checked by `doris_invoicing` itself.
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
impl InvoicingService for InvoicingApi {
    async fn list_customers(
        &self,
        request: Request<pb::ListCustomersRequest>,
    ) -> Result<Response<pb::ListCustomersResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let customers = doris_invoicing::list_customers(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(customer_pb)
            .collect();
        Ok(Response::new(pb::ListCustomersResponse { customers }))
    }

    async fn add_customer(
        &self,
        request: Request<pb::AddCustomerRequest>,
    ) -> Result<Response<pb::AddCustomerResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let details = request.into_inner().details.unwrap_or_default();
        let number =
            doris_invoicing::add_customer(&self.pool, company, user, &customer_form(&details))
                .await
                .map_err(status)?;
        Ok(Response::new(pb::AddCustomerResponse { number }))
    }

    async fn update_customer(
        &self,
        request: Request<pb::UpdateCustomerRequest>,
    ) -> Result<Response<pb::UpdateCustomerResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let details = req.details.unwrap_or_default();
        doris_invoicing::update_customer(
            &self.pool,
            company,
            user,
            req.number,
            &customer_form(&details),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::UpdateCustomerResponse {}))
    }

    async fn set_customer_active(
        &self,
        request: Request<pb::SetCustomerActiveRequest>,
    ) -> Result<Response<pb::SetCustomerActiveResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::set_customer_active(&self.pool, company, user, req.number, req.active)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::SetCustomerActiveResponse {}))
    }

    async fn list_suppliers(
        &self,
        request: Request<pb::ListSuppliersRequest>,
    ) -> Result<Response<pb::ListSuppliersResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let suppliers = doris_invoicing::list_suppliers(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(supplier_pb)
            .collect();
        Ok(Response::new(pb::ListSuppliersResponse { suppliers }))
    }

    async fn add_supplier(
        &self,
        request: Request<pb::AddSupplierRequest>,
    ) -> Result<Response<pb::AddSupplierResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let details = request.into_inner().details.unwrap_or_default();
        let number =
            doris_invoicing::add_supplier(&self.pool, company, user, &supplier_form(&details))
                .await
                .map_err(status)?;
        Ok(Response::new(pb::AddSupplierResponse { number }))
    }

    async fn update_supplier(
        &self,
        request: Request<pb::UpdateSupplierRequest>,
    ) -> Result<Response<pb::UpdateSupplierResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let details = req.details.unwrap_or_default();
        doris_invoicing::update_supplier(
            &self.pool,
            company,
            user,
            req.number,
            &supplier_form(&details),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::UpdateSupplierResponse {}))
    }

    async fn set_supplier_active(
        &self,
        request: Request<pb::SetSupplierActiveRequest>,
    ) -> Result<Response<pb::SetSupplierActiveResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::set_supplier_active(&self.pool, company, user, req.number, req.active)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::SetSupplierActiveResponse {}))
    }

    async fn list_supplier_invoices(
        &self,
        request: Request<pb::ListSupplierInvoicesRequest>,
    ) -> Result<Response<pb::ListSupplierInvoicesResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let invoices = doris_invoicing::list_supplier_invoices(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(supplier_invoice_pb)
            .collect();
        let method = doris_company::get_company(&self.pool, company, user)
            .await
            .map_err(|err| status(err.into()))?
            .accounting_method;
        Ok(Response::new(pb::ListSupplierInvoicesResponse {
            invoices,
            cash_method: method == AccountingMethod::Cash,
        }))
    }

    async fn register_supplier_invoice(
        &self,
        request: Request<pb::RegisterSupplierInvoiceRequest>,
    ) -> Result<Response<pb::RegisterSupplierInvoiceResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let lines = req
            .lines
            .iter()
            .map(|l| InvoiceLine::new(l.account, l.net, l.vat_rate))
            .collect::<Result<Vec<_>, _>>()
            .map_err(domain_status)?;
        let new = NewSupplierInvoice {
            invoice_number: &req.invoice_number,
            invoice_date: date(&req.invoice_date)?,
            due_date: date(&req.due_date)?,
            reference: &req.reference,
            lines,
            vat: req.vat,
        };
        let attachments = new_attachments(req.attachments.clone())?;
        let number = doris_invoicing::register_supplier_invoice(
            &self.pool,
            company,
            user,
            req.supplier_number,
            new,
            attachments,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::RegisterSupplierInvoiceResponse {
            number,
        }))
    }

    async fn pay_supplier_invoice(
        &self,
        request: Request<pb::PaySupplierInvoiceRequest>,
    ) -> Result<Response<pb::PaySupplierInvoiceResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::pay_supplier_invoice(
            &self.pool,
            company,
            user,
            req.number,
            date(&req.date)?,
            req.account,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::PaySupplierInvoiceResponse {}))
    }

    async fn cancel_supplier_invoice(
        &self,
        request: Request<pb::CancelSupplierInvoiceRequest>,
    ) -> Result<Response<pb::CancelSupplierInvoiceResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::cancel_supplier_invoice(
            &self.pool,
            company,
            user,
            req.number,
            &req.reason,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::CancelSupplierInvoiceResponse {}))
    }

    async fn reverse_supplier_invoice_payment(
        &self,
        request: Request<pb::ReverseSupplierInvoicePaymentRequest>,
    ) -> Result<Response<pb::ReverseSupplierInvoicePaymentResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::reverse_supplier_invoice_payment(
            &self.pool,
            company,
            user,
            req.number,
            &req.reason,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::ReverseSupplierInvoicePaymentResponse {}))
    }

    async fn get_supplier_invoice_attachment(
        &self,
        request: Request<pb::GetSupplierInvoiceAttachmentRequest>,
    ) -> Result<Response<pb::GetSupplierInvoiceAttachmentResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let (attachment, data) = doris_invoicing::supplier_invoice_attachment(
            &self.pool,
            company,
            user,
            req.number,
            &req.sha256,
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::GetSupplierInvoiceAttachmentResponse {
            attachment: Some(attachment_message(&attachment)),
            data,
        }))
    }
}

fn customer_form(d: &pb::CustomerDetails) -> CustomerForm<'_> {
    CustomerForm {
        name: &d.name,
        org_nr: &d.org_nr,
        vat_number: &d.vat_number,
        street: &d.street,
        postal_code: &d.postal_code,
        city: &d.city,
        email: &d.email,
        payment_terms: d.payment_terms,
    }
}

fn supplier_form(d: &pb::SupplierDetails) -> SupplierForm<'_> {
    SupplierForm {
        name: &d.name,
        org_nr: &d.org_nr,
        vat_number: &d.vat_number,
        street: &d.street,
        postal_code: &d.postal_code,
        city: &d.city,
        email: &d.email,
        bankgiro: &d.bankgiro,
        plusgiro: &d.plusgiro,
        iban: &d.iban,
        bic: &d.bic,
    }
}

fn customer_pb(c: Customer) -> pb::Customer {
    let d = c.details;
    pb::Customer {
        number: c.number,
        active: c.active,
        details: Some(pb::CustomerDetails {
            name: d.name.as_str().to_owned(),
            org_nr: d.org_nr.map(|o| o.formatted()).unwrap_or_default(),
            vat_number: d
                .vat_number
                .map(|v| v.as_str().to_owned())
                .unwrap_or_default(),
            street: d.address.street.unwrap_or_default(),
            postal_code: d.address.postal_code.unwrap_or_default(),
            city: d.address.city.unwrap_or_default(),
            email: d.email.map(|e| e.as_str().to_owned()).unwrap_or_default(),
            payment_terms: d.payment_terms.get(),
        }),
    }
}

fn supplier_pb(s: Supplier) -> pb::Supplier {
    let d = s.details;
    pb::Supplier {
        number: s.number,
        active: s.active,
        details: Some(pb::SupplierDetails {
            name: d.name.as_str().to_owned(),
            org_nr: d.org_nr.map(|o| o.formatted()).unwrap_or_default(),
            vat_number: d
                .vat_number
                .map(|v| v.as_str().to_owned())
                .unwrap_or_default(),
            street: d.address.street.unwrap_or_default(),
            postal_code: d.address.postal_code.unwrap_or_default(),
            city: d.address.city.unwrap_or_default(),
            email: d.email.map(|e| e.as_str().to_owned()).unwrap_or_default(),
            bankgiro: d.bankgiro.map(|b| b.formatted()).unwrap_or_default(),
            plusgiro: d.plusgiro.map(|p| p.formatted()).unwrap_or_default(),
            iban: d.iban.map(|i| i.formatted()).unwrap_or_default(),
            bic: d.bic.map(|b| b.as_str().to_owned()).unwrap_or_default(),
        }),
    }
}

fn company_not_found() -> Status {
    Status::not_found("company_not_found")
}

fn domain_status(err: DomainError) -> Status {
    use DomainError::*;
    match err {
        InvalidName => Status::invalid_argument("invalid_name"),
        InvalidOrgNr => Status::invalid_argument("invalid_org_nr"),
        InvalidAddress => Status::invalid_argument("invalid_address"),
        InvalidEmail => Status::invalid_argument("invalid_email"),
        InvalidVatNumber => Status::invalid_argument("invalid_vat_number"),
        InvalidPaymentTerms => Status::invalid_argument("invalid_payment_terms"),
        InvalidBankgiro => Status::invalid_argument("invalid_bankgiro"),
        InvalidPlusgiro => Status::invalid_argument("invalid_plusgiro"),
        InvalidIban => Status::invalid_argument("invalid_iban"),
        InvalidBic => Status::invalid_argument("invalid_bic"),
        CustomerNotFound => Status::not_found("customer_not_found"),
        SupplierNotFound => Status::not_found("supplier_not_found"),
        InvalidInvoiceNumber => Status::invalid_argument("invalid_invoice_number"),
        DuplicateSupplierInvoice => Status::already_exists("duplicate_supplier_invoice"),
        InvalidDueDate => Status::invalid_argument("invalid_due_date"),
        InvalidReference => Status::invalid_argument("invalid_reference"),
        InvalidInvoiceLines => Status::invalid_argument("invalid_invoice_lines"),
        InvalidVatRate => Status::invalid_argument("invalid_vat_rate"),
        InvalidVatAmount => Status::invalid_argument("invalid_vat_amount"),
        InvalidInvoiceAccount => Status::invalid_argument("invalid_invoice_account"),
        InvalidPaymentAccount => Status::invalid_argument("invalid_payment_account"),
        SupplierInactive => Status::failed_precondition("supplier_inactive"),
        SupplierInvoiceNotFound => Status::not_found("supplier_invoice_not_found"),
        SupplierInvoicePaid => Status::failed_precondition("supplier_invoice_paid"),
        SupplierInvoiceNotPaid => Status::failed_precondition("supplier_invoice_not_paid"),
        SupplierInvoiceCancelled => Status::failed_precondition("supplier_invoice_cancelled"),
        InvalidReason => Status::invalid_argument("invalid_reason"),
        InvoiceDateInFuture => Status::invalid_argument("voucher_date_in_future"),
        CustomerInactive => Status::failed_precondition("customer_inactive"),
        CustomerInvoiceNotFound => Status::not_found("customer_invoice_not_found"),
        DuplicateCustomerInvoice => Status::already_exists("duplicate_customer_invoice"),
        CustomerInvoicePaid => Status::failed_precondition("customer_invoice_paid"),
        CustomerInvoiceNotPaid => Status::failed_precondition("customer_invoice_not_paid"),
        CustomerInvoiceCancelled => Status::failed_precondition("customer_invoice_cancelled"),
    }
}

fn status(err: Error) -> Status {
    match err {
        Error::Domain(err) => domain_status(err),
        Error::NotFound => company_not_found(),
        Error::Ledger(err) => crate::ledger::status(err),
        Error::Store(err) => {
            tracing::error!("store: {err}");
            Status::internal("internal")
        }
    }
}

fn supplier_invoice_pb(i: SupplierInvoice) -> pb::SupplierInvoice {
    let status_code = i.status_code().to_owned();
    let paid_date = match &i.status {
        InvoiceStatus::Paid { date, .. } => date.to_string(),
        _ => String::new(),
    };
    let r = i.invoice;
    pb::SupplierInvoice {
        number: i.number,
        supplier_number: r.supplier.number,
        supplier_name: r.supplier.name.as_str().to_owned(),
        invoice_number: r.invoice_number.as_str().to_owned(),
        invoice_date: r.invoice_date.to_string(),
        due_date: r.due_date.to_string(),
        reference: r
            .reference
            .map(|x| x.as_str().to_owned())
            .unwrap_or_default(),
        lines: r
            .lines
            .iter()
            .map(|l| pb::InvoiceLine {
                account: l.account.get().into(),
                net: l.net,
                vat_rate: l.vat_rate.percent(),
            })
            .collect(),
        vat: r.vat,
        total: r.total,
        status: status_code,
        paid_date,
        vouchers: i
            .vouchers
            .iter()
            .map(|v| pb::VoucherRef {
                fiscal_year_start: v.fiscal_year_start.to_string(),
                number: v.number,
            })
            .collect(),
        attachments: i.attachments.iter().map(attachment_message).collect(),
        bankgiro: r
            .supplier
            .bankgiro
            .map(|b| b.formatted())
            .unwrap_or_default(),
        plusgiro: r
            .supplier
            .plusgiro
            .map(|p| p.formatted())
            .unwrap_or_default(),
        iban: r.supplier.iban.map(|x| x.formatted()).unwrap_or_default(),
    }
}
