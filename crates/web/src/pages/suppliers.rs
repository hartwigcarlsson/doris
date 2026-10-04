//! The active company's suppliers: add, edit, (de)activate.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb};
use crate::errors::describe;
use crate::ui::{
    Button, Card, ErrorAlert, Field, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table, Variant,
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
    bankgiro: RwSignal<String>,
    plusgiro: RwSignal<String>,
    iban: RwSignal<String>,
    bic: RwSignal<String>,
}

impl Form {
    fn new() -> Self {
        let text = || RwSignal::new(String::new());
        Self {
            name: text(),
            org_nr: text(),
            vat_number: text(),
            street: text(),
            postal_code: text(),
            city: text(),
            email: text(),
            bankgiro: text(),
            plusgiro: text(),
            iban: text(),
            bic: text(),
        }
    }

    fn fill(&self, d: &ipb::SupplierDetails) {
        self.name.set(d.name.clone());
        self.org_nr.set(d.org_nr.clone());
        self.vat_number.set(d.vat_number.clone());
        self.street.set(d.street.clone());
        self.postal_code.set(d.postal_code.clone());
        self.city.set(d.city.clone());
        self.email.set(d.email.clone());
        self.bankgiro.set(d.bankgiro.clone());
        self.plusgiro.set(d.plusgiro.clone());
        self.iban.set(d.iban.clone());
        self.bic.set(d.bic.clone());
    }

    fn clear(&self) {
        self.fill(&ipb::SupplierDetails::default());
    }

    fn details(&self) -> ipb::SupplierDetails {
        ipb::SupplierDetails {
            name: self.name.get_untracked(),
            org_nr: self.org_nr.get_untracked(),
            vat_number: self.vat_number.get_untracked(),
            street: self.street.get_untracked(),
            postal_code: self.postal_code.get_untracked(),
            city: self.city.get_untracked(),
            email: self.email.get_untracked(),
            bankgiro: self.bankgiro.get_untracked(),
            plusgiro: self.plusgiro.get_untracked(),
            iban: self.iban.get_untracked(),
            bic: self.bic.get_untracked(),
        }
    }
}

#[component]
pub fn Suppliers() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // The suppliers and the company they were loaded for, set together.
    let suppliers = RwSignal::new((String::new(), Vec::<ipb::Supplier>::new()));
    let form = Form::new();
    // None: closed. Some(None): a new supplier. Some(Some(n)): editing supplier n.
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
                .list_suppliers(ipb::ListSuppliersRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => suppliers.set((company_id, response.into_inner().suppliers)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        // Never leave the previous company's rows (or form) on screen.
        suppliers.set((String::new(), Vec::new()));
        error.set(None);
        open.set(None);
        form.clear();
        load();
    });
    let changed = Callback::new(move |()| load());
    let edit = Callback::new(move |supplier: ipb::Supplier| {
        error.set(None);
        form.fill(&supplier.details.unwrap_or_default());
        open.set(Some(Some(supplier.number)));
    });

    let save = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        // The company whose suppliers are on screen, not whatever is active now.
        let company_id = suppliers.with_untracked(|(id, _)| id.clone());
        let details = Some(form.details());
        let editing = open.get_untracked().flatten();
        spawn_local(async move {
            let result = match editing {
                Some(number) => invoicing_api()
                    .update_supplier(ipb::UpdateSupplierRequest {
                        company_id,
                        number,
                        details,
                    })
                    .await
                    .map(|_| ()),
                None => invoicing_api()
                    .add_supplier(ipb::AddSupplierRequest {
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
        <div class="grid gap-6" data-wide>
            <div class="flex items-center justify-between">
                <h1 class="text-sm font-medium">"Leverantörer"</h1>
                <Button
                    kind="button"
                    on:click=move |_| {
                        error.set(None);
                        form.clear();
                        open.set(Some(None));
                    }
                >
                    "Ny leverantör"
                </Button>
            </div>
            <ErrorAlert message=error />
            <Show when=move || open.get().is_some()>
                <Card title="Leverantörsuppgifter">
                    <form class="grid grid-cols-2 gap-4" novalidate on:submit=save>
                        <Field label="Namn" id="supplier_name" value=form.name />
                        <Field label="Org.nr" id="supplier_org_nr" value=form.org_nr />
                        <Field label="Momsreg.nr" id="supplier_vat_number" value=form.vat_number />
                        <Field label="E-post" id="supplier_email" value=form.email />
                        <Field label="Gatuadress" id="supplier_street" value=form.street />
                        <Field label="Postnummer" id="supplier_postal_code" value=form.postal_code />
                        <Field label="Ort" id="supplier_city" value=form.city />
                        <Field label="Bankgiro" id="supplier_bankgiro" value=form.bankgiro />
                        <Field label="Plusgiro" id="supplier_plusgiro" value=form.plusgiro />
                        <Field label="IBAN" id="supplier_iban" value=form.iban />
                        <Field label="BIC" id="supplier_bic" value=form.bic />
                        <div class="col-span-2 flex gap-2">
                            <Button disabled=busy>"Spara"</Button>
                            <Button variant=Variant::Ghost kind="button" on:click=move |_| open.set(None)>
                                "Avbryt"
                            </Button>
                        </div>
                    </form>
                </Card>
            </Show>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Nr"</th>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=TABLE_HEADER_CELL>"Org.nr"</th>
                        <th class=TABLE_HEADER_CELL>"Bankgiro"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let (company_id, list) = suppliers.get();
                            list.into_iter().map(|s| (company_id.clone(), s)).collect::<Vec<_>>()
                        }
                        key=|(company_id, s)| (company_id.clone(), s.number, s.active, format!("{:?}", s.details))
                        let((company_id, supplier))
                    >
                        <SupplierRow company_id=company_id supplier=supplier edit=edit changed=changed error=error />
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

#[component]
fn SupplierRow(
    company_id: String,
    supplier: ipb::Supplier,
    edit: Callback<ipb::Supplier>,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    // The company this row was loaded for, not whatever is active now.
    let company_id = StoredValue::new(company_id);
    let (number, active) = (supplier.number, supplier.active);
    let details = supplier.details.clone().unwrap_or_default();
    let supplier = StoredValue::new(supplier);

    let toggle = move |_| {
        error.set(None);
        spawn_local(async move {
            let request = ipb::SetSupplierActiveRequest {
                company_id: company_id.get_value(),
                number,
                active: !active,
            };
            match invoicing_api().set_supplier_active(request).await {
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
            <td class=TABLE_CELL>{details.bankgiro}</td>
            <td class=TABLE_CELL>{if active { "Aktiv" } else { "Inaktiv" }}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Button variant=Variant::Ghost kind="button" on:click=move |_| edit.run(supplier.get_value())>
                    "Redigera"
                </Button>
                <Button variant=Variant::Ghost kind="button" on:click=toggle>
                    {if active { "Inaktivera" } else { "Aktivera" }}
                </Button>
            </td>
        </tr>
    }
}
