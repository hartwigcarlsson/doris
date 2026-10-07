use super::Context;
use crate::output::{Failure, Output};
use doris_proto::ledger::v1 as lpb;
use serde_json::json;

pub async fn list(context: &Context, output: &mut Output<'_>) -> Result<(), Failure> {
    let company = super::company(context).await?;
    let accounts = context
        .doris
        .ledger()
        .list_accounts(context.doris.request(lpb::ListAccountsRequest {
            company_id: company.id,
        }))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner()
        .accounts;
    let value: Vec<_> = accounts
        .iter()
        .map(|a| json!({ "number": a.number, "name": a.name, "active": a.active }))
        .collect();
    let text: String = accounts
        .iter()
        .map(|a| {
            let inactive = if a.active { "" } else { " (inaktivt)" };
            format!("{}  {}{inactive}\n", a.number, a.name)
        })
        .collect();
    context.print(output, json!(value), text);
    Ok(())
}
