use crate::api::pb;
use crate::app::Session;
use crate::ui::Card;
use leptos::prelude::*;

#[component]
pub fn Home() -> impl IntoView {
    let session = expect_context::<Session>();
    let user = move || session.user.get().unwrap_or_default();
    let role = move || match user().role() {
        pb::Role::Admin => "administratör",
        _ => "användare",
    };
    view! {
        <Card title="Välkommen">
            <p>
                "Inloggad som " <strong>{move || user().display_name}</strong> " (" {move || user().email} "), " {role} "."
            </p>
        </Card>
    }
}
