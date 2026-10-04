use doris_payroll::agi::*;
use doris_payroll::domain::DomainError;
use jiff::civil::date;

#[test]
fn a_period_is_six_digits_year_and_month() {
    let p = Period::parse(" 202610 ").unwrap();
    assert_eq!(p.get(), 202610);
    assert_eq!(p.to_string(), "202610");
    assert_eq!(p.first_day(), date(2026, 10, 1));
    assert_eq!(Period::of(date(2026, 10, 25)), p);
    assert!(Period::parse("202612").is_ok());
    for bad in [
        "", "202613", "202600", "2026-10", "26010", "2026100", "18991", "189912", "2026１0",
    ] {
        assert_eq!(
            Period::parse(bad),
            Err(DomainError::InvalidPeriod),
            "{bad:?}"
        );
    }
    assert!(Period::parse("202609").unwrap() < p);
}

#[test]
fn a_contact_follows_skatteverkets_schema() {
    let ok = AgiContact::parse(" Anna Andersson ", " 070-123 45 67 ", " anna@example.se ").unwrap();
    assert_eq!(
        ok,
        AgiContact {
            name: "Anna Andersson".into(),
            phone: "070-123 45 67".into(),
            email: "anna@example.se".into()
        }
    );
    assert!(
        AgiContact::parse(
            &"å".repeat(50),
            &"1".repeat(20),
            "a.b-c+d'e_f@x-y.example.se"
        )
        .is_ok()
    );
    let refused = |n: &str, p: &str, e: &str| {
        AgiContact::parse(n, p, e) == Err(DomainError::InvalidAgiContact)
    };
    assert!(refused(&"å".repeat(51), "070", "a@b.se"), "name too long");
    assert!(refused("Anna", &"1".repeat(21), "a@b.se"), "phone too long");
    assert!(refused("", "070", "a@b.se"), "no name");
    assert!(refused("Anna", "   ", "a@b.se"), "blank phone");
    assert!(refused("Anna <AB>", "070", "a@b.se"), "angle brackets");
    assert!(
        refused("Anna\u{1}", "070", "a@b.se"),
        "control character in name"
    );
    assert!(
        refused("Anna", "070\t12", "a@b.se"),
        "control character in phone"
    );
    for email in [
        "",
        "a@b",
        "a@b.",
        "@b.se",
        "a@.se",
        "a..b@c.se",
        "a@b..se",
        "a b@c.se",
        "a@b@c.se",
        "å@b.se",
        "a@b.s",
        "ab.se",
    ] {
        // "a@b.s" is valid by the pattern; only the length rule (5) refuses "a@b." etc.
        if email == "a@b.s" {
            assert!(AgiContact::parse("Anna", "070", email).is_ok(), "{email:?}");
        } else {
            assert!(refused("Anna", "070", email), "{email:?}");
        }
    }
}

use doris_payroll::domain::*;
use doris_payroll::tax::TaxBasis;
use jiff::civil::Date;
use std::collections::HashSet;
use uuid::Uuid;

const KR: i64 = 100;

/// A company's payroll events, plus the corrected (reversed) vouchers.
#[derive(Default)]
struct World {
    events: Vec<PayrollEvent>,
    reversed: HashSet<BookedVoucher>,
    vouchers: u32,
}

impl World {
    fn payroll(&self) -> Payroll {
        Payroll::from_events(&self.events, self.reversed.clone())
    }

    fn hire(&mut self, name: &str, personnummer: &str) -> Uuid {
        let id = Uuid::new_v4();
        self.events.push(PayrollEvent::EmployeeAdded {
            employee_id: id,
            name: EmployeeName::parse(name).unwrap(),
            personal_identity_number: PersonalIdentityNumber::parse(personnummer).unwrap(),
            monthly_salary: 1,
            salary_account: SalaryAccount::DEFAULT,
        });
        id
    }

