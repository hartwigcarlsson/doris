use crate::api::{company_api, cpb};
use crate::errors::describe;
use crate::ui::{Card, ErrorAlert};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

#[component]
pub fn Companies() -> impl IntoView {
    let companies = RwSignal::new(None::<Vec<cpb::CompanySummary>>);
    let error = RwSignal::new(None::<String>);
    spawn_local(async move {
        match company_api()
            .list_companies(cpb::ListCompaniesRequest {})
            .await
        {
            Ok(list) => companies.set(Some(list.into_inner().companies)),
            Err(status) => error.set(Some(describe(&status))),
        }
    });

    view! {
        <Card title="Företag" description="Företagen du sköter bokföringen åt.">
            <div class="grid gap-4">
                <ErrorAlert message=error />
                {move || {
                    companies
                        .get()
                        .map(|list| {
                            if list.is_empty() {
                                view! { <p class="text-muted-foreground">"Inga företag än."</p> }
                                    .into_any()
                            } else {
                                view! {
                                    <ul class="grid gap-2">
                                        {list
                                            .into_iter()
                                            .map(|c| {
                                                view! {
                                                    <li class="flex justify-between gap-2">
                                                        <A
                                                            href=format!("/companies/{}", c.id)
                                                            attr:class="font-medium hover:underline"
                                                        >
                                                            {c.name}
                                                        </A>
                                                        <span class="text-muted-foreground">{c.org_nr}</span>
                                                    </li>
                                                }
                                            })
                                            .collect_view()}
                                    </ul>
                                }
                                    .into_any()
                            }
                        })
                }}
                <A
                    href="/companies/new"
                    attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline"
                >
                    "Lägg till företag"
                </A>
            </div>
        </Card>
    }
}
