//! Which vouchers the list shows for what is typed and ticked. Pure: the
//! page already holds the year's vouchers and the chart of accounts.

// Used by Verifikationer from the next commits on.
#![allow(dead_code)]

use crate::api::lpb;
use crate::format::parse_amount;

/// Rows shown before "Visa fler", and how many more each click adds.
pub const PAGE: usize = 50;

/// What is typed in the search field and ticked next to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub query: String,
    pub missing_attachment: bool,
    pub corrections: bool,
}

/// Whether `word` (lower case, no spaces) is found in the voucher: as its
/// number (exactly), an account on it (by prefix), a whole amount on it,
/// or a part of its text or of an account's name.
fn word_matches(word: &str, voucher: &lpb::Voucher, accounts: &[lpb::Account]) -> bool {
    if word.bytes().all(|b| b.is_ascii_digit()) {
        if word.parse() == Ok(voucher.number) {
            return true;
        }
        if voucher
            .lines
            .iter()
            .any(|l| l.account.to_string().starts_with(word))
        {
            return true;
        }
    }
    // "1250", "1250,00" and "1250.0" are amounts; the öre must agree too.
    // Not 0: every line has an empty side.
    if let Some(ore) = parse_amount(word).filter(|ore| *ore != 0) {
        let total: i64 = voucher.lines.iter().map(|l| l.debit).sum();
        if ore == total
            || voucher
                .lines
                .iter()
                .any(|l| ore == l.debit || ore == l.credit)
        {
            return true;
        }
    }
    if voucher.text.to_lowercase().contains(word) {
        return true;
    }
    voucher.lines.iter().any(|line| {
        accounts
            .iter()
            .find(|a| a.number == line.account)
            .is_some_and(|a| a.name.to_lowercase().contains(word))
    })
}

/// Whether the list shows `voucher` under `filter`. Every word of the
/// search and every ticked box has to hold.
pub fn visible(voucher: &lpb::Voucher, accounts: &[lpb::Account], filter: &Filter) -> bool {
    if filter.missing_attachment && !voucher.attachments.is_empty() {
        return false;
    }
    if filter.corrections && voucher.corrects == 0 && voucher.corrected_by == 0 {
        return false;
    }
    filter
        .query
        .to_lowercase()
        .split_whitespace()
        .all(|word| word_matches(word, voucher, accounts))
}