    /// A run finalized with `lines` (employee, gross öre, tax öre; fee 1 öre
    /// each) and booked; returns its voucher.
    fn booked(&mut self, pay_date: Date, lines: &[(Uuid, i64, i64)]) -> BookedVoucher {
        let run = Uuid::new_v4();
        let draft = PayrollRunDraft {
            pay_date,
            text: "Lön".into(),
            lines: lines
                .iter()
                .map(|&(employee_id, gross, tax)| DraftLine {
                    employee_id,
                    gross,
                    tax: Some(tax),
                })
                .collect(),
        };
        let locked = lines
            .iter()
            .map(|&(employee_id, gross, tax)| PayrollRunLine {
                employee_id,
                salary_account: SalaryAccount::DEFAULT,
                gross,
                tax,
                fee_rate: FULL_RATE,
                fee: 1,
                net: gross - tax,
                tax_basis: TaxBasis::Manual,
            })
            .collect();
        self.vouchers += 1;
        let voucher = BookedVoucher {
            fiscal_year_start: date(2026, 1, 1),
            number: self.vouchers,
        };
        self.events.push(PayrollEvent::PayrollRunCreated {
            payroll_run_id: run,
            draft,
        });
        self.events.push(PayrollEvent::PayrollRunFinalized {
            payroll_run_id: run,
            lines: locked,
        });
        self.events.push(PayrollEvent::PayrollRunBooked {
            payroll_run_id: run,
            voucher,
        });
        voucher
    }

    fn contact(&mut self) {
        let contact =
            AgiContact::parse("Anna Andersson", "070-123 45 67", "anna@example.se").unwrap();
        self.events
            .extend(set_agi_contact(&self.payroll(), contact));
    }

    fn submit(&mut self, period: Period) {
        let event = submit_agi_month(&self.payroll(), period).unwrap();
        self.events.push(event);
    }
}

fn oct() -> Period {
    Period::parse("202610").unwrap()
}

fn amounts(month: &AgiMonth) -> Vec<(u64, i64, i64, AgiChange)> {
    month
        .lines
        .iter()
        .map(|(l, c)| (l.specification_number, l.gross, l.tax, *c))
        .collect()
}

#[test]
fn only_booked_runs_in_the_month_count_summed_per_employee() {
    let mut w = World::default();
    let asa = w.hire("Åsa Öberg", "19800101-1231");
    let bo = w.hire("Bo Ek", "19500301-1235");
    w.booked(
        date(2026, 10, 25),
        &[
            (asa, 20_000 * KR + 50, 4_000 * KR),
            (bo, 20_000 * KR, 3_000 * KR),
        ],
    );
    w.booked(date(2026, 10, 31), &[(asa, 15_000 * KR + 50, 3_134 * KR)]);
    w.booked(date(2026, 11, 25), &[(asa, 99_999 * KR, 0)]);
    let reversed = w.booked(date(2026, 10, 28), &[(bo, 99_999 * KR, 0)]);
    w.reversed.insert(reversed);
    // Open and finalized-only runs don't count either.
    w.events.push(PayrollEvent::PayrollRunCreated {
        payroll_run_id: Uuid::new_v4(),
        draft: PayrollRunDraft {
            pay_date: date(2026, 10, 20),
            text: "Öppen".into(),
            lines: vec![DraftLine {
                employee_id: bo,
                gross: 99_999 * KR,
                tax: Some(0),
            }],
        },
    });

    let lines = agi_lines(&w.payroll(), oct());

    // Öre are summed first, then dropped: 20 000,50 + 15 000,50 = 35 001 kr.
    let got: Vec<_> = lines
        .iter()
        .map(|l| (l.employee_id, l.gross, l.tax))
        .collect();
    assert_eq!(got.len(), 2);
    assert!(got.contains(&(asa, 35_001, 7_134)));
    assert!(got.contains(&(bo, 20_000, 3_000)));
}

