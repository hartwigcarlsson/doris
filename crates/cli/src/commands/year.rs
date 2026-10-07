use super::Context;
use crate::output::{Failure, Output};
use doris_proto::ledger::v1 as lpb;
use serde_json::json;

pub async fn list(context: &Context, output: &mut Output<'_>) -> Result<(), Failure> {
    let company = super::company(context).await?;
    let years = context
        .doris
        .ledger()
        .list_fiscal_years(context.doris.request(lpb::ListFiscalYearsRequest {
            company_id: company.id,
        }))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner()
        .fiscal_years;
    let value: Vec<_> = years
        .iter()
        .map(|y| json!({ "start": y.start, "end": y.end, "closed": y.closed }))
        .collect();
    let text: String = years
        .iter()
        .map(|y| {
            let state = if y.closed { "Stängt" } else { "Öppet" };
            format!("{} – {}  {state}\n", y.start, y.end)
        })
        .collect();
    context.print(output, json!(value), text);
    Ok(())
}
