use super::Context;
use crate::output::{Failure, Output};
use doris_proto::auth::v1::GetStatusRequest;
use serde_json::json;

pub async fn status(context: &Context, output: &mut Output<'_>) -> Result<(), Failure> {
    let status = context
        .doris
        .auth()
        .get_status(context.doris.request(GetStatusRequest {}))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner();
    let user = status
        .current_user
        .ok_or_else(|| Failure::new("not_signed_in"))?;
    context.print(
        output,
        json!({ "name": user.display_name, "email": user.email }),
        format!("{} <{}>\n", user.display_name, user.email),
    );
    Ok(())
}
