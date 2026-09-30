//! The active company: the one the user is keeping the books for right now.
//! Chosen in the header and remembered per user in this browser's
//! localStorage. It grants nothing: every company RPC still checks
//! membership on the server.

use crate::api::cpb;

/// Which company is active: `preferred` (the current or remembered choice)
/// if the user still has it, otherwise the first one listed.
#[allow(dead_code)] // used from Task 2
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

#[allow(dead_code)] // used from Task 2
fn remembered(user_id: &str) -> Option<String> {
    storage()?.get_item(&storage_key(user_id)).ok().flatten()
}

#[allow(dead_code)] // used from Task 2
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