#[test]
fn the_fee_sum_follows_skatteverkets_calculation() {
    let mut w = World::default();
    let asa = w.hire("Åsa Öberg", "19800101-1231");
    let bo = w.hire("Bo Ek", "19500301-1235");
    let ung = w.hire("Unga Ung", "20050615-1232");
    w.booked(
        date(2026, 10, 25),
        &[(asa, 35_000 * KR, 0), (bo, 20_000 * KR, 0)],
    );
    // 35 000 × 31,42 % + 20 000 × 10,21 % = 10 997 + 2 042.
    let p = w.payroll();
    assert_eq!(agi_fee_sum(&p, oct(), &agi_lines(&p, oct())), 13_039);

    // Two runs for the young employee the same month: one IU of 40 000 kr,
    // 25 000 × 20,81 % + 15 000 × 31,42 % = 9 915,5 → 9 915.
    w.booked(date(2026, 11, 10), &[(ung, 20_000 * KR, 0)]);
    w.booked(date(2026, 11, 25), &[(ung, 20_000 * KR, 0)]);
    let p = w.payroll();
    let nov = Period::parse("202611").unwrap();
    assert_eq!(agi_fee_sum(&p, nov, &agi_lines(&p, nov)), 9_915);

    // Before the reduction (March 2026): full rate, 30 000 × 31,42 %.
    w.booked(date(2026, 3, 25), &[(ung, 30_000 * KR, 0)]);
    let p = w.payroll();
    let mar = Period::parse("202603").unwrap();
    assert_eq!(agi_fee_sum(&p, mar, &agi_lines(&p, mar)), 9_426);
}

#[test]
fn a_month_goes_not_submitted_submitted_changed() {
    let mut w = World::default();
    let asa = w.hire("Åsa Öberg", "19800101-1231");
    let bo = w.hire("Bo Ek", "19500301-1235");
    w.contact();
    let first = w.booked(
        date(2026, 10, 25),
        &[
            (asa, 35_000 * KR, 7_134 * KR),
            (bo, 20_000 * KR, 3_000 * KR),
        ],
    );
    let month = agi_month(&w.payroll(), oct());
    assert_eq!(month.status, AgiStatus::NotSubmitted);
    // Specification numbers are the position in the register: Åsa, then Bo.
    assert_eq!(
        amounts(&month),
        [
            (1, 35_000, 7_134, AgiChange::New),
            (2, 20_000, 3_000, AgiChange::New)
        ]
    );
    assert_eq!(
        (month.fee_sum, month.tax_sum, month.booked_fees),
        (13_039, 10_134, 2)
    );

    w.submit(oct());
    let month = agi_month(&w.payroll(), oct());
    assert_eq!(month.status, AgiStatus::Submitted);
    assert!(month.lines.iter().all(|(_, c)| *c == AgiChange::Unchanged));
    assert_eq!(
        submit_agi_month(&w.payroll(), oct()),
        Err(DomainError::AgiUnchanged)
    );

    // The run is backed out and Åsa alone is paid again: Åsa changes,
    // Bo is removed with his number.
    w.reversed.insert(first);
    w.booked(date(2026, 10, 30), &[(asa, 36_000 * KR, 7_400 * KR)]);
    let month = agi_month(&w.payroll(), oct());
    assert_eq!(month.status, AgiStatus::Changed);
    assert_eq!(amounts(&month), [(1, 36_000, 7_400, AgiChange::Changed)]);
    assert_eq!(
        month
            .removed
            .iter()
            .map(|l| (l.employee_id, l.specification_number))
            .collect::<Vec<_>>(),
        [(bo, 2)]
    );
    w.submit(oct());
    assert_eq!(agi_month(&w.payroll(), oct()).status, AgiStatus::Submitted);
}

#[test]
fn a_month_with_everything_backed_out_can_be_declared_empty() {
    let mut w = World::default();
    let asa = w.hire("Åsa Öberg", "19800101-1231");
    w.contact();
    let run = w.booked(date(2026, 10, 25), &[(asa, 35_000 * KR, 7_134 * KR)]);
    w.submit(oct());
    w.reversed.insert(run);

    let month = agi_month(&w.payroll(), oct());

    assert_eq!(month.status, AgiStatus::Changed);
    assert!(month.lines.is_empty());
    assert_eq!(month.removed.len(), 1);
    assert_eq!((month.fee_sum, month.tax_sum), (0, 0));
    assert!(agi_periods(&w.payroll()).contains(&oct()));
    w.submit(oct());
    assert_eq!(agi_month(&w.payroll(), oct()).status, AgiStatus::Submitted);
}

