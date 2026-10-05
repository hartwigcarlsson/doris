//! The overview: the active company's key figures, what needs doing, and
//! the chosen fiscal year at a glance. Every number comes from
//! `crate::overview`; this file loads and draws.

use crate::active_company::Companies;
use crate::api::{company_api, cpb, ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::{FiscalYearSelect, keep_year_in_url};
use crate::format::{accounting_method_label, legal_form_label, today};
use crate::overview::{KeyFigures, default_year, key_figures, progress, whole_kronor};
use crate::ui::{Badge, BadgeVariant, Card, IconName, LinkButton, PageHeader, Panel, Variant};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::use_query_map;

/// `None` while loading; `Err` holds the Swedish message.
type Loaded<T> = RwSignal<Option<Result<T, String>>>;

/// A card on the overview: a `Panel` with its heading.
#[component]
fn OverviewCard(
    title: &'static str,
    #[prop(optional)] class: &'static str,
    children: Children,
) -> impl IntoView {
    view! {
        <Panel class=class>
            <div class="grid gap-3">
                <h2 class="text-sm font-medium">{title}</h2>
                {children()}
            </div>
        </Panel>
    }
}

/// "Laddar…", the error, or `view` of what arrived.
fn pending<T: 'static>(
    data: impl Fn() -> Option<Result<T, String>> + 'static,
    view: impl Fn(T) -> AnyView + 'static,
) -> impl Fn() -> AnyView {
    move || match data() {
        None => view! { <p class="text-muted-foreground">"Laddar…"</p> }.into_any(),
        Some(Err(message)) => {
            view! { <p role="alert" class="text-destructive">{message}</p> }.into_any()
        }
        Some(Ok(value)) => view(value),
    }
}

#[component]
pub fn Home() -> impl IntoView {
    let companies = expect_context::<Companies>();
    view! {
        <Show when=move || !companies.active.get().is_empty() fallback=|| view! { <NoCompany /> }>
            <Overview />
        </Show>
    }
}

/// The start page before the first company exists.
#[component]
fn NoCompany() -> impl IntoView {
    let companies = expect_context::<Companies>();
    view! {
        <div class="grid gap-6">
            <PageHeader title="Översikt" />
            <Show when=move || companies.loaded.get()>
                <Card title="Aktivt företag" narrow=true>
                    <div class="grid gap-2">
                        <p class="text-muted-foreground">"Du har inga företag än."</p>
                        <A href="/companies/new" attr:class="font-medium underline-offset-4 hover:underline">
                            "Lägg till företag"
                        </A>
                    </div>
                </Card>
            </Show>
        </div>
    }
}

