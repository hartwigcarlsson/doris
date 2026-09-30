use crate::api::{company_api, cpb};
use crate::errors::describe;
use crate::fiscal_year::default_end;
use crate::format::{LEGAL_FORMS, current_year, legal_form_label};
use crate::ui::{Button, Card, Checkbox, ErrorAlert, Field, Radio, SELECT_OPTION, Select, Variant};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_navigate;

#[component]
pub fn NewCompany() -> impl IntoView {
    let year = current_year();
    let org_nr = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let legal_form = RwSignal::new(String::new()); // the enum's i32, as text
    let street = RwSignal::new(String::new());
    let postal_code = RwSignal::new(String::new());
    let city = RwSignal::new(String::new());
    let start = RwSignal::new(format!("{year}-01-01"));
    // Only used when the first year is shortened or extended; otherwise the
    // end follows from the start and the legal form.
    let custom_end = RwSignal::new(false);
    let end = RwSignal::new(String::new());
    let derived_end = Memo::new(move |_| {
        let form = legal_form
            .get()
            .parse::<i32>()
            .ok()
            .and_then(|f| cpb::LegalForm::try_from(f).ok());
        default_end(&start.get(), form.unwrap_or(cpb::LegalForm::Unspecified))
    });
    Effect::new(move |_| {
        if custom_end.get() {
            end.set(derived_end.get_untracked().unwrap_or_default());
        }
    });
    // `None` until the server has answered.
    let lookup_available = RwSignal::new(None::<bool>);
    spawn_local(async move {
        if let Ok(status) = company_api()
            .get_lookup_status(cpb::GetLookupStatusRequest {})
            .await
        {
            lookup_available.set(Some(status.into_inner().available));
        }
    });
    let method = RwSignal::new(cpb::AccountingMethod::Unspecified);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let navigate = use_navigate();

    let fetch = move |_| {
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let request = cpb::LookupCompanyRequest {
                org_nr: org_nr.get_untracked(),
            };
            match company_api().lookup_company(request).await {
                Ok(found) => {
                    let found = found.into_inner();
                    let address = found.address.clone().unwrap_or_default();
                    org_nr.set(found.org_nr.clone());
                    name.set(found.name.clone());
                    legal_form.set(found.legal_form.to_string());
                    street.set(address.street);
                    postal_code.set(address.postal_code);
                    city.set(address.city);
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        let navigate = navigate.clone();
        spawn_local(async move {
            let request = cpb::CreateCompanyRequest {
                org_nr: org_nr.get_untracked(),
                name: name.get_untracked(),
                legal_form: legal_form.get_untracked().parse().unwrap_or(0),
                address: Some(cpb::Address {
                    street: street.get_untracked(),
                    postal_code: postal_code.get_untracked(),
                    city: city.get_untracked(),
                }),
                fiscal_year_start: start.get_untracked(),
                fiscal_year_end: if custom_end.get_untracked() {
                    end.get_untracked()
                } else {
                    derived_end.get_untracked().unwrap_or_default()
                },
                accounting_method: method.get_untracked() as i32,
            };
            match company_api().create_company(request).await {
                Ok(created) => navigate(
                    &format!("/companies/{}", created.into_inner().company_id),
                    Default::default(),
                ),
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <Card
            title="Lägg till företag"
            description="Uppgifter om företaget du ska sköta bokföringen åt."
        >
            <form class="grid gap-4" novalidate on:submit=submit>
                <Field
                    label="Organisationsnummer"
                    id="org_nr"
                    value=org_nr
                    placeholder="556016-0680"
                />
                {move || match lookup_available.get() {
                    Some(true) => {
                        view! {
                            <Button variant=Variant::Ghost kind="button" disabled=busy on:click=fetch>
                                "Hämta från Bolagsverket"
                            </Button>
                        }
                            .into_any()
                    }
                    Some(false) => {
                        view! {
                            <p class="text-xs/relaxed text-muted-foreground">
                                "Hämtning från Bolagsverket är inte konfigurerad. Fyll i uppgifterna själv."
                            </p>
                        }
                            .into_any()
                    }
                    None => ().into_any(),
                }}
                <Field label="Företagsnamn" id="name" value=name autocomplete="organization" />
                <Select label="Juridisk form" id="legal_form" value=legal_form>
                    <option class=SELECT_OPTION value="">{legal_form_label(cpb::LegalForm::Unspecified)}</option>
                    {LEGAL_FORMS
                        .map(|f| {
                            view! {
                                <option class=SELECT_OPTION value=(f as i32).to_string()>{legal_form_label(f)}</option>
                            }
                        })
                        .collect_view()}
                </Select>
                <Field label="Utdelningsadress" id="street" value=street />
                <Field label="Postnummer" id="postal_code" value=postal_code />
                <Field label="Postort" id="city" value=city />
                <Field label="Räkenskapsåret börjar" id="fiscal_year_start" kind="date" value=start />
                <Show
                    when=move || custom_end.get()
                    fallback=move || {
                        view! {
                            <p class="text-xs/relaxed text-muted-foreground">
                                {move || match derived_end.get() {
                                    Some(end) => format!("Räkenskapsåret slutar {end}."),
                                    None => "Räkenskapsåret börjar den 1:a i en månad.".to_owned(),
                                }}
                            </p>
                        }
                    }
                >
                    <Field
                        label="Räkenskapsåret slutar"
                        id="fiscal_year_end"
                        kind="date"
                        value=end
                        hint=Signal::derive(|| {
                            Some(
                                "Första räkenskapsåret får vara 1–18 månader. Enskild firma och handelsbolag följer kalenderåret.",
                            )
                        })
                    />
                </Show>
                <Checkbox
                    label="Första räkenskapsåret är förkortat eller förlängt"
                    id="custom_fiscal_year_end"
                    checked=custom_end
                />
                <fieldset class="grid gap-2">
                    <legend class="text-xs/relaxed font-medium">"Bokföringsmetod"</legend>
                    <Radio
                        label="Faktureringsmetoden"
                        name="method"
                        checked=Signal::derive(move || method.get() == cpb::AccountingMethod::Invoice)
                        on_select=move || method.set(cpb::AccountingMethod::Invoice)
                    />
                    <Radio
                        label="Kontantmetoden"
                        name="method"
                        checked=Signal::derive(move || method.get() == cpb::AccountingMethod::Cash)
                        on_select=move || method.set(cpb::AccountingMethod::Cash)
                    />
                    <p class="text-xs/relaxed text-muted-foreground">
                        "Kontantmetoden får bara användas om nettoomsättningen normalt är högst 3 miljoner kronor per år."
                    </p>
                </fieldset>
                <ErrorAlert message=error />
                <Button disabled=busy>"Spara företag"</Button>
            </form>
        </Card>
    }
}
