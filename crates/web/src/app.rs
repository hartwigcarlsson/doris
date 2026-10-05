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

/// The design is kept by tests as well as by habit: a new route has to be
/// given to the design test and to the menu, or these fail.
#[cfg(test)]
mod tests {
    use crate::nav::section_of;

    const APP: &str = include_str!("app.rs");
    const NAV: &str = include_str!("nav.rs");
    const DESIGN_SPEC: &str = include_str!("../../../e2e/tests/design.spec.ts");

    /// Pages without the signed-in shell.
    const SIGNED_OUT: [&str; 2] = ["/register", "/login"];
    /// Pages that belong to no menu group: the start page and the account menu's.
    const OUTSIDE_THE_GROUPS: [&str; 4] = ["/", "/companies", "/settings", "/admin"];
    /// Top-level pages reached from another page rather than from the menu.
    const NOT_IN_THE_MENU: [&str; 1] = ["/opening-balances"];

    /// The path of every `path!` route in `source`.
    fn routes(source: &str) -> Vec<&str> {
        source
            .split("path!(\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect()
    }

    /// The paths the e2e test "every signed-in view has one h1…" visits.
    fn design_paths(spec: &str) -> Vec<&str> {
        let list = spec
            .split("const paths = [")
            .nth(1)
            .and_then(|rest| rest.split("];").next())
            .unwrap_or_default();
        list.split('"').skip(1).step_by(2).collect()
    }

    /// Whether `path` is an instance of `route` (`:name` matches any segment).
    fn is_instance(route: &str, path: &str) -> bool {
        let (route, path): (Vec<_>, Vec<_>) =
            (route.split('/').collect(), path.split('/').collect());
        route.len() == path.len()
            && route
                .iter()
                .zip(&path)
                .all(|(r, p)| r.starts_with(':') || r == p)
    }

    /// The signed-in routes that none of `visited` is an instance of.
    fn unvisited<'a>(routes: &[&'a str], visited: &[&str]) -> Vec<&'a str> {
        routes
            .iter()
            .filter(|route| !SIGNED_OUT.contains(route))
            .filter(|route| !visited.iter().any(|path| is_instance(route, path)))
            .copied()
            .collect()
    }

    #[test]
    fn the_checks_notice_a_route_that_was_forgotten() {
        let all = ["/login", "/vouchers", "/vouchers/:id", "/new-page"];
        assert_eq!(
            unvisited(&all, &["/vouchers", "/vouchers/7"]),
            ["/new-page"]
        );
        assert_eq!(
            unvisited(&all, &["/vouchers", "/new-page"]),
            ["/vouchers/:id"]
        );
        assert_eq!(
            routes("a path!(\"/x\") b path!(\"/y/:id\")"),
            ["/x", "/y/:id"]
        );
        assert_eq!(
            design_paths("const paths = [\n \"/\", \"/a\",\n \"/b\",\n];"),
            ["/", "/a", "/b"]
        );
    }

    #[test]
    fn every_signed_in_route_is_in_the_design_test() {
        let (routes, visited) = (routes(APP), design_paths(DESIGN_SPEC));
        assert!(
            routes.len() > 20 && visited.len() > 15,
            "the sources were not read"
        );
        assert_eq!(
            unvisited(&routes, &visited),
            Vec::<&str>::new(),
            "add these to `paths` in e2e/tests/design.spec.ts, so the page's h1 and width are checked"
        );
    }

    #[test]
    fn every_page_belongs_to_a_menu_group_and_top_level_pages_are_linked() {
        for route in routes(APP) {
            let outside = |prefixes: &[&str]| {
                prefixes
                    .iter()
                    .any(|p| route == *p || (*p != "/" && route.starts_with(&format!("{p}/"))))
            };
            if SIGNED_OUT.contains(&route) || outside(&OUTSIDE_THE_GROUPS) {
                continue;
            }
            assert!(
                section_of(route).is_some(),
                "{route}: add it to SECTIONS in nav.rs, so its menu is marked when the page is open"
            );
            let top_level = route.matches('/').count() == 1;
            if top_level && !NOT_IN_THE_MENU.contains(&route) {
                assert!(
                    NAV.contains(&format!("href=\"{route}\"")),
                    "{route}: add a NavItem for it in nav.rs, or list it in NOT_IN_THE_MENU"
                );
            }
        }
    }
}
