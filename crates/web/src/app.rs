//! Routes, the session state and the page shell.

use crate::active_company::{ActiveCompanySelect, Companies};
use crate::api::{api, pb};
use crate::pages::{
    AccountLedger, Accounts, Agi, Companies, CompanyPage, Customers, Employees,
    FinancialStatements, FiscalYears, Home, Invitations, Login, NewCompany, NewVoucher,
    OpeningBalances, Passkeys, PayrollRunPage, PayrollRuns, Register, Suppliers, TrialBalance,
    Vouchers,
};
use crate::ui::{Button, Variant};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::{A, Redirect, Route, Router, Routes};
use leptos_router::hooks::use_navigate;
use leptos_router::path;

/// Who is signed in, as last reported by the server.
#[derive(Clone, Copy)]
pub struct Session {
    pub loaded: RwSignal<bool>,
    pub bootstrap_required: RwSignal<bool>,
    pub user: RwSignal<Option<pb::User>>,
}

impl Session {
    pub fn signed_in(&self, user: pb::User) {
        self.bootstrap_required.set(false);
        self.user.set(Some(user));
    }

    pub fn is_admin(&self) -> bool {
        self.user
            .get()
            .is_some_and(|user| user.role() == pb::Role::Admin)
    }
}

#[component]
pub fn App() -> impl IntoView {
    let session = Session {
        loaded: RwSignal::new(false),
        bootstrap_required: RwSignal::new(false),
        user: RwSignal::new(None),
    };
    provide_context(session);
    let companies = Companies::new();
    provide_context(companies);
    Effect::new(move |_| match session.user.get() {
        Some(user) => companies.load(user.id),
        None => companies.clear(),
    });
    spawn_local(async move {
        if let Ok(status) = api().get_status(pb::GetStatusRequest {}).await {
            let status = status.into_inner();
            session.bootstrap_required.set(status.bootstrap_required);
            session.user.set(status.current_user);
        }
        session.loaded.set(true);
    });

    view! {
        <Router>
            <Header />
            // A page that marks an element `data-wide` (the ledger's tables and line
            // editor) gets the header's width; forms stay narrow.
            <main class="mx-auto w-full max-w-sm px-4 py-10 has-[[data-wide]]:max-w-3xl">
                <Show when=move || session.loaded.get() fallback=|| view! { <p class="text-muted-foreground">"Laddar…"</p> }>
                    <Routes fallback=|| view! { <p>"Sidan finns inte."</p> }>
                        <Route path=path!("/register") view=Register />
                        <Route path=path!("/login") view=Login />
                        <Route path=path!("/") view=|| view! { <SignedIn><Home /></SignedIn> } />
                        <Route path=path!("/companies") view=|| view! { <SignedIn><Companies /></SignedIn> } />
                        <Route path=path!("/companies/new") view=|| view! { <SignedIn><NewCompany /></SignedIn> } />
                        <Route path=path!("/companies/:id") view=|| view! { <SignedIn><CompanyPage /></SignedIn> } />
                        <Route path=path!("/accounts") view=|| view! { <SignedIn><Accounts /></SignedIn> } />
                        <Route path=path!("/vouchers") view=|| view! { <SignedIn><Vouchers /></SignedIn> } />
                        <Route path=path!("/customers") view=|| view! { <SignedIn><Customers /></SignedIn> } />
                        <Route path=path!("/suppliers") view=|| view! { <SignedIn><Suppliers /></SignedIn> } />
                        <Route path=path!("/vouchers/new") view=|| view! { <SignedIn><NewVoucher /></SignedIn> } />
                        <Route path=path!("/trial-balance") view=|| view! { <SignedIn><TrialBalance /></SignedIn> } />
                        <Route path=path!("/trial-balance/:account") view=|| view! { <SignedIn><AccountLedger /></SignedIn> } />
                        <Route path=path!("/financial-statements") view=|| view! { <SignedIn><FinancialStatements /></SignedIn> } />
                        <Route path=path!("/fiscal-years") view=|| view! { <SignedIn><FiscalYears /></SignedIn> } />
                        <Route path=path!("/opening-balances") view=|| view! { <SignedIn><OpeningBalances /></SignedIn> } />
                        <Route path=path!("/employees") view=|| view! { <SignedIn><Employees /></SignedIn> } />
                        <Route path=path!("/agi") view=|| view! { <SignedIn><Agi /></SignedIn> } />
                        <Route path=path!("/payroll-runs") view=|| view! { <SignedIn><PayrollRuns /></SignedIn> } />
                        <Route path=path!("/payroll-runs/new") view=|| view! { <SignedIn><PayrollRunPage /></SignedIn> } />
                        <Route path=path!("/payroll-runs/:id") view=|| view! { <SignedIn><PayrollRunPage /></SignedIn> } />
                        <Route path=path!("/settings/passkeys") view=|| view! { <SignedIn><Passkeys /></SignedIn> } />
                        <Route path=path!("/admin/invitations") view=|| view! { <SignedIn admin=true><Invitations /></SignedIn> } />
                    </Routes>
                </Show>
            </main>
        </Router>
    }
}

