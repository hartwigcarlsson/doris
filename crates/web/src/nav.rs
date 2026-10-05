//! The header: the company picker, the grouped main menu and the account menu.

use crate::active_company::{ActiveCompanySelect, Companies};
use crate::api::{api, pb};
use crate::app::Session;
use crate::ui::{Icon, IconName};
use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::{use_location, use_navigate};
use wasm_bindgen::JsCast;

/// The main menu's groups, for marking where the current page lives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Section {
    Bookkeeping,
    Purchases,
    Customers,
    Payroll,
}

const SECTIONS: [(&str, Section); 11] = [
    ("/vouchers", Section::Bookkeeping),
    ("/trial-balance", Section::Bookkeeping),
    ("/financial-statements", Section::Bookkeeping),
    ("/accounts", Section::Bookkeeping),
    ("/fiscal-years", Section::Bookkeeping),
    ("/opening-balances", Section::Bookkeeping),
    ("/supplier-invoices", Section::Purchases),
    ("/suppliers", Section::Purchases),
    ("/customers", Section::Customers),
    ("/payroll-runs", Section::Payroll),
    ("/employees", Section::Payroll),
];

/// The menu a path belongs to: the page itself or a page under it.
pub fn section_of(path: &str) -> Option<Section> {
    SECTIONS
        .iter()
        .find(|(prefix, _)| {
            path.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
        .map(|(_, section)| *section)
}

/// Up to two initials for the account button: "Erik Berg" → "EB".
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .take(2)
        .filter_map(|word| word.chars().next())
        .flat_map(char::to_uppercase)
        .collect()
}

/// Every menu shares this name, so the browser keeps one open at a time.
const MENU_NAME: &str = "doris-nav";
const TOP: &str = "inline-flex h-7 items-center gap-1 rounded-md px-2 text-muted-foreground hover:bg-muted hover:text-foreground aria-[current=page]:bg-muted aria-[current=page]:font-medium aria-[current=page]:text-foreground data-[current=true]:font-medium data-[current=true]:text-foreground";
const PANEL: &str = "absolute top-8 z-10 grid min-w-46 gap-0 rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10";
const ITEM: &str = "flex h-7 w-full items-center gap-2 rounded-sm px-2 whitespace-nowrap hover:bg-muted aria-[current=page]:bg-muted aria-[current=page]:font-medium";

/// Closes every open menu, except the one `keep` is inside.
fn close_menus(keep: Option<&web_sys::Element>) {
    let Ok(open) = document().query_selector_all(&format!("details[name='{MENU_NAME}'][open]"))
    else {
        return;
    };
    for i in 0..open.length() {
        let Some(menu) = open
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        if keep.is_none_or(|el| !menu.contains(Some(el))) {
            let _ = menu.remove_attribute("open");
        }
    }
}

/// One group in the header. `label` is the menu's visible name unless
/// `summary` draws its own (the account menu); it is always its accessible one.
#[component]
fn NavMenu(
    label: &'static str,
    #[prop(optional, into)] current: Signal<bool>,
    #[prop(optional)] right: bool,
    #[prop(optional, into)] summary: Option<ViewFn>,
    children: Children,
) -> impl IntoView {
    let side = if right { "right-0" } else { "left-0" };
    view! {
        <details name=MENU_NAME class="relative">
            <summary
                class=format!("{TOP} cursor-pointer list-none select-none [&::-webkit-details-marker]:hidden")
                data-current=move || current.get().to_string()
            >
                {match summary {
                    Some(own) => view! { <span class="sr-only">{label}</span> {own.run()} }.into_any(),
                    None => view! { <span>{label}</span> }.into_any(),
                }}
                <Icon name=IconName::ChevronDown />
            </summary>
            <ul class=format!("{PANEL} {side}")>{children()}</ul>
        </details>
    }
}

#[component]
fn NavItem(href: &'static str, icon: IconName, label: &'static str) -> impl IntoView {
    view! {
        <li>
            <A href=href exact=true attr:class=ITEM>
                <Icon name=icon class="size-3.5 text-muted-foreground" />
                {label}
            </A>
        </li>
    }
}

#[component]
pub fn Header() -> impl IntoView {
    let session = expect_context::<Session>();
    let companies = expect_context::<Companies>();
    let navigate = use_navigate();
    let path = use_location().pathname;
    let section = Memo::new(move |_| section_of(&path.get()));
    let in_section = move |wanted: Section| Signal::derive(move || section.get() == Some(wanted));

    // A click outside a menu closes it; so does a click on one of its
    // links, since choosing the current page changes no path.
    let on_click = window_event_listener(ev::click, |event| {
        let target = event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        let on_link = target
            .as_ref()
            .is_some_and(|el| el.closest("details a").ok().flatten().is_some());
        close_menus(if on_link { None } else { target.as_ref() });
    });
    let on_key = window_event_listener(ev::keydown, |event| {
        if event.key() == "Escape" {
            close_menus(None);
        }
    });
    on_cleanup(move || {
        on_click.remove();
        on_key.remove();
    });
    Effect::new(move |_| {
        path.track();
        close_menus(None);
    });

    // Stored so that the click handler is `Copy` and can sit inside `<Show>`.
    let navigate = StoredValue::new_local(navigate);
    let log_out = move |_| {
        spawn_local(async move {
            let _ = api().logout(pb::LogoutRequest {}).await;
            session.user.set(None);
            navigate.with_value(|go| go("/login", Default::default()));
        });
    };
    let name = move || {
        session
            .user
            .get()
            .map(|u| u.display_name)
            .unwrap_or_default()
    };

    view! {
        <header class="border-b">
            <div class="mx-auto flex min-h-12 max-w-6xl flex-wrap items-center gap-x-4 gap-y-2 px-4 py-2 text-xs/relaxed">
                <A href="/" attr:class="text-sm font-semibold">"Doris"</A>
                <Show when=move || session.user.get().is_some()>
                    <ActiveCompanySelect />
                    <Show when=move || !companies.active.get().is_empty()>
                        <nav aria-label="Huvudmeny" class="flex flex-wrap items-center gap-1">
                            <A href="/" exact=true attr:class=TOP>"Översikt"</A>
                            <NavMenu label="Bokföring" current=in_section(Section::Bookkeeping)>
                                <NavItem href="/vouchers" icon=IconName::ReceiptText label="Verifikationer" />
                                <NavItem href="/trial-balance" icon=IconName::Scale label="Saldobalans" />
                                <NavItem href="/financial-statements" icon=IconName::ChartColumn label="Rapporter" />
                                <NavItem href="/accounts" icon=IconName::ListTree label="Kontoplan" />
                                <NavItem href="/fiscal-years" icon=IconName::CalendarRange label="Räkenskapsår" />
                            </NavMenu>
                            <NavMenu label="Inköp" current=in_section(Section::Purchases)>
                                <NavItem href="/supplier-invoices" icon=IconName::FileText label="Leverantörsfakturor" />
                                <NavItem href="/suppliers" icon=IconName::Building2 label="Leverantörer" />
                            </NavMenu>
                            <A href="/customers" attr:class=TOP>"Kunder"</A>
                            <NavMenu label="Lön" current=in_section(Section::Payroll)>
                                <NavItem href="/payroll-runs" icon=IconName::Banknote label="Lönekörningar" />
                                <NavItem href="/employees" icon=IconName::Users label="Anställda" />
                            </NavMenu>
                        </nav>
                    </Show>
                    <div class="ml-auto">
                        <NavMenu
                            label="Konto"
                            right=true
                            summary=move || {
                                view! {
                                    <span aria-hidden="true" class="flex size-5 items-center justify-center rounded-full bg-muted text-[0.625rem] font-medium text-foreground">
                                        {move || initials(&name())}
                                    </span>
                                    <span class="max-w-32 truncate font-medium text-foreground">{name}</span>
                                }
                            }
                        >
                            <NavItem href="/companies" icon=IconName::Building label="Företag" />
                            <NavItem href="/settings/passkeys" icon=IconName::KeyRound label="Passkeys" />
                            <Show when=move || session.is_admin()>
                                <NavItem href="/admin/invitations" icon=IconName::MailPlus label="Inbjudningar" />
                            </Show>
                            <li aria-hidden="true" class="-mx-1 my-1 h-px bg-border"></li>
                            <li>
                                <button type="button" class=ITEM on:click=log_out>
                                    <Icon name=IconName::LogOut class="size-3.5 text-muted-foreground" />
                                    "Logga ut"
                                </button>
                            </li>
                        </NavMenu>
                    </div>
                </Show>
            </div>
        </header>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_belongs_to_the_menu_that_lists_it() {
        use Section::*;
        for (path, section) in [
            ("/vouchers", Some(Bookkeeping)),
            ("/vouchers/new", Some(Bookkeeping)),
            ("/trial-balance", Some(Bookkeeping)),
            ("/trial-balance/1930", Some(Bookkeeping)),
            ("/financial-statements", Some(Bookkeeping)),
            ("/accounts", Some(Bookkeeping)),
            ("/fiscal-years", Some(Bookkeeping)),
            ("/opening-balances", Some(Bookkeeping)),
            ("/supplier-invoices", Some(Purchases)),
            ("/supplier-invoices/new", Some(Purchases)),
            ("/suppliers", Some(Purchases)),
            ("/customers", Some(Customers)),
            ("/payroll-runs", Some(Payroll)),
            ("/payroll-runs/abc", Some(Payroll)),
            ("/employees", Some(Payroll)),
            ("/", None),
            ("/companies", None),
            ("/companies/abc", None),
            ("/settings/passkeys", None),
            ("/admin/invitations", None),
            // A longer word that only starts the same is another page.
            ("/suppliers-old", None),
        ] {
            assert_eq!(section_of(path), section, "{path}");
        }
    }

    #[test]
    fn initials_come_from_the_first_two_words() {
        assert_eq!(initials("Erik Berg"), "EB");
        assert_eq!(initials("anna"), "A");
        assert_eq!(initials("  Åsa   von Ö  "), "ÅV");
        assert_eq!(initials(""), "");
    }
}