#[test]
fn an_employee_in_two_runs_one_backed_out_is_changed_not_removed() {
    let mut w = World::default();
    let asa = w.hire("Åsa Öberg", "19800101-1231");
    w.contact();
    w.booked(date(2026, 10, 10), &[(asa, 20_000 * KR, 4_000 * KR)]);
    let second = w.booked(date(2026, 10, 25), &[(asa, 15_000 * KR, 3_134 * KR)]);
    w.submit(oct());
    w.reversed.insert(second);

    let month = agi_month(&w.payroll(), oct());

    assert_eq!(amounts(&month), [(1, 20_000, 4_000, AgiChange::Changed)]);
    assert!(month.removed.is_empty());
}

#[test]
fn specification_numbers_are_the_position_in_the_register() {
    let mut w = World::default();
    let asa = w.hire("Åsa Öberg", "19800101-1231");
    let bo = w.hire("Bo Ek", "19500301-1235");
    w.contact();
    w.booked(date(2026, 10, 25), &[(asa, 100 * KR, 0), (bo, 100 * KR, 0)]);
    w.submit(oct());
    let cy = w.hire("Cy Al", "19800102-1230");
    w.booked(date(2026, 11, 25), &[(asa, 100 * KR, 0), (cy, 100 * KR, 0)]);

    let nov = agi_month(&w.payroll(), Period::parse("202611").unwrap());

    let numbers: Vec<_> = nov
        .lines
        .iter()
        .map(|(l, _)| (l.employee_id, l.specification_number))
        .collect();
    assert_eq!(numbers, [(asa, 1), (cy, 3)]);
}

#[test]
fn numbers_do_not_move_when_an_earlier_month_is_marked() {
    let mut w = World::default();
    let asa = w.hire("Åsa Öberg", "19800101-1231");
    let bo = w.hire("Bo Ek", "19500301-1235");
    w.contact();
    let sep = Period::parse("202609").unwrap();
    w.booked(date(2026, 9, 25), &[(bo, 100 * KR, 0)]);
    w.booked(date(2026, 10, 25), &[(asa, 100 * KR, 0), (bo, 100 * KR, 0)]);
    let numbers = |w: &World, p: Period| {
        agi_lines(&w.payroll(), p)
            .iter()
            .map(|l| (l.employee_id, l.specification_number))
            .collect::<Vec<_>>()
    };
    let sep_before = numbers(&w, sep);
    let oct_before = numbers(&w, oct());

    w.submit(sep);

    assert_eq!(numbers(&w, sep), sep_before);
    assert_eq!(numbers(&w, oct()), oct_before);
    // The position in the register: Åsa was hired first.
    assert_eq!(oct_before, [(asa, 1), (bo, 2)]);
}

#[test]
fn submitting_needs_something_and_a_contact() {
    let mut w = World::default();
    let asa = w.hire("Åsa Öberg", "19800101-1231");
    assert_eq!(
        submit_agi_month(&w.payroll(), oct()),
        Err(DomainError::AgiPeriodEmpty)
    );
    w.booked(date(2026, 10, 25), &[(asa, 100 * KR, 0)]);
    assert_eq!(
        submit_agi_month(&w.payroll(), oct()),
        Err(DomainError::AgiContactMissing)
    );
    w.contact();
    assert!(matches!(
        submit_agi_month(&w.payroll(), oct()),
        Ok(PayrollEvent::AgiMonthSubmitted { .. })
    ));
}