/// Renders `children` only for a signed-in user (an admin, if `admin`);
/// everyone else is sent to registration (first run) or login.
#[component]
fn SignedIn(#[prop(optional)] admin: bool, children: ChildrenFn) -> impl IntoView {
    let session = expect_context::<Session>();
    move || match session.user.get() {
        None if session.bootstrap_required.get() => {
            view! { <Redirect path="/register" /> }.into_any()
        }
        None => view! { <Redirect path="/login" /> }.into_any(),
        Some(_) if admin && !session.is_admin() => {
            view! { <p role="alert" class="text-destructive">"Du saknar behörighet."</p> }
                .into_any()
        }
        Some(_) => children().into_any(),
    }
}

#[component]
fn Header() -> impl IntoView {
    let session = expect_context::<Session>();
    let companies = expect_context::<Companies>();
    let navigate = use_navigate();
    let log_out = move |_| {
        let navigate = navigate.clone();
        spawn_local(async move {
            let _ = api().logout(pb::LogoutRequest {}).await;
            session.user.set(None);
            navigate("/login", Default::default());
        });
    };
    view! {
        // Two rows: the account (which company, your settings) on top, and
        // the active company's bookkeeping below, so the company picker
        // never has to give up width to another page's link. The account
        // links wrap as one group on a narrow screen.
        <header class="border-b">
            <div class="mx-auto max-w-3xl px-4 text-xs/relaxed">
                <nav aria-label="Konto" class="flex min-h-12 flex-wrap items-center gap-x-4 gap-y-2 py-2">
                    <A href="/" attr:class="text-sm font-semibold">"Doris"</A>
                    <Show when=move || session.user.get().is_some()>
                        <ActiveCompanySelect />
                        <div class="ml-auto flex items-center gap-4">
                            <A href="/companies" attr:class="text-muted-foreground hover:text-foreground">"Företag"</A>
                            <A href="/settings/passkeys" attr:class="text-muted-foreground hover:text-foreground">"Passkeys"</A>
                            <Show when=move || session.is_admin()>
                                <A href="/admin/invitations" attr:class="text-muted-foreground hover:text-foreground">"Inbjudningar"</A>
                            </Show>
                            <Button variant=Variant::Ghost kind="button" on:click=log_out.clone()>"Logga ut"</Button>
                        </div>
                    </Show>
                </nav>
                <Show when=move || session.user.get().is_some() && !companies.active.get().is_empty()>
                    <nav aria-label="Bokföring" class="flex flex-wrap items-center gap-x-4 gap-y-2 pb-3">
                        <A href="/vouchers" attr:class="text-muted-foreground hover:text-foreground">"Verifikationer"</A>
                        <A href="/customers" attr:class="text-muted-foreground hover:text-foreground">"Kunder"</A>
                        <A href="/suppliers" attr:class="text-muted-foreground hover:text-foreground">"Leverantörer"</A>
                        <A href="/trial-balance" attr:class="text-muted-foreground hover:text-foreground">"Saldobalans"</A>
                        <A href="/financial-statements" attr:class="text-muted-foreground hover:text-foreground">"Rapporter"</A>
                        <A href="/fiscal-years" attr:class="text-muted-foreground hover:text-foreground">"Räkenskapsår"</A>
                        <A href="/accounts" attr:class="text-muted-foreground hover:text-foreground">"Kontoplan"</A>
                        <A href="/payroll-runs" attr:class="text-muted-foreground hover:text-foreground">"Lönekörningar"</A>
                        <A href="/employees" attr:class="text-muted-foreground hover:text-foreground">"Anställda"</A>
                        <A href="/agi" attr:class="text-muted-foreground hover:text-foreground">"Arbetsgivardeklaration"</A>
                    </nav>
                </Show>
            </div>
        </header>
    }
}
