use crate::api::{company_api, cpb};
use crate::errors::describe;
use crate::format::{accounting_method_label, legal_form_label};
use crate::task::spawn_local;
use crate::ui::{Button, Card, ErrorAlert, Field, PageHeader, Panel};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[component]
pub fn CompanyPage() -> impl IntoView {
    let params = use_params_map();
    let id = move || params.read().get("id").unwrap_or_default();
    let company = RwSignal::new(None::<cpb::Company>);
    let members = RwSignal::new(Vec::<cpb::Member>::new());
    let email = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let member_error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let load = move || {
        let company_id = id();
        spawn_local(async move {
            let mut api = company_api();
            match api
                .get_company(cpb::GetCompanyRequest {
                    company_id: company_id.clone(),
                })
                .await
            {
                Ok(c) => company.set(Some(c.into_inner())),
                Err(status) => return error.set(Some(describe(&status))),
            }
            match api
                .list_members(cpb::ListMembersRequest { company_id })
                .await
            {
                Ok(list) => members.set(list.into_inner().members),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    load();

    let add = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        member_error.set(None);
        spawn_local(async move {
            let request = cpb::AddMemberRequest {
                company_id: id(),
                email: email.get_untracked(),
            };
            match company_api().add_member(request).await {
                Ok(_) => {
                    email.set(String::new());
                    load();
                }
                Err(status) => member_error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <ErrorAlert message=error />
            {move || {
                company
                    .get()
                    .map(|c| {
                        let address = c.address.clone().unwrap_or_default();
                        let postal = format!("{} {}", address.postal_code, address.city)
                            .trim()
                            .to_owned();
                        view! {
                            <PageHeader title=c.name.clone() />
                            <Panel class="grid w-full max-w-xl gap-2">
                                <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
                                    <dt class="text-muted-foreground">"Organisationsnummer"</dt>
                                    <dd>{c.org_nr.clone()}</dd>
                                    <dt class="text-muted-foreground">"Juridisk form"</dt>
                                    <dd>{legal_form_label(c.legal_form())}</dd>
                                    <dt class="text-muted-foreground">"Adress"</dt>
                                    <dd>{address.street.clone()} " " {postal}</dd>
                                    <dt class="text-muted-foreground">"Räkenskapsår"</dt>
                                    <dd>{format!("{} – {}", c.fiscal_year_start, c.fiscal_year_end)}</dd>
                                    <dt class="text-muted-foreground">"Bokföringsmetod"</dt>
                                    <dd>{accounting_method_label(c.accounting_method())}</dd>
                                </dl>
                            </Panel>
                        }
                    })
            }}
            <Show when=move || company.get().is_some()>
                <Card title="Medlemmar" description="De som har tillgång till företaget." narrow=true>
                    <ul class="mb-4 grid gap-2">
                        <For each=move || members.get() key=|m| m.email.clone() let(member)>
                            <li class="flex justify-between gap-2">
                                <span>{member.display_name}</span>
                                <span class="text-muted-foreground">{member.email}</span>
                            </li>
                        </For>
                    </ul>
                    <form class="grid gap-4" novalidate on:submit=add>
                        <ErrorAlert message=member_error />
                        <Field label="E-post" id="member_email" kind="email" value=email />
                        <Button disabled=busy>"Lägg till medlem"</Button>
                    </form>
                </Card>
            </Show>
        </div>
    }
}
