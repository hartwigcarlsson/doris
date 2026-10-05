//! The overview: the active company's key figures, what needs doing, and
//! the chosen fiscal year at a glance. Every number comes from
//! `crate::overview`; this file loads and draws.

use crate::active_company::Companies;
use crate::api::{company_api, cpb, invoicing_api, ipb, ledger_api, lpb, payroll_api, ppb};
use crate::errors::describe;
use crate::fiscal_year::{FiscalYearSelect, keep_year_in_url};
use crate::format::{accounting_method_label, amount, legal_form_label, today};
use crate::overview::{
    KeyFigures, Todo, TodoInput, bar_height, by_month, default_year, key_figures, month_label,
    progress, scale, todo_list, unpaid_supplier_invoices, whole_kronor,
};
use crate::ui::{
    Badge, BadgeVariant, Card, Icon, IconName, LinkButton, PageHeader, Panel, TABLE_AMOUNT_CELL,
    TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table, Variant,
};
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
    // What "Att göra" reads; none of it depends on the year.
    let supplier_invoices: Loaded<Vec<ipb::SupplierInvoice>> = RwSignal::new(None);
    let customer_invoices: Loaded<Vec<ipb::CustomerInvoice>> = RwSignal::new(None);
    let payroll_runs: Loaded<Vec<ppb::PayrollRun>> = RwSignal::new(None);
    let agi_months: Loaded<Vec<ppb::AgiMonthSummary>> = RwSignal::new(None);
    keep_year_in_url("/".into(), year);

    // Per company: who it is and which years it has.
    Effect::new(move |_| {
        let company_id = companies.active.get();
        // Never leave the previous company's figures on screen.
        company.set(None);
        years.set(None);
        year_list.set(Vec::new());
        year.set(String::new());
        supplier_invoices.set(None);
        customer_invoices.set(None);
        payroll_runs.set(None);
        agi_months.set(None);
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
            let suppliers = invoicing_api()
                .list_supplier_invoices(ipb::ListSupplierInvoicesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let customers = invoicing_api()
                .list_customer_invoices(ipb::ListCustomerInvoicesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let runs = payroll_api()
                .list_payroll_runs(ppb::ListPayrollRunsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let months = payroll_api()
                .list_agi_months(ppb::ListAgiMonthsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            supplier_invoices.set(Some(
                suppliers
                    .map(|r| r.into_inner().invoices)
                    .map_err(|s| describe(&s)),
            ));
            customer_invoices.set(Some(
                customers
                    .map(|r| r.into_inner().invoices)
                    .map_err(|s| describe(&s)),
            ));
            payroll_runs.set(Some(
                runs.map(|r| r.into_inner().payroll_runs)
                    .map_err(|s| describe(&s)),
            ));
            agi_months.set(Some(
                months
                    .map(|r| r.into_inner().months)
                    .map_err(|s| describe(&s)),
            ));
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

    // "Att göra" needs all four lists: loading until the last one is in,
    // and the first error if any call failed.
    let todos = Signal::derive(move || {
        let (s, c, r, m) = (
            supplier_invoices.get()?,
            customer_invoices.get()?,
            payroll_runs.get()?,
            agi_months.get()?,
        );
        Some((|| {
            let (s, c, r, m) = (s?, c?, r?, m?);
            Ok::<_, String>(todo_list(
                &TodoInput {
                    supplier_invoices: &s,
                    customer_invoices: &c,
                    payroll_runs: &r,
                    agi_months: &m,
                },
                &today(),
            ))
        })())
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
                <TodoCard todos=todos />
                <FiscalYearCard years=years chosen=chosen vouchers=vouchers />
            </div>
            <div class="flex flex-wrap gap-4">
                <MonthChart chosen=chosen vouchers=vouchers />
                <UnpaidCard invoices=supplier_invoices />
            </div>
            <LatestVouchers vouchers=vouchers />
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
                                            <div class="h-full bg-chart-1" style=format!("width: {}%", p.percent)></div>
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

/// What needs doing, most pressing first.
#[component]
fn TodoCard(todos: Signal<Option<Result<Vec<Todo>, String>>>) -> impl IntoView {
    view! {
        <Panel class="min-w-0 flex-[2_1_480px] px-0 pb-1">
            <div class="grid gap-3">
                <div class="flex items-center gap-2 px-4">
                    <h2 class="text-sm font-medium">"Att göra"</h2>
                    {move || todos.get().and_then(Result::ok).filter(|l| !l.is_empty()).map(|l| view! { <Badge>{l.len()}</Badge> })}
                </div>
                {pending(move || todos.get(), |list: Vec<Todo>| {
                    if list.is_empty() {
                        return view! { <p class="px-4 pb-3 text-muted-foreground">"Inget att göra just nu."</p> }.into_any();
                    }
                    view! {
                        <ul>
                            {list
                                .into_iter()
                                .map(|item| {
                                    let (round, icon) = if item.urgent {
                                        ("bg-destructive/10 text-destructive dark:bg-destructive/20", IconName::CircleAlert)
                                    } else {
                                        ("bg-muted", IconName::Clock)
                                    };
                                    view! {
                                        <li class="flex flex-wrap items-center gap-3 border-t px-4 py-3">
                                            <span class=format!("flex size-7 shrink-0 items-center justify-center rounded-full {round}")>
                                                <Icon name=icon />
                                            </span>
                                            <div class="min-w-0 flex-[1_1_240px]">
                                                <p class="font-medium">{item.title}</p>
                                                <p class="text-muted-foreground">{item.detail}</p>
                                            </div>
                                            <LinkButton href=item.href variant=Variant::Outline>{item.action}</LinkButton>
                                        </li>
                                    }
                                })
                                .collect_view()}
                        </ul>
                    }
                        .into_any()
                })}
            </div>
        </Panel>
    }
}

/// The unpaid supplier invoices: how many, how much, and the next four.
#[component]
fn UnpaidCard(invoices: Loaded<Vec<ipb::SupplierInvoice>>) -> impl IntoView {
    view! {
        <OverviewCard title="Obetalda leverantörsfakturor" class="min-w-0 flex-[1_1_280px]">
            {pending(move || invoices.get(), |list: Vec<ipb::SupplierInvoice>| {
                let unpaid = unpaid_supplier_invoices(&list);
                if unpaid.is_empty() {
                    return view! { <p class="text-muted-foreground">"Inga obetalda leverantörsfakturor."</p> }.into_any();
                }
                let today = today();
                let total: i64 = unpaid.iter().map(|i| i.total).sum();
                let summary = format!(
                    "{} {}, {} kr",
                    unpaid.len(),
                    if unpaid.len() == 1 { "faktura" } else { "fakturor" },
                    amount(total)
                );
                view! {
                    <p class="text-muted-foreground">{summary}</p>
                    <ul>
                        {unpaid
                            .into_iter()
                            .take(4)
                            .map(|invoice| {
                                let late = invoice.due_date < today;
                                view! {
                                    <li class="flex items-center justify-between gap-3 border-t py-2">
                                        <div class="min-w-0">
                                            <p class="truncate font-medium">{invoice.supplier_name.clone()}</p>
                                            <p class="flex items-center gap-1.5 text-muted-foreground">
                                                {format!("{} {}", if late { "Förföll" } else { "Förfaller" }, invoice.due_date)}
                                                {late.then(|| view! { <Badge variant=BadgeVariant::Destructive>"Förfallen"</Badge> })}
                                            </p>
                                        </div>
                                        <p class="whitespace-nowrap tabular-nums">{amount(invoice.total)}</p>
                                    </li>
                                }
                            })
                            .collect_view()}
                    </ul>
                    <A href="/supplier-invoices" attr:class="font-medium underline-offset-4 hover:underline">"Alla leverantörsfakturor"</A>
                }
                    .into_any()
            })}
        </OverviewCard>
    }
}

/// The five vouchers with the highest numbers.
#[component]
fn LatestVouchers(vouchers: Loaded<Vec<lpb::Voucher>>) -> impl IntoView {
    view! {
        <OverviewCard title="Senaste verifikationer">
            {pending(move || vouchers.get(), |mut list: Vec<lpb::Voucher>| {
                if list.is_empty() {
                    return view! { <p class="text-muted-foreground">"Inga verifikationer än."</p> }.into_any();
                }
                list.sort_by_key(|v| std::cmp::Reverse(v.number));
                view! {
                    <Table>
                        <thead class=TABLE_HEAD>
                            <tr class=TABLE_ROW>
                                <th class=TABLE_HEADER_CELL>"Nr"</th>
                                <th class=TABLE_HEADER_CELL>"Datum"</th>
                                <th class=TABLE_HEADER_CELL>"Text"</th>
                                <th class=format!("{TABLE_HEADER_CELL} text-right")>"Belopp"</th>
                            </tr>
                        </thead>
                        <tbody class=TABLE_BODY>
                            {list
                                .into_iter()
                                .take(5)
                                .map(|v| {
                                    let total: i64 = v.lines.iter().map(|l| l.debit).sum();
                                    view! {
                                        <tr class=TABLE_ROW>
                                            <td class=format!("{TABLE_CELL} tabular-nums")>{v.number}</td>
                                            <td class=format!("{TABLE_CELL} tabular-nums")>{v.date}</td>
                                            <td class=TABLE_CELL>{v.text}</td>
                                            <td class=TABLE_AMOUNT_CELL>{amount(total)}</td>
                                        </tr>
                                    }
                                })
                                .collect_view()}
                        </tbody>
                    </Table>
                    <A href="/vouchers" attr:class="font-medium underline-offset-4 hover:underline">"Alla verifikationer"</A>
                }
                    .into_any()
            })}
        </OverviewCard>
    }
}

/// The chart's plot height in pixels.
const PLOT: u32 = 160;

/// Income and costs per month, as two bars a month.
#[component]
fn MonthChart(
    chosen: Signal<Option<lpb::FiscalYear>>,
    vouchers: Loaded<Vec<lpb::Voucher>>,
) -> impl IntoView {
    // The axis counts thousands of kronor, or kronor while the largest
    // month is under 2 000 kr and the half-way line would read "0".
    let top = Signal::derive(move || match (chosen.get(), vouchers.get()) {
        (Some(y), Some(Ok(list))) => scale(&by_month(&y.start, &y.end, &list)),
        _ => 0,
    });
    let thousands = move || top.get() == 0 || top.get() >= 200_000;
    view! {
        <Panel class="min-w-0 flex-[2_1_480px]">
            <div class="grid gap-3">
                <div class="flex flex-wrap items-start justify-between gap-4">
                    <div>
                        <h2 class="text-sm font-medium">"Intäkter och kostnader per månad"</h2>
                        <p class="text-muted-foreground">{move || if thousands() { "Tusental kronor" } else { "Kronor" }}</p>
                    </div>
                    <ul class="flex gap-4">
                        <li class="flex items-center gap-1.5"><span data-legend class="size-2 rounded-xs bg-chart-1"></span>"Intäkter"</li>
                        <li class="flex items-center gap-1.5"><span data-legend class="size-2 rounded-xs bg-chart-2"></span>"Kostnader"</li>
                    </ul>
                </div>
                {pending(move || vouchers.get(), move |list: Vec<lpb::Voucher>| {
                    let Some(fiscal_year) = chosen.get() else {
                        return ().into_any();
                    };
                    let months = by_month(&fiscal_year.start, &fiscal_year.end, &list);
                    let top = scale(&months);
                    let unit = if thousands() { 100_000 } else { 100 };
                    let this_month = today().get(..7).unwrap_or_default().to_owned();
                    // Gridlines at half and full scale.
                    let label = move |ore: i64| (ore / unit).to_string();
                    let columns = format!("grid-template-columns: repeat({}, minmax(0, 1fr))", months.len().max(1));
                    view! {
                        <figure
                            role="img"
                            aria-label=format!("Intäkter och kostnader per månad, {} – {}", fiscal_year.start, fiscal_year.end)
                            class="grid gap-1.5"
                        >
                            <div class="flex gap-2">
                                <div class="relative w-8 text-right text-muted-foreground tabular-nums" style=format!("height: {PLOT}px")>
                                    <span class="absolute right-0 bottom-0 translate-y-1/2 leading-none">"0"</span>
                                    {(top > 0).then(|| view! {
                                        <span class="absolute right-0 bottom-1/2 translate-y-1/2 leading-none">{label(top / 2)}</span>
                                        <span class="absolute top-0 right-0 -translate-y-1/2 leading-none">{label(top)}</span>
                                    })}
                                </div>
                                <div class="relative min-w-0 flex-1 border-b" style=format!("height: {PLOT}px")>
                                    <div class="absolute inset-x-0 top-0 border-t"></div>
                                    <div class="absolute inset-x-0 top-1/2 border-t"></div>
                                    <div class="absolute inset-0 grid items-end" style=columns.clone()>
                                        {months
                                            .iter()
                                            .map(|m| view! {
                                                <div data-month=m.month.clone() class="flex items-end justify-center gap-0.5">
                                                    <div class="w-3 max-w-[40%] rounded-t-sm bg-chart-1" style=format!("height: {}px", bar_height(m.income, top, PLOT))></div>
                                                    <div class="w-3 max-w-[40%] rounded-t-sm bg-chart-2" style=format!("height: {}px", bar_height(m.costs, top, PLOT))></div>
                                                </div>
                                            })
                                            .collect_view()}
                                    </div>
                                </div>
                            </div>
                            <div class="ml-10 grid text-center text-muted-foreground" style=columns>
                                {months
                                    .iter()
                                    .map(|m| {
                                        let current = m.month == this_month;
                                        view! { <span class=if current { "font-medium text-foreground" } else { "" }>{month_label(&m.month)}</span> }
                                    })
                                    .collect_view()}
                            </div>
                        </figure>
                        <A href="/financial-statements" attr:class="font-medium underline-offset-4 hover:underline">"Visa resultaträkningen"</A>
                    }
                        .into_any()
                })}
            </div>
        </Panel>
    }
}