#[component]
fn Overview() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let preferred = use_query_map()
        .read_untracked()
        .get("fy")
        .unwrap_or_default();

    let company: Loaded<cpb::Company> = RwSignal::new(None);
    let years: Loaded<Vec<lpb::FiscalYear>> = RwSignal::new(None);
    // The same years for the select, which wants a plain list.
    let year_list = RwSignal::new(Vec::<lpb::FiscalYear>::new());
    let year = RwSignal::new(String::new());
    let balance: Loaded<Vec<lpb::TrialBalanceRow>> = RwSignal::new(None);
    let vouchers: Loaded<Vec<lpb::Voucher>> = RwSignal::new(None);
    keep_year_in_url("/".into(), year);

    // Per company: who it is and which years it has.
    Effect::new(move |_| {
        let company_id = companies.active.get();
        // Never leave the previous company's figures on screen.
        company.set(None);
        years.set(None);
        year_list.set(Vec::new());
        year.set(String::new());
        if company_id.is_empty() {
            return;
        }
        let preferred = preferred.clone();
        spawn_local(async move {
            let found = company_api()
                .get_company(cpb::GetCompanyRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let listed = ledger_api()
                .list_fiscal_years(lpb::ListFiscalYearsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: these answers are stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            company.set(Some(
                found.map(|r| r.into_inner()).map_err(|s| describe(&s)),
            ));
            match listed {
                Ok(response) => {
                    let list = response.into_inner().fiscal_years;
                    year_list.set(list.clone());
                    year.set(default_year(&list, &preferred, &today()));
                    years.set(Some(Ok(list)));
                }
                Err(status) => years.set(Some(Err(describe(&status)))),
            }
        });
    });

    // Per year: the trial balance and the vouchers.
    Effect::new(move |_| {
        let start = year.get();
        let company_id = companies.active.get_untracked();
        balance.set(None);
        vouchers.set(None);
        if start.is_empty() || company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            // Stale once the company or the year has changed.
            let current = {
                let (company_id, start) = (company_id.clone(), start.clone());
                move || {
                    company_id == companies.active.get_untracked() && start == year.get_untracked()
                }
            };
            let rows = ledger_api()
                .get_trial_balance(lpb::GetTrialBalanceRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                })
                .await;
            if current() {
                balance.set(Some(
                    rows.map(|r| r.into_inner().rows).map_err(|s| describe(&s)),
                ));
            }
            let listed = ledger_api()
                .list_vouchers(lpb::ListVouchersRequest {
                    company_id,
                    fiscal_year_start: start,
                })
                .await;
            if current() {
                vouchers.set(Some(
                    listed
                        .map(|r| r.into_inner().vouchers)
                        .map_err(|s| describe(&s)),
                ));
            }
        });
    });

    // The name is known from the header's list before GetCompany answers.
    let title = Signal::derive(move || match company.get() {
        Some(Ok(c)) => c.name,
        _ => companies
            .active_company()
            .map(|c| c.name)
            .unwrap_or_default(),
    });
    let description = Signal::derive(move || match company.get() {
        Some(Ok(c)) => format!(
            "{} · {} · {}",
            c.org_nr,
            legal_form_label(c.legal_form()),
            accounting_method_label(c.accounting_method())
        ),
        _ => String::new(),
    });
    let chosen = Signal::derive(move || {
        year_list.with(|ys| ys.iter().find(|y| y.start == year.get()).cloned())
    });

    view! {
        <div class="grid gap-6">
            <PageHeader title=title description=description>
                <FiscalYearSelect years=year_list year=year />
                <LinkButton href="/payroll-runs/new" variant=Variant::Outline>"Ny lönekörning"</LinkButton>
                <LinkButton href="/supplier-invoices/new" variant=Variant::Outline>"Ny leverantörsfaktura"</LinkButton>
                <LinkButton href="/customer-invoices/new" variant=Variant::Outline>"Ny kundfaktura"</LinkButton>
                <LinkButton href="/vouchers/new" icon=IconName::Plus>"Ny verifikation"</LinkButton>
            </PageHeader>
            <div class="grid grid-cols-[repeat(auto-fit,minmax(min(220px,100%),1fr))] gap-4">
                <KeyFigure title="Resultat hittills i år" note="Efter finansiella poster" balance=balance pick=|f| f.result />
                <KeyFigure title="Intäkter" note="Konto 3000–3999" balance=balance pick=|f| f.income />
                <KeyFigure title="Kostnader" note="Konto 4000–8989" balance=balance pick=|f| f.costs />
                <KeyFigure title="Kassa och bank" note="Konto 1900–1999" balance=balance pick=|f| f.cash />
            </div>
            <div class="flex flex-wrap gap-4">
                <FiscalYearCard years=years chosen=chosen vouchers=vouchers />
            </div>
        </div>
    }
}

/// One headline number from the trial balance.
#[component]
fn KeyFigure(
    title: &'static str,
    note: &'static str,
    balance: Loaded<Vec<lpb::TrialBalanceRow>>,
    pick: fn(KeyFigures) -> i64,
) -> impl IntoView {
    view! {
        <Panel>
            <div class="grid gap-1">
                <h2 class="text-xs/relaxed font-normal text-muted-foreground">{title}</h2>
                {pending(
                    move || balance.get(),
                    move |rows| {
                        view! {
                            <p class="text-2xl/8 font-semibold tracking-tight tabular-nums">
                                {whole_kronor(pick(key_figures(&rows)))}
                            </p>
                        }
                            .into_any()
                    },
                )}
                <p class="text-muted-foreground">{note}</p>
            </div>
        </Panel>
    }
}

