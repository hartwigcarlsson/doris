//! The rows of konteringar shared by Ny verifikation and Ingående balanser:
//! Konto, Debet and Kredit per row, a running total, and the request lines.

use crate::api::lpb;
use crate::format::{amount, parse_amount};
use crate::ui::{Button, TextInput, Variant};
use leptos::prelude::*;

/// The account number at the start of what was typed or picked from the
/// list ("1930 Företagskonto" → 1930). 0 when there is none; the server
/// refuses it as an unknown account.
pub fn account_number(raw: &str) -> u32 {
    raw.split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// An empty amount field is 0; anything else must parse.
fn field_amount(raw: &str) -> Option<i64> {
    if raw.trim().is_empty() {
        Some(0)
    } else {
        parse_amount(raw)
    }
}

/// A saved amount as it goes back into a field: empty for 0, else
/// `1 234,50`, which `parse_amount` reads back exactly.
fn side(ore: i64) -> String {
    if ore == 0 { String::new() } else { amount(ore) }
}

#[derive(Clone, Copy)]
struct Line {
    id: usize,
    account: RwSignal<String>,
    debit: RwSignal<String>,
    credit: RwSignal<String>,
}

impl Line {
    fn is_blank(&self) -> bool {
        [self.account, self.debit, self.credit]
            .iter()
            .all(|s| s.get_untracked().trim().is_empty())
    }
}

/// The editable rows. `Copy`, so pages can hand it to closures freely.
#[derive(Clone, Copy)]
pub struct Lines {
    next_id: StoredValue<usize>,
    rows: RwSignal<Vec<Line>>,
}

impl Default for Lines {
    fn default() -> Self {
        Self::new()
    }
}

impl Lines {
    /// Two empty rows.
    pub fn new() -> Self {
        let lines = Self {
            next_id: StoredValue::new(0),
            rows: RwSignal::new(Vec::new()),
        };
        lines.clear();
        lines
    }

    /// Back to two empty rows. New ids, so the view rebuilds every row.
    pub fn clear(&self) {
        self.fill(&[]);
    }

    /// One row per saved line, topped up to at least two rows.
    pub fn fill(&self, saved: &[lpb::VoucherLine]) {
        let mut rows: Vec<Line> = saved
            .iter()
            .map(|l| self.line(l.account.to_string(), side(l.debit), side(l.credit)))
            .collect();
        while rows.len() < 2 {
            rows.push(self.line(String::new(), String::new(), String::new()));
        }
        self.rows.set(rows);
    }

    fn line(&self, account: String, debit: String, credit: String) -> Line {
        let id = self.next_id.get_value();
        self.next_id.set_value(id + 1);
        Line {
            id,
            account: RwSignal::new(account),
            debit: RwSignal::new(debit),
            credit: RwSignal::new(credit),
        }
    }

    fn add(&self) {
        let line = self.line(String::new(), String::new(), String::new());
        self.rows.update(|rows| rows.push(line));
    }

    /// Debit and credit totals of what is typed; unreadable amounts count
    /// as 0.
    fn totals(&self) -> (i64, i64) {
        self.rows.with(|rows| {
            rows.iter().fold((0_i64, 0_i64), |(d, c), line| {
                (
                    d + line.debit.with(|s| field_amount(s)).unwrap_or(0),
                    c + line.credit.with(|s| field_amount(s)).unwrap_or(0),
                )
            })
        })
    }

    /// The non-blank rows as request lines, or `None` if an amount can't be
    /// read.
    pub fn request(&self) -> Option<Vec<lpb::VoucherLine>> {
        self.rows
            .get_untracked()
            .iter()
            .filter(|l| !l.is_blank())
            .map(|line| {
                Some(lpb::VoucherLine {
                    account: account_number(&line.account.get_untracked()),
                    debit: field_amount(&line.debit.get_untracked())?,
                    credit: field_amount(&line.credit.get_untracked())?,
                })
            })
            .collect()
    }
}

/// The Konto, Debet and Kredit inputs of every row ("Konto, rad 1" …) with
/// "Ta bort", then "Lägg till rad" and the running "Debet · Kredit ·
/// Differens". `list` is the id of the page's account `<datalist>`.
#[component]
pub fn LineRows(lines: Lines, list: &'static str) -> impl IntoView {
    view! {
        <div class="grid gap-2">
            <div class="grid grid-cols-[1fr_8rem_8rem_auto] gap-2 text-muted-foreground">
                <span>"Konto"</span>
                <span>"Debet"</span>
                <span>"Kredit"</span>
                <span></span>
            </div>
            <For each=move || { lines.rows.get().into_iter().enumerate().collect::<Vec<_>>() } key=|(i, l)| (*i, l.id) let((index, line))>
                <div class="grid grid-cols-[1fr_8rem_8rem_auto] gap-2">
                    <TextInput label=format!("Konto, rad {}", index + 1) value=line.account list=list />
                    <TextInput label=format!("Debet, rad {}", index + 1) value=line.debit inputmode="decimal" />
                    <TextInput label=format!("Kredit, rad {}", index + 1) value=line.credit inputmode="decimal" />
                    <Button
                        variant=Variant::Ghost
                        kind="button"
                        on:click=move |_| lines.rows.update(|rows| rows.retain(|other| other.id != line.id))
                    >
                        "Ta bort"
                    </Button>
                </div>
            </For>
            <div>
                <Button variant=Variant::Ghost kind="button" on:click=move |_| lines.add()>"Lägg till rad"</Button>
            </div>
        </div>
        <p class="text-xs/relaxed text-muted-foreground">
            {move || {
                let (debit, credit) = lines.totals();
                format!("Debet {} · Kredit {} · Differens {}", amount(debit), amount(credit), amount(debit - credit))
            }}
        </p>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::parse_amount;

    #[test]
    fn account_number_takes_the_leading_digits() {
        assert_eq!(account_number("1930"), 1930);
        assert_eq!(
            account_number(" 1930 Företagskonto/checkkonto/affärskonto"),
            1930
        );
        assert_eq!(account_number("Företagskonto"), 0);
        assert_eq!(account_number(""), 0);
        assert_eq!(account_number("19x0"), 0);
    }

    #[test]
    fn an_empty_amount_field_is_zero_and_junk_is_unreadable() {
        assert_eq!(field_amount(" "), Some(0));
        assert_eq!(field_amount("1 250,50"), Some(125_050));
        assert_eq!(field_amount("1oo"), None);
    }

    #[test]
    fn saved_amounts_read_back_as_typed() {
        assert_eq!(side(0), "");
        for ore in [1, 125_050, 100_000_000_000] {
            assert_eq!(parse_amount(&side(ore)), Some(ore), "{ore}");
        }
    }
}
