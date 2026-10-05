//! Routes, the session state and the page shell.

use crate::active_company::Companies;
use crate::api::{api, pb, prefetched_status};
use crate::nav::Header;
use crate::pages::{
    AccountLedger, Accounts, Agi, Companies, CompanyPage, CustomerInvoices, Customers, Employees,
    FinancialStatements, FiscalYears, Home, Invitations, Login, NewCompany, NewCustomerInvoice,
    NewSupplierInvoice, NewVoucher, OpeningBalances, Passkeys, PayrollRunPage, PayrollRuns,
    Register, SupplierInvoices, Suppliers, TrialBalance, Vouchers,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::{Redirect, Route, Router, Routes};
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
        let status = match prefetched_status().await {
            Some(status) => Some(status),
            None => api()
                .get_status(pb::GetStatusRequest {})
                .await
                .ok()
                .map(|status| status.into_inner()),
        };
        if let Some(status) = status {
            session.bootstrap_required.set(status.bootstrap_required);
            session.user.set(status.current_user);
        }
        session.loaded.set(true);
    });

    view! {
        <Router>
            <Header />
            <main class="mx-auto w-full max-w-6xl px-4 py-10">
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
                        <Route path=path!("/customer-invoices") view=|| view! { <SignedIn><CustomerInvoices /></SignedIn> } />
                        <Route path=path!("/customer-invoices/new") view=|| view! { <SignedIn><NewCustomerInvoice /></SignedIn> } />
                        <Route path=path!("/suppliers") view=|| view! { <SignedIn><Suppliers /></SignedIn> } />
                        <Route path=path!("/supplier-invoices") view=|| view! { <SignedIn><SupplierInvoices /></SignedIn> } />
                        <Route path=path!("/supplier-invoices/new") view=|| view! { <SignedIn><NewSupplierInvoice /></SignedIn> } />
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
