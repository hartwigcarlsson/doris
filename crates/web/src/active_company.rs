//! The active company: the one the user is keeping the books for right now.
//! Chosen in the header and remembered per user in this browser's
//! localStorage. It grants nothing: every company RPC still checks
//! membership on the server.

use crate::api::{company_api, cpb};
use crate::ui::{SELECT_OPTION, Select};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

/// The signed-in user's companies and which one is active. Provided as
/// context by `App`, which loads it whenever the signed-in user changes.
#[derive(Clone, Copy)]
pub struct Companies {
    pub list: RwSignal<Vec<cpb::CompanySummary>>,
    /// The active company's id; empty when there is none.
    pub active: RwSignal<String>,
    /// False until the list has been fetched for the current user.
    pub loaded: RwSignal<bool>,
    user_id: RwSignal<Option<String>>,
}

impl Companies {
    pub fn new() -> Self {
        let companies = Self {
            list: RwSignal::new(Vec::new()),
            active: RwSignal::new(String::new()),
            loaded: RwSignal::new(false),
            user_id: RwSignal::new(None),
        };
        // Remember every choice, whoever made it: the header, a new company,
        // or the automatic fallback.
        Effect::new(move |_| {
            let active = companies.active.get();
            if let Some(user_id) = companies.user_id.get_untracked()
                && !active.is_empty()
            {
                remember(&user_id, &active);
            }
        });
        companies
    }

    /// Fetches `user_id`'s companies and settles the active one: the current
    /// choice, else the remembered one, if still listed; else the first.
    pub fn load(self, user_id: String) {
        if self.user_id.get_untracked().as_deref() != Some(user_id.as_str()) {
            self.clear();
            self.user_id.set(Some(user_id.clone()));
        }
        spawn_local(async move {
            let Ok(response) = company_api()
                .list_companies(cpb::ListCompaniesRequest {})
                .await
            else {
                return;
            };
            // Another user signed in while this was in flight.
            if self.user_id.get_untracked().as_deref() != Some(user_id.as_str()) {
                return;
            }
            let list = response.into_inner().companies;
            let current = self.active.get_untracked();
            let preferred = if current.is_empty() {
                remembered(&user_id)
            } else {
                Some(current)
            };
            self.active
                .set(resolve_active(preferred.as_deref(), &list).unwrap_or_default());
            self.list.set(list);
            self.loaded.set(true);
        });
    }

    /// Fetches the list again for the same user, e.g. after adding a company.
    pub fn reload(self) {
        if let Some(user_id) = self.user_id.get_untracked() {
            self.load(user_id);
        }
    }

    pub fn clear(self) {
        self.user_id.set(None);
        self.list.set(Vec::new());
        self.active.set(String::new());
        self.loaded.set(false);
    }

    pub fn active_company(&self) -> Option<cpb::CompanySummary> {
        let active = self.active.get();
        self.list
            .with(|list| list.iter().find(|c| c.id == active).cloned())
    }
}

/// The header's company switcher, or a link to add the first company.
#[component]
pub fn ActiveCompanySelect() -> impl IntoView {
    let companies = expect_context::<Companies>();
    move || {
        if !companies.loaded.get() {
            return ().into_any();
        }
        if companies.list.with(Vec::is_empty) {
            return view! {
                <A href="/companies/new" attr:class="text-muted-foreground hover:text-foreground">"Lägg till företag"</A>
            }
            .into_any();
        }
        view! {
            <div class="w-48">
                <Select label="Aktivt företag" id="active_company" hide_label=true value=companies.active>
                    {move || {
                        companies
                            .list
                            .get()
                            .into_iter()
                            .map(|c| view! { <option class=SELECT_OPTION value=c.id>{c.name}</option> })
                            .collect_view()
                    }}
                </Select>
            </div>
        }
        .into_any()
    }
}

/// Which company is active: `preferred` (the current or remembered choice)
/// if the user still has it, otherwise the first one listed.
pub fn resolve_active(
    preferred: Option<&str>,
    companies: &[cpb::CompanySummary],
) -> Option<String> {
    companies
        .iter()
        .find(|c| Some(c.id.as_str()) == preferred)
        .or(companies.first())
        .map(|c| c.id.clone())
}

fn storage_key(user_id: &str) -> String {
    format!("doris.active_company.{user_id}")
}

/// `None` in private mode or when storage is blocked: the choice then lives
/// only as long as the page.
fn storage() -> Option<web_sys::Storage> {
    leptos::prelude::window().local_storage().ok().flatten()
}

fn remembered(user_id: &str) -> Option<String> {
    storage()?.get_item(&storage_key(user_id)).ok().flatten()
}

fn remember(user_id: &str, company_id: &str) {
    if let Some(storage) = storage() {
        let _ = storage.set_item(&storage_key(user_id), company_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn company(id: &str) -> cpb::CompanySummary {
        cpb::CompanySummary {
            id: id.into(),
            org_nr: "556016-0680".into(),
            name: id.into(),
        }
    }

    #[test]
    fn a_stored_company_that_is_still_listed_stays_active() {
        let list = [company("a"), company("b")];
        assert_eq!(resolve_active(Some("b"), &list).as_deref(), Some("b"));
    }

    #[test]
    fn a_stored_company_that_is_no_longer_listed_falls_back_to_the_first() {
        let list = [company("a"), company("b")];
        assert_eq!(resolve_active(Some("gone"), &list).as_deref(), Some("a"));
    }

    #[test]
    fn without_a_stored_choice_the_first_company_is_active() {
        let list = [company("a"), company("b")];
        assert_eq!(resolve_active(None, &list).as_deref(), Some("a"));
        assert_eq!(resolve_active(Some(""), &list).as_deref(), Some("a"));
    }

    #[test]
    fn no_companies_means_no_active_company() {
        assert_eq!(resolve_active(Some("a"), &[]), None);
        assert_eq!(resolve_active(None, &[]), None);
    }

    #[test]
    fn the_storage_key_is_per_user() {
        assert_eq!(storage_key("u-1"), "doris.active_company.u-1");
    }
}
