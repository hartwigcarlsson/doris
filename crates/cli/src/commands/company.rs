use super::Context;
use crate::output::{Failure, Output};
use doris_proto::company::v1 as cpb;
use serde_json::json;

pub async fn list(context: &Context, output: &mut Output<'_>) -> Result<(), Failure> {
    let companies = context
        .doris
        .companies()
        .list_companies(context.doris.request(cpb::ListCompaniesRequest {}))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner()
        .companies;
    let value: Vec<_> = companies
        .iter()
        .map(|c| json!({ "id": c.id, "org_nr": c.org_nr, "name": c.name }))
        .collect();
    let text: String = companies
        .iter()
        .map(|c| format!("{}  {}\n", c.org_nr, c.name))
        .collect();
    context.print(output, json!(value), text);
    Ok(())
}

/// `LEGAL_FORM_AKTIEBOLAG` -> `aktiebolag`.
fn short(name: &str, prefix: &str) -> String {
    name.strip_prefix(prefix).unwrap_or(name).to_lowercase()
}

pub async fn view(context: &Context, output: &mut Output<'_>) -> Result<(), Failure> {
    let summary = super::company(context).await?;
    let company = context
        .doris
        .companies()
        .get_company(context.doris.request(cpb::GetCompanyRequest {
            company_id: summary.id,
        }))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner();
    let legal_form = short(company.legal_form().as_str_name(), "LEGAL_FORM_");
    let method = short(
        company.accounting_method().as_str_name(),
        "ACCOUNTING_METHOD_",
    );
    let text = format!(
        "Namn: {}\nOrg.nr: {}\nRäkenskapsår: {} – {}\n",
        company.name, company.org_nr, company.fiscal_year_start, company.fiscal_year_end
    );
    context.print(
        output,
        json!({
            "id": company.id,
            "org_nr": company.org_nr,
            "name": company.name,
            "legal_form": legal_form,
            "accounting_method": method,
            "fiscal_year_start": company.fiscal_year_start,
            "fiscal_year_end": company.fiscal_year_end,
        }),
        text,
    );
    Ok(())
}