/// How many of `total` matching rows are shown under `limit`.
pub fn shown(total: usize, limit: usize) -> usize {
    total.min(limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voucher(number: u32, text: &str, lines: &[(u32, i64, i64)]) -> lpb::Voucher {
        lpb::Voucher {
            number,
            text: text.into(),
            lines: lines
                .iter()
                .map(|&(account, debit, credit)| lpb::VoucherLine {
                    account,
                    debit,
                    credit,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn accounts() -> Vec<lpb::Account> {
        [
            (1930, "Företagskonto"),
            (3001, "Försäljning inom Sverige"),
            (5010, "Lokalhyra"),
        ]
        .into_iter()
        .map(|(number, name)| lpb::Account {
            number,
            name: name.into(),
            active: true,
        })
        .collect()
    }

    fn sale() -> lpb::Voucher {
        voucher(
            21,
            "Försäljning Nordvik Bygg",
            &[(1930, 1_250_00, 0), (3001, 0, 1_250_00)],
        )
    }

    fn found(voucher: &lpb::Voucher, query: &str) -> bool {
        visible(
            voucher,
            &accounts(),
            &Filter {
                query: query.into(),
                ..Default::default()
            },
        )
    }

    #[test]
    fn an_empty_search_matches_everything() {
        assert!(found(&sale(), ""));
        assert!(found(&sale(), "   "));
    }

    #[test]
    fn text_matches_anywhere_and_ignores_case() {
        assert!(found(&sale(), "nordvik"));
        assert!(found(&sale(), "SÄLJ"));
        assert!(!found(&sale(), "hyra"));
    }

    #[test]
    fn every_word_must_match_something() {
        assert!(found(&sale(), "nordvik 1930"));
        assert!(found(&sale(), "bygg försäljning"));
        assert!(!found(&sale(), "nordvik hyra"));
    }

    #[test]
    fn a_number_matches_the_voucher_number_exactly() {
        assert!(found(&sale(), "21"));
        assert!(!found(&voucher(210, "x", &[]), "21"));
        assert!(!found(&voucher(121, "x", &[]), "21"));
    }

    #[test]
    fn a_number_matches_accounts_that_start_with_it() {
        assert!(found(&sale(), "1930"));
        assert!(found(&sale(), "19"));
        assert!(found(&sale(), "3"));
        assert!(!found(&sale(), "930"));
        assert!(!found(&sale(), "5010"));
    }

    #[test]
    fn an_account_name_visible() {
        assert!(found(&sale(), "företagskonto"));
        assert!(found(&sale(), "inom sverige"));
        // 5010 Lokalhyra is in the chart but not on this voucher.
        assert!(!found(&sale(), "lokalhyra"));
        // An account missing from the chart has no name to match, and no panic.
        assert!(!found(
            &voucher(1, "x", &[(9999, 1_00, 0)]),
            "företagskonto"
        ));
    }

    #[test]
    fn an_amount_matches_whole_amounts_only() {
        assert!(found(&sale(), "1250"));
        assert!(found(&sale(), "1250,00"));
        assert!(found(&sale(), "1250.00"));
        assert!(found(&sale(), "1250,0"));
        assert!(!found(&sale(), "125"));
        assert!(!found(&sale(), "1250,01"));
        assert!(!found(&sale(), "250"));
        let small = voucher(7, "Bankavgift", &[(6570, 12_50, 0), (1930, 0, 12_50)]);
        assert!(found(&small, "12,50"));
        assert!(found(&small, "12,5"));
        assert!(!found(&small, "12"));
    }

    #[test]
    fn an_amount_typed_with_a_space_is_two_words() {
        // "1 250,00" asks for "1" and "250,00": neither is this voucher's
        // amount, and "1" is not its number. Documented, not clever.
        assert!(!found(&sale(), "1 250,00"));
        // The same words do match a voucher that has both.
        let both = voucher(1, "x", &[(1930, 250_00, 0), (3001, 0, 250_00)]);
        assert!(found(&both, "1 250,00"));
    }

    #[test]
    fn odd_characters_are_just_characters() {
        for query in ["(", "*", ".*", ",", ".", "å", "\\", "1,2,3", "--", "💰"] {
            // No panic, and nothing in the sale contains these.
            assert!(!found(&sale(), query), "{query}");
        }
        assert!(found(&voucher(1, "Hyra (mars)", &[]), "(mars)"));
    }

    #[test]
    fn missing_attachment_keeps_vouchers_without_underlag() {
        let filter = Filter {
            missing_attachment: true,
            ..Default::default()
        };
        let mut with = sale();
        with.attachments.push(lpb::Attachment::default());
        assert!(visible(&sale(), &accounts(), &filter));
        assert!(!visible(&with, &accounts(), &filter));
    }

    #[test]
    fn corrections_keeps_both_the_corrected_and_the_correction() {
        let filter = Filter {
            corrections: true,
            ..Default::default()
        };
        let (mut corrected, mut correction) = (sale(), sale());
        corrected.corrected_by = 22;
        correction.corrects = 21;
        assert!(visible(&corrected, &accounts(), &filter));
        assert!(visible(&correction, &accounts(), &filter));
        assert!(!visible(&sale(), &accounts(), &filter));
    }

    #[test]
    fn filters_and_search_all_have_to_hold() {
        let mut correction = sale();
        correction.corrects = 20;
        let filter = |query: &str| Filter {
            query: query.into(),
            missing_attachment: true,
            corrections: true,
        };
        assert!(visible(&correction, &accounts(), &filter("nordvik")));
        assert!(!visible(&correction, &accounts(), &filter("hyra")));
        assert!(!visible(&sale(), &accounts(), &filter("nordvik")));
    }

    #[test]
    fn a_page_shows_the_limit_or_what_there_is() {
        assert_eq!(shown(0, PAGE), 0);
        assert_eq!(shown(49, PAGE), 49);
        assert_eq!(shown(50, PAGE), 50);
        assert_eq!(shown(51, PAGE), 50);
        assert_eq!(shown(51, 2 * PAGE), 51);
    }

    #[test]
    fn zero_is_not_an_amount_to_search_for() {
        assert!(!found(&sale(), "0"));
        assert!(!found(&sale(), "0,00"));
    }
}