#[test]
fn the_contact_is_set_once_and_changed() {
    let mut w = World::default();
    w.contact();
    assert_eq!(
        w.payroll().agi_contact.as_ref().unwrap().phone,
        "070-123 45 67"
    );
    let same = AgiContact::parse("Anna Andersson", "070-123 45 67", "anna@example.se").unwrap();
    assert!(set_agi_contact(&w.payroll(), same).is_empty());
    let other = AgiContact::parse("Bo Ek", "08-123", "bo@example.se").unwrap();
    assert_eq!(
        set_agi_contact(&w.payroll(), other.clone()),
        [PayrollEvent::AgiContactChanged { contact: other }]
    );
}

#[test]
fn periods_list_booked_months_and_submissions_newest_first() {
    let mut w = World::default();
    let asa = w.hire("Åsa Öberg", "19800101-1231");
    w.booked(date(2026, 9, 25), &[(asa, 100 * KR, 0)]);
    w.booked(date(2026, 11, 25), &[(asa, 100 * KR, 0)]);
    let periods: Vec<_> = agi_periods(&w.payroll()).iter().map(|p| p.get()).collect();
    assert_eq!(periods, [202611, 202609]);
}

#[test]
fn a_removed_employee_keeps_her_number_for_later_months() {
    let mut w = World::default();
    let asa = w.hire("Åsa Öberg", "19800101-1231");
    let bo = w.hire("Bo Ek", "19500301-1235");
    w.contact();
    let first = w.booked(date(2026, 10, 25), &[(asa, 100 * KR, 0), (bo, 100 * KR, 0)]);
    w.submit(oct());
    w.reversed.insert(first);
    w.booked(date(2026, 10, 30), &[(asa, 100 * KR, 0)]);
    w.submit(oct());
    w.booked(date(2026, 11, 25), &[(bo, 100 * KR, 0)]);

    let nov = agi_month(&w.payroll(), Period::parse("202611").unwrap());

    assert_eq!(
        nov.lines
            .iter()
            .map(|(l, _)| l.specification_number)
            .collect::<Vec<_>>(),
        [2]
    );
}

#[test]
fn a_new_employee_never_takes_a_removed_employees_number() {
    let mut w = World::default();
    let asa = w.hire("Åsa Öberg", "19800101-1231");
    let bo = w.hire("Bo Ek", "19500301-1235");
    w.contact();
    let first = w.booked(date(2026, 10, 25), &[(bo, 100 * KR, 0), (asa, 100 * KR, 0)]);
    w.submit(oct());
    w.reversed.insert(first);
    w.booked(date(2026, 10, 30), &[(bo, 100 * KR, 0)]);
    w.submit(oct());
    let cy = w.hire("Cy Al", "19800102-1230");
    w.booked(date(2026, 11, 25), &[(cy, 100 * KR, 0)]);

    let nov = agi_month(&w.payroll(), Period::parse("202611").unwrap());

    assert_eq!(
        nov.lines
            .iter()
            .map(|(l, _)| l.specification_number)
            .collect::<Vec<_>>(),
        [3]
    );
}

#[test]
fn an_employee_paid_under_one_krona_takes_no_number() {
    let mut w = World::default();
    let asa = w.hire("Ada Ek", "19800101-1231");
    let bo = w.hire("Bo Ek", "19500301-1235");
    w.booked(date(2026, 10, 25), &[(asa, 50, 0), (bo, 100 * KR, 0)]);

    let month = agi_month(&w.payroll(), oct());

    // Ada keeps her number 1 in the register, but has no line.
    assert_eq!(amounts(&month), [(2, 100, 0, AgiChange::New)]);
}

use doris_company::domain::OrgNr;
use std::collections::HashMap;

fn line(employee_id: Uuid, number: u64, gross: i64, tax: i64) -> AgiLine {
    AgiLine {
        employee_id,
        specification_number: number,
        gross,
        tax,
    }
}

fn contact() -> AgiContact {
    AgiContact::parse("Anna Andersson", "070-123 45 67", "anna@example.se").unwrap()
}

fn created() -> jiff::civil::DateTime {
    jiff::civil::date(2026, 11, 5).at(9, 30, 0, 0)
}

