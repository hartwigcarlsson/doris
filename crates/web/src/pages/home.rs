use crate::active_company::Companies;
use crate::api::pb;
use crate::app::Session;
use crate::ui::{Card, PageHeader};
use leptos::prelude::*;
use leptos_router::components::A;

#[component]
pub fn Home() -> impl IntoView {
    let session = expect_context::<Session>();
    let companies = expect_context::<Companies>();
    let user = move || session.user.get().unwrap_or_default();
    let role = move || match user().role() {
        pb::Role::Admin => "administratör",
        _ => "användare",
    };
    view! {
        <div class="grid gap-6">
            <PageHeader title="Översikt" />
            <Card title="Välkommen" narrow=true>
                <p>
                    "Inloggad som " <strong>{move || user().display_name}</strong> " (" {move || user().email} "), " {role} "."
                </p>
            </Card>
            <Card title="Aktivt företag" narrow=true>
                {move || match companies.active_company() {
                    Some(c) => {
                        view! {
                            <div class="grid gap-2">
                                <p>
                                    <strong>{c.name.clone()}</strong>
                                    " "
                                    <span class="text-muted-foreground">{c.org_nr.clone()}</span>
                                </p>
                                <A href=format!("/companies/{}", c.id) attr:class="font-medium underline-offset-4 hover:underline">
                                    "Visa företaget"
                                </A>
                            </div>
                        }
                            .into_any()
                    }
                    None if companies.loaded.get() => {
                        view! {
                            <div class="grid gap-2">
                                <p class="text-muted-foreground">"Du har inga företag än."</p>
                                <A href="/companies/new" attr:class="font-medium underline-offset-4 hover:underline">
                                    "Lägg till företag"
                                </A>
                            </div>
                        }
                            .into_any()
                    }
                    None => ().into_any(),
                }}
            </Card>
        </div>
    }
}