/// The chosen year: open or closed, how far in, and what is booked.
#[component]
fn FiscalYearCard(
    years: Loaded<Vec<lpb::FiscalYear>>,
    chosen: Signal<Option<lpb::FiscalYear>>,
    vouchers: Loaded<Vec<lpb::Voucher>>,
) -> impl IntoView {
    const ROW: &str = "flex justify-between gap-3 border-t py-2";
    view! {
        <OverviewCard title="Räkenskapsåret" class="min-w-0 flex-[1_1_280px]">
            {pending(
                move || years.get(),
                move |list| {
                    let Some(fiscal_year) = chosen.get() else {
                        return view! { <p class="text-muted-foreground">"Inget räkenskapsår."</p> }
                            .into_any();
                    };
                    let progress = progress(&fiscal_year.start, &fiscal_year.end, &today());
                    // Years are newest first: the one before is listed next.
                    let previous = list
                        .iter()
                        .position(|y| y.start == fiscal_year.start)
                        .and_then(|i| list.get(i + 1))
                        .cloned();
                    view! {
                        <div>
                            {if fiscal_year.closed {
                                view! { <Badge variant=BadgeVariant::Outline>"Stängt"</Badge> }.into_any()
                            } else {
                                view! { <Badge>"Öppet"</Badge> }.into_any()
                            }}
                        </div>
                        {progress
                            .map(|p| {
                                let left = if p.left == 1 {
                                    "1 dag kvar".to_owned()
                                } else {
                                    format!("{} dagar kvar", p.left)
                                };
                                view! {
                                    <div class="grid gap-2">
                                        <div
                                            role="progressbar"
                                            aria-label="Andel av året som har gått"
                                            aria-valuemin="0"
                                            aria-valuemax="100"
                                            aria-valuenow=p.percent.to_string()
                                            class="h-1.5 overflow-hidden rounded-full bg-muted"
                                        >
                                            <div class="h-full bg-primary" style=format!("width: {}%", p.percent)></div>
                                        </div>
                                        <div class="flex justify-between gap-2 text-muted-foreground">
                                            <span>{format!("Dag {} av {}", p.day, p.days)}</span>
                                            <span>{left}</span>
                                        </div>
                                    </div>
                                }
                            })}
                        <dl>
                            <div class=ROW>
                                <dt class="text-muted-foreground">"Period"</dt>
                                <dd class="tabular-nums">{format!("{} – {}", fiscal_year.start, fiscal_year.end)}</dd>
                            </div>
                            {move || match vouchers.get() {
                                Some(Ok(list)) => {
                                    let latest = list.iter().map(|v| v.date.clone()).max();
                                    view! {
                                        <div class=ROW>
                                            <dt class="text-muted-foreground">"Verifikationer"</dt>
                                            <dd class="tabular-nums">{list.len()}</dd>
                                        </div>
                                        <div class=ROW>
                                            <dt class="text-muted-foreground">"Senast bokfört"</dt>
                                            <dd class="tabular-nums">{latest.unwrap_or_else(|| "–".into())}</dd>
                                        </div>
                                    }
                                        .into_any()
                                }
                                _ => ().into_any(),
                            }}
                            {previous
                                .map(|p| {
                                    view! {
                                        <div class=ROW>
                                            <dt class="text-muted-foreground">
                                                {format!("Föregående år, {}", p.start.get(..4).unwrap_or_default())}
                                            </dt>
                                            <dd>{if p.closed { "Stängt" } else { "Öppet" }}</dd>
                                        </div>
                                    }
                                })}
                        </dl>
                        <A href="/fiscal-years" attr:class="font-medium underline-offset-4 hover:underline">
                            "Visa räkenskapsår"
                        </A>
                    }
                        .into_any()
                },
            )}
        </OverviewCard>
    }
}