#[test]
fn the_employer_id_has_twelve_digits() {
    assert_eq!(
        employer_id(&OrgNr::parse("556016-0680").unwrap(), 2026),
        "165560160680"
    );
    // Enskild firma: the owner's personnummer, its century from the year.
    assert_eq!(
        employer_id(&OrgNr::parse("800101-1231").unwrap(), 2026),
        "198001011231"
    );
    assert_eq!(
        employer_id(&OrgNr::parse("050615-1232").unwrap(), 2026),
        "200506151232"
    );
}

#[test]
fn a_month_not_yet_submitted_writes_the_hu_and_every_iu() {
    let (asa, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let month = AgiMonth {
        period: oct(),
        lines: vec![
            (line(asa, 1, 35_000, 7_134), AgiChange::New),
            (line(bo, 2, 20_000, 3_000), AgiChange::New),
        ],
        removed: vec![],
        fee_sum: 13_039,
        tax_sum: 10_134,
        booked_fees: 1_303_900,
        status: AgiStatus::NotSubmitted,
    };
    let ids = HashMap::from([
        (asa, "198001011231".to_owned()),
        (bo, "195003011235".to_owned()),
    ]);

    let xml = agi_xml(&month, "165560160680", &contact(), &ids, created());

    assert_eq!(xml, include_str!("fixtures/agi_202610.xml"));
}

#[test]
fn a_changed_month_writes_only_changes_and_removals() {
    let (asa, bo, cy) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let month = AgiMonth {
        period: oct(),
        lines: vec![
            (line(asa, 1, 35_000, 7_134), AgiChange::Unchanged),
            (line(cy, 3, 10_000, 2_000), AgiChange::Changed),
        ],
        removed: vec![line(bo, 2, 20_000, 3_000)],
        fee_sum: 14_139,
        tax_sum: 9_134,
        booked_fees: 0,
        status: AgiStatus::Changed,
    };
    let ids = HashMap::from([
        (asa, "198001011231".to_owned()),
        (bo, "195003011235".to_owned()),
        (cy, "198001021230".to_owned()),
    ]);

    let xml = agi_xml(&month, "165560160680", &contact(), &ids, created());

    assert!(xml.contains(r#"<agd:SummaArbAvgSlf faltkod="487">14139</agd:SummaArbAvgSlf>"#));
    assert!(xml.contains(r#"<agd:SummaSkatteavdr faltkod="497">9134</agd:SummaSkatteavdr>"#));
    assert!(
        !xml.contains("198001011231"),
        "unchanged lines are not sent"
    );
    assert!(xml.contains(
        r#"<agd:BetalningsmottagarId faltkod="215">198001021230</agd:BetalningsmottagarId>"#
    ));
    let removal = r#"<agd:BetalningsmottagarId faltkod="215">195003011235</agd:BetalningsmottagarId>
          </agd:BetalningsmottagareIDChoice>
        </agd:BetalningsmottagareIUGROUP>
        <agd:RedovisningsPeriod faltkod="006">202610</agd:RedovisningsPeriod>
        <agd:Specifikationsnummer faltkod="570">2</agd:Specifikationsnummer>
        <agd:Borttag faltkod="205">1</agd:Borttag>
      </agd:IU>"#;
    assert!(xml.contains(removal), "{xml}");
    assert_eq!(xml.matches("<agd:IU>").count(), 2);
}

#[test]
fn text_is_escaped() {
    let month = AgiMonth {
        period: oct(),
        lines: vec![],
        removed: vec![],
        fee_sum: 0,
        tax_sum: 0,
        booked_fees: 0,
        status: AgiStatus::Changed,
    };
    let contact = AgiContact {
        name: "Åsa & Bo <AB> \"x\" 'y'".into(),
        phone: "070".into(),
        email: "a@b.se".into(),
    };

    let xml = agi_xml(&month, "165560160680", &contact, &HashMap::new(), created());

    assert!(
        xml.contains("<agd:Namn>Åsa &amp; Bo &lt;AB&gt; &quot;x&quot; &apos;y&apos;</agd:Namn>")
    );
}
