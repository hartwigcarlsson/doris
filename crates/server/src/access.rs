//! What a call with an API token may do, per gRPC method. Sessions are not
//! limited by this table. A method missing from it is closed to tokens, and
//! a test keeps every rpc in `proto/` in it.

use doris_identity::domain::Scope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// Concerns one company and needs this scope for it.
    Company(Scope),
    /// Any token may call it: it answers about the token's owner.
    Owner,
    /// Sessions only.
    SessionOnly,
}

/// The access a gRPC path (`/package.Service/Method`) needs from a token.
pub(crate) fn access(path: &str) -> Access {
    path.strip_prefix('/')
        .and_then(|p| p.split_once('/'))
        .and_then(|(service, method)| classify(service, method))
        .unwrap_or(Access::SessionOnly)
}

fn classify(service: &str, method: &str) -> Option<Access> {
    use Access::*;
    use Scope::*;
    Some(match (service, method) {
        ("doris.auth.v1.AuthService", "GetStatus") => Owner,
        (
            "doris.auth.v1.AuthService",
            "BeginRegistration"
            | "FinishRegistration"
            | "BeginLogin"
            | "FinishLogin"
            | "Logout"
            | "BeginAddPasskey"
            | "FinishAddPasskey"
            | "ListPasskeys"
            | "GetInvitation"
            | "CreateInvitation"
            | "ListInvitations"
            | "BeginCreateApiToken"
            | "FinishCreateApiToken"
            | "BeginChangeApiToken"
            | "FinishChangeApiToken"
            | "ListApiTokens"
            | "RevokeApiToken",
        ) => SessionOnly,

        ("doris.company.v1.CompanyService", "ListCompanies") => Owner,
        ("doris.company.v1.CompanyService", "GetCompany" | "ListMembers") => Company(CompanyRead),
        (
            "doris.company.v1.CompanyService",
            "GetLookupStatus" | "LookupCompany" | "CreateCompany" | "AddMember",
        ) => SessionOnly,

        (
            "doris.ledger.v1.LedgerService",
            "ListAccounts"
            | "ListFiscalYears"
            | "ListVouchers"
            | "GetAttachment"
            | "GetTrialBalance"
            | "GetAccountLedger"
            | "GetFinancialStatements"
            | "GetOpeningBalances",
        ) => Company(LedgerRead),
        (
            "doris.ledger.v1.LedgerService",
            "AddAccount" | "RenameAccount" | "SetAccountActive" | "SetAccountVatBox"
            | "RecordVoucher" | "CorrectVoucher" | "AddAttachment" | "SetOpeningBalances"
            | "CloseFiscalYear" | "ReopenFiscalYear",
        ) => Company(LedgerWrite),

        (
            "doris.invoicing.v1.InvoicingService",
            "ListCustomers"
            | "ListSuppliers"
            | "ListSupplierInvoices"
            | "GetSupplierInvoiceAttachment"
            | "ListCustomerInvoices"
            | "GetCustomerInvoiceAttachment",
        ) => Company(InvoicingRead),
        (
            "doris.invoicing.v1.InvoicingService",
            "AddCustomer"
            | "UpdateCustomer"
            | "SetCustomerActive"
            | "AddSupplier"
            | "UpdateSupplier"
            | "SetSupplierActive"
            | "RegisterSupplierInvoice"
            | "PaySupplierInvoice"
            | "CancelSupplierInvoice"
            | "ReverseSupplierInvoicePayment"
            | "RegisterCustomerInvoice"
            | "PayCustomerInvoice"
            | "CancelCustomerInvoice"
            | "ReverseCustomerInvoicePayment",
        ) => Company(InvoicingWrite),

        (
            "doris.payroll.v1.PayrollService",
            "ListEmployees" | "PreviewPayrollRun" | "GetPayrollRun" | "ListPayrollRuns"
            | "GetAgiContact" | "ListAgiMonths" | "GetAgiMonth" | "ExportAgiFile",
        ) => Company(PayrollRead),
        (
            "doris.payroll.v1.PayrollService",
            "AddEmployee" | "UpdateEmployee" | "DeactivateEmployee" | "SetEmployeeTax"
            | "CreatePayrollRun" | "UpdatePayrollRun" | "FinalizePayrollRun" | "ReopenPayrollRun"
            | "BookPayrollRun" | "UnbookPayrollRun" | "SetAgiContact" | "MarkAgiSubmitted",
        ) => Company(PayrollWrite),

        ("doris.vat.v1.VatService", "ListVatReturns" | "GetVatReturn" | "ExportVatFile") => {
            Company(VatRead)
        }
        ("doris.vat.v1.VatService", "SetVatPeriod" | "MarkVatReturnSubmitted") => Company(VatWrite),

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROTOS: [&str; 6] = [
        include_str!("../../../proto/doris/auth/v1/auth.proto"),
        include_str!("../../../proto/doris/company/v1/company.proto"),
        include_str!("../../../proto/doris/invoicing/v1/invoicing.proto"),
        include_str!("../../../proto/doris/ledger/v1/ledger.proto"),
        include_str!("../../../proto/doris/payroll/v1/payroll.proto"),
        include_str!("../../../proto/doris/vat/v1/vat.proto"),
    ];

    /// `(package.Service, Method)` for every rpc in a .proto file.
    fn methods(proto: &str) -> Vec<(String, String)> {
        let line = |start: &str| {
            proto
                .lines()
                .map(str::trim)
                .find_map(|l| l.strip_prefix(start))
                .map(|rest| rest.trim_end_matches([';', '{', ' ']).trim().to_owned())
                .unwrap()
        };
        let service = format!("{}.{}", line("package "), line("service "));
        proto
            .lines()
            .filter_map(|l| l.trim().strip_prefix("rpc "))
            .map(|rest| {
                (
                    service.clone(),
                    rest.split('(').next().unwrap().trim().to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn every_rpc_is_in_the_table() {
        let all: Vec<_> = PROTOS.iter().flat_map(|p| methods(p)).collect();
        assert!(all.len() > 80, "the protos were not read: {}", all.len());
        let missing: Vec<_> = all
            .iter()
            .filter(|(service, method)| classify(service, method).is_none())
            .collect();
        assert_eq!(
            missing,
            Vec::<&(String, String)>::new(),
            "add these to `classify`"
        );
    }

    #[test]
    fn the_table_follows_the_spec() {
        use Access::*;
        use Scope::*;
        for (path, expected) in [
            (
                "/doris.ledger.v1.LedgerService/RecordVoucher",
                Company(LedgerWrite),
            ),
            (
                "/doris.ledger.v1.LedgerService/ListVouchers",
                Company(LedgerRead),
            ),
            (
                "/doris.payroll.v1.PayrollService/ExportAgiFile",
                Company(PayrollRead),
            ),
            (
                "/doris.payroll.v1.PayrollService/BookPayrollRun",
                Company(PayrollWrite),
            ),
            (
                "/doris.invoicing.v1.InvoicingService/PaySupplierInvoice",
                Company(InvoicingWrite),
            ),
            (
                "/doris.vat.v1.VatService/MarkVatReturnSubmitted",
                Company(VatWrite),
            ),
            (
                "/doris.company.v1.CompanyService/GetCompany",
                Company(CompanyRead),
            ),
            ("/doris.company.v1.CompanyService/ListCompanies", Owner),
            ("/doris.auth.v1.AuthService/GetStatus", Owner),
            (
                "/doris.auth.v1.AuthService/BeginCreateApiToken",
                SessionOnly,
            ),
            (
                "/doris.auth.v1.AuthService/FinishChangeApiToken",
                SessionOnly,
            ),
            ("/doris.company.v1.CompanyService/AddMember", SessionOnly),
            ("/doris.ledger.v1.LedgerService/SomethingNew", SessionOnly),
            ("/index.html", SessionOnly),
        ] {
            assert_eq!(access(path), expected, "{path}");
        }
    }
}
