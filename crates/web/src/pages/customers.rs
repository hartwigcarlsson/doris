//! The active company's customers: add, edit, (de)activate.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb};
use crate::errors::describe;
use crate::ui::{
    Badge, BadgeVariant, Button, Card, ErrorAlert, Field, Icon, IconName, PageHeader, TABLE_BODY,
    TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table, TableCard, Variant,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

/// The form's fields, one signal each.
#[derive(Clone, Copy)]
struct Form {
    name: RwSignal<String>,
    org_nr: RwSignal<String>,
    vat_number: RwSignal<String>,
    street: RwSignal<String>,
    postal_code: RwSignal<String>,
    city: RwSignal<String>,
    email: RwSignal<String>,
    payment_terms: RwSignal<String>,
}

impl Form {
    fn new() -> Self {
        let text = || RwSignal::new(String::new());
        let form = Self {
            name: text(),
            org_nr: text(),
            vat_number: text(),
            street: text(),
            postal_code: text(),
            city: text(),
            email: text(),
            payment_terms: text(),
        };
        form.clear();
        form
    }

    fn fill(&self, d: &ipb::CustomerDetails) {
        self.name.set(d.name.clone());
        self.org_nr.set(d.org_nr.clone());
        self.vat_number.set(d.vat_number.clone());
        self.street.set(d.street.clone());
        self.postal_code.set(d.postal_code.clone());
        self.city.set(d.city.clone());
        self.email.set(d.email.clone());
        self.payment_terms.set(d.payment_terms.to_string());
    }

    /// Empty, with the usual 30 days.
    fn clear(&self) {
        self.fill(&ipb::CustomerDetails {
            payment_terms: 30,
            ..Default::default()
        });
    }

    fn details(&self) -> ipb::CustomerDetails {
        ipb::CustomerDetails {
            name: self.name.get_untracked(),
            org_nr: self.org_nr.get_untracked(),
            vat_number: self.vat_number.get_untracked(),
            street: self.street.get_untracked(),
            postal_code: self.postal_code.get_untracked(),
            city: self.city.get_untracked(),
            email: self.email.get_untracked(),
            // Not a number: out of range, so the server refuses it with its own message.
            payment_terms: self
                .payment_terms
                .get_untracked()
                .trim()
                .parse()
                .unwrap_or(u32::MAX),
        }
    }
}

#[component]
pub fn Customers() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // The customers and the company they were loaded for, set together.
    let customers = RwSignal::new((String::new(), Vec::<ipb::Customer>::new()));
    let form = Form::new();
    // None: closed. Some(None): a new customer. Some(Some(n)): editing customer n.
    let open = RwSignal::new(None::<Option<u32>>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = invoicing_api()
                .list_customers(ipb::ListCustomersRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => customers.set((company_id, response.into_inner().customers)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        // Never leave the previous company's rows (or form) on screen.
        customers.set((String::new(), Vec::new()));
        error.set(None);
        open.set(None);
        form.clear();
        load();
    });
    let changed = Callback::new(move |()| load());
    let edit = Callback::new(move |customer: ipb::Customer| {
        error.set(None);
        form.fill(&customer.details.unwrap_or_default());
        open.set(Some(Some(customer.number)));
    });

    let save = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        // The company whose customers are on screen, not whatever is active now.
        let company_id = customers.with_untracked(|(id, _)| id.clone());
        let details = Some(form.details());
        let editing = open.get_untracked().flatten();
        spawn_local(async move {
            let result = match editing {
                Some(number) => invoicing_api()
                    .update_customer(ipb::UpdateCustomerRequest {
                        company_id,
                        number,
                        details,
                    })
                    .await
                    .map(|_| ()),
                None => invoicing_api()
                    .add_customer(ipb::AddCustomerRequest {
                        company_id,
                        details,
                    })
                    .await
                    .map(|_| ()),
            };
            match result {
                Ok(()) => {
                    open.set(None);
                    form.clear();
                    load();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <PageHeader title="Kunder">
                <Button
                    kind="button"
                    on:click=move |_| {
                        error.set(None);
                        form.clear();
                        open.set(Some(None));
                    }
                >
                    <Icon name=IconName::Plus />
                    "Ny kund"
                </Button>
            </PageHeader>
            <ErrorAlert message=error />
            <Show when=move || open.get().is_some()>
                <Card title="Kunduppgifter">
                    <form class="grid grid-cols-2 gap-4" novalidate on:submit=save>
                        <Field label="Namn" id="customer_name" value=form.name />
                        <Field label="Org.nr/personnr" id="customer_org_nr" value=form.org_nr />
                        <Field label="Momsreg.nr" id="customer_vat_number" value=form.vat_number />
                        <Field label="E-post" id="customer_email" value=form.email />
                        <Field label="Gatuadress" id="customer_street" value=form.street />
                        <Field label="Postnummer" id="customer_postal_code" value=form.postal_code />
                        <Field label="Ort" id="customer_city" value=form.city />
                        <Field
                            label="Betalningsvillkor (dagar)"
                            id="customer_payment_terms"
                            value=form.payment_terms
                        />
                        <div class="col-span-2 flex gap-2">
                            <Button disabled=busy>"Spara"</Button>
                            <Button variant=Variant::Ghost kind="button" on:click=move |_| open.set(None)>
                                "Avbryt"
                            </Button>
                        </div>
                    </form>
                </Card>
            </Show>
            <TableCard><Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Nr"</th>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=TABLE_HEADER_CELL>"Org.nr"</th>
                        <th class=TABLE_HEADER_CELL>"Ort"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let (company_id, list) = customers.get();
                            list.into_iter().map(|c| (company_id.clone(), c)).collect::<Vec<_>>()
                        }
                        key=|(company_id, c)| (company_id.clone(), c.number, c.active, format!("{:?}", c.details))
                        let((company_id, customer))
                    >
                        <CustomerRow company_id=company_id customer=customer edit=edit changed=changed error=error />
                    </For>
                </tbody>
            </Table></TableCard>
        </div>
    }
}

#[component]
fn CustomerRow(
    company_id: String,
    customer: ipb::Customer,
    edit: Callback<ipb::Customer>,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    // The company this row was loaded for, not whatever is active now.
    let company_id = StoredValue::new(company_id);
    let (number, active) = (customer.number, customer.active);
    let details = customer.details.clone().unwrap_or_default();
    let customer = StoredValue::new(customer);

    let toggle = move |_| {
        error.set(None);
        spawn_local(async move {
            let request = ipb::SetCustomerActiveRequest {
                company_id: company_id.get_value(),
                number,
                active: !active,
            };
            match invoicing_api().set_customer_active(request).await {
                Ok(_) => changed.run(()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>{number}</td>
            <td class=TABLE_CELL>{details.name}</td>
            <td class=TABLE_CELL>{details.org_nr}</td>
            <td class=TABLE_CELL>{details.city}</td>
            <td class=TABLE_CELL>
                {if active {
                    view! { <Badge>"Aktiv"</Badge> }.into_any()
                } else {
                    view! { <Badge variant=BadgeVariant::Outline>"Inaktiv"</Badge> }.into_any()
                }}
            </td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Button variant=Variant::Ghost kind="button" on:click=move |_| edit.run(customer.get_value())>
                    "Redigera"
                </Button>
                <Button variant=Variant::Ghost kind="button" on:click=toggle>
                    {if active { "Inaktivera" } else { "Aktivera" }}
                </Button>
            </td>
        </tr>
    }
}
