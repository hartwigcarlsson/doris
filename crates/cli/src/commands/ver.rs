use super::Context;
use crate::amount::{display, kronor, parse_kronor};
use crate::output::{Failure, Output};
use clap::{Args, Subcommand};
use doris_proto::ledger::v1 as lpb;
use serde_json::{Value, json};
use std::path::PathBuf;

const FILE_LIMIT: usize = 10 << 20;
const TOTAL_LIMIT: usize = 20 << 20;

#[derive(Subcommand)]
pub enum VerAction {
    /// Räkenskapsårets verifikationer, nyast först.
    List {
        /// Räkenskapsår: ett år eller ett startdatum. Standard är året som innehåller idag.
        #[arg(long)]
        year: Option<String>,
    },
    /// Ett verifikat med konteringar och underlag.
    View {
        number: u32,
        #[arg(long)]
        year: Option<String>,
    },
    /// Bokför ett verifikat (med --dry-run: se vad som skulle bokföras).
    New(NewVoucher),
    /// Rätta ett verifikat med en rättelse som vänder varje kontering.
    Correct {
        number: u32,
        /// Rättelsens datum (ÅÅÅÅ-MM-DD).
        #[arg(long)]
        date: String,
        #[arg(long)]
        year: Option<String>,
    },
}

#[derive(Args)]
pub struct NewVoucher {
    /// Datum (ÅÅÅÅ-MM-DD); datumet avgör räkenskapsåret.
    #[arg(long)]
    date: Option<String>,
    /// Verifikatets text.
    #[arg(long)]
    text: Option<String>,
    /// KONTO=BELOPP i kronor; upprepa för varje debetrad.
    #[arg(long = "debit", value_name = "KONTO=BELOPP")]
    debits: Vec<String>,
    /// KONTO=BELOPP i kronor; upprepa för varje kreditrad.
    #[arg(long = "credit", value_name = "KONTO=BELOPP")]
    credits: Vec<String>,
    /// Ett underlag (PDF, JPEG, PNG); upprepa för fler.
    #[arg(long = "attach", value_name = "FIL")]
    attachments: Vec<PathBuf>,
    /// Hela verifikatet som JSON, från en fil eller - för stdin.
    #[arg(long, value_name = "FIL")]
    input: Option<String>,
    /// Ignoreras: datumet avgör räkenskapsåret.
    #[arg(long)]
    year: Option<String>,
}

/// A voucher as typed: debit and credit in öre per line.
#[derive(Debug, PartialEq)]
pub struct Voucher {
    pub(crate) date: String,
    pub(crate) text: String,
    pub(crate) lines: Vec<(u32, i64, i64)>,
    pub(crate) attachments: Vec<PathBuf>,
}

fn bad_line(raw: &str) -> Failure {
    Failure::usage(format!(
        "Ogiltig rad \"{raw}\": skriv KONTO=BELOPP, till exempel 6110=800 eller 2641=200,50."
    ))
}

/// `KONTO=BELOPP` → account and öre.
fn line(raw: &str) -> Result<(u32, i64), Failure> {
    let (account, amount) = raw.split_once('=').ok_or_else(|| bad_line(raw))?;
    if account.is_empty() || !account.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad_line(raw));
    }
    let account = account.parse().map_err(|_| bad_line(raw))?;
    Ok((account, parse_kronor(amount).ok_or_else(|| bad_line(raw))?))
}

fn voucher_from_flags(
    date: Option<String>,
    text: Option<String>,
    debits: &[String],
    credits: &[String],
    attachments: &[PathBuf],
) -> Result<Voucher, Failure> {
    let date = date.ok_or_else(|| Failure::usage("--date saknas."))?;
    let text = text.ok_or_else(|| Failure::usage("--text saknas."))?;
    if debits.is_empty() && credits.is_empty() {
        return Err(Failure::usage("Minst en --debit eller --credit krävs."));
    }
    let mut lines = Vec::new();
    for raw in debits {
        let (account, ore) = line(raw)?;
        lines.push((account, ore, 0));
    }
    for raw in credits {
        let (account, ore) = line(raw)?;
        lines.push((account, 0, ore));
    }
    Ok(Voucher {
        date,
        text,
        lines,
        attachments: attachments.to_vec(),
    })
}

/// A JSON amount: kronor as a string or a number, no sign, two decimals at most.
fn json_amount(value: Option<&Value>, field: &str) -> Result<i64, Failure> {
    let bad = || Failure::usage(format!("Ogiltigt belopp i \"{field}\"."));
    match value {
        None | Some(Value::Null) => Ok(0),
        Some(Value::String(s)) => parse_kronor(s).ok_or_else(bad),
        Some(Value::Number(n)) => parse_kronor(&n.to_string()).ok_or_else(bad),
        Some(_) => Err(bad()),
    }
}

fn voucher_from_json(raw: &str) -> Result<Voucher, Failure> {
    let value: Value =
        serde_json::from_str(raw).map_err(|e| Failure::usage(format!("Ogiltig --input: {e}.")))?;
    voucher_from_value(&value, "Ogiltig --input")
}

/// A voucher as JSON (`{"date","text","lines","attachments"}`); messages
/// start with `prefix` ("Ogiltig --input", "Ogiltigt argument").
pub(crate) fn voucher_from_value(value: &Value, prefix: &str) -> Result<Voucher, Failure> {
    let bad = |what: &str| Failure::usage(format!("{prefix}: {what}."));
    let field = |name: &str| {
        value[name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| bad(&format!("\"{name}\" saknas")))
    };
    let mut lines = Vec::new();
    for l in value["lines"]
        .as_array()
        .ok_or_else(|| bad("\"lines\" saknas"))?
    {
        let account = match &l["account"] {
            Value::Null => return Err(bad("\"account\" saknas")),
            a => a
                .as_u64()
                .and_then(|a| u32::try_from(a).ok())
                .ok_or_else(|| bad("\"account\" måste vara ett heltal"))?,
        };
        lines.push((
            account,
            json_amount(l.get("debit"), "debit")?,
            json_amount(l.get("credit"), "credit")?,
        ));
    }
    let attachments = match value.get("attachments") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(paths)) => paths
            .iter()
            .map(|p| p.as_str().map(PathBuf::from))
            .collect::<Option<_>>()
            .ok_or_else(|| bad("\"attachments\" ska vara filnamn"))?,
        Some(_) => return Err(bad("\"attachments\" ska vara en lista")),
    };
    Ok(Voucher {
        date: field("date")?,
        text: field("text")?,
        lines,
        attachments,
    })
}

fn voucher_input(new: &NewVoucher) -> Result<Voucher, Failure> {
    let Some(input) = &new.input else {
        return voucher_from_flags(
            new.date.clone(),
            new.text.clone(),
            &new.debits,
            &new.credits,
            &new.attachments,
        );
    };
    if new.date.is_some()
        || new.text.is_some()
        || !new.debits.is_empty()
        || !new.credits.is_empty()
        || !new.attachments.is_empty()
    {
        return Err(Failure::usage(
            "--input kan inte kombineras med --date, --text, --debit, --credit eller --attach.",
        ));
    }
    let raw = if input == "-" {
        std::io::read_to_string(std::io::stdin())
    } else {
        std::fs::read_to_string(input)
    }
    .map_err(|_| Failure::usage(format!("Kan inte läsa {input}.")))?;
    voucher_from_json(&raw)
}

/// Reads the underlag, refusing too large ones before anything is sent.
fn read_attachments(paths: &[PathBuf]) -> Result<Vec<lpb::NewAttachment>, Failure> {
    let mut total = 0;
    let mut files = Vec::new();
    for path in paths {
        let missing = || Failure::usage(format!("Kan inte läsa {}.", path.display()));
        let size = std::fs::metadata(path).map_err(|_| missing())?.len();
        if size > FILE_LIMIT as u64 {
            return Err(Failure::new("attachment_too_large"));
        }
        let data = std::fs::read(path).map_err(|_| missing())?;
        total += data.len();
        if data.len() > FILE_LIMIT || total > TOTAL_LIMIT {
            return Err(Failure::new("attachment_too_large"));
        }
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(missing)?;
        files.push(lpb::NewAttachment { file_name, data });
    }
    Ok(files)
}

fn lines_json(lines: &[lpb::VoucherLine]) -> Vec<Value> {
    lines
        .iter()
        .map(|l| json!({ "account": l.account, "debit": kronor(l.debit), "credit": kronor(l.credit) }))
        .collect()
}

fn attachments_json(attachments: &[lpb::Attachment]) -> Vec<Value> {
    attachments
        .iter()
        .map(|a| json!({ "file_name": a.file_name, "sha256": a.id, "size": a.size, "content_type": a.content_type }))
        .collect()
}

fn none_if_zero(n: u32) -> Value {
    if n == 0 { Value::Null } else { json!(n) }
}

fn recorded_by(v: &lpb::Voucher) -> Value {
    if v.recorded_by_name.is_empty() {
        Value::Null
    } else {
        json!(v.recorded_by_name)
    }
}

fn total(lines: &[lpb::VoucherLine]) -> i64 {
    lines.iter().map(|l| l.debit).sum()
}

/// An older server drops the unknown `dry_run` field and books for real:
/// say so, with the number, instead of reporting success.
fn check_dry_run(
    requested: bool,
    answered: bool,
    number: u32,
    start: &str,
    what: &str,
) -> Result<(), Failure> {
    if !requested || answered {
        return Ok(());
    }
    let mut failure = Failure::new("dry_run_unsupported");
    // A rättelse cannot itself be corrected, so only a voucher gets the advice.
    let advice = if what == "Verifikationen" {
        format!("; rätta den med doris-cli ver correct {number}")
    } else {
        String::new()
    };
    failure.message = format!(
        "Servern stöder inte --dry-run. {what} bokfördes som nummer {number} i räkenskapsåret {}{advice}.",
        year_label(start)
    );
    failure.details = Some(json!({ "number": number, "fiscal_year_start": start }));
    Err(failure)
}

/// `2026` when the year starts on 1 January, else the start date.
fn year_label(start: &str) -> &str {
    start.strip_suffix("-01-01").unwrap_or(start)
}

async fn vouchers(
    context: &Context,
    year: Option<&str>,
) -> Result<(String, Vec<lpb::Voucher>), Failure> {
    let company = super::company(context).await?;
    let start = super::fiscal_year(context, &company.id, year).await?;
    let mut vouchers = context
        .doris
        .ledger()
        .list_vouchers(context.doris.request(lpb::ListVouchersRequest {
            company_id: company.id,
            fiscal_year_start: start.clone(),
        }))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner()
        .vouchers;
    vouchers.sort_by_key(|v| std::cmp::Reverse(v.number));
    Ok((start, vouchers))
}

pub async fn list(
    context: &Context,
    output: &mut Output<'_>,
    year: Option<&str>,
) -> Result<(), Failure> {
    let (_, vouchers) = vouchers(context, year).await?;
    let value: Vec<_> = vouchers
        .iter()
        .map(|v| {
            json!({
                "number": v.number, "date": v.date, "text": v.text,
                "total": kronor(total(&v.lines)),
                "corrects": none_if_zero(v.corrects),
                "corrected_by": none_if_zero(v.corrected_by),
                "attachments": v.attachments.len(),
                "recorded_at": v.recorded_at, "recorded_by": recorded_by(v),
            })
        })
        .collect();
    let text: String = vouchers
        .iter()
        .map(|v| {
            format!(
                "{:>4}  {}  {:>12}  {}\n",
                v.number,
                v.date,
                display(total(&v.lines)),
                v.text
            )
        })
        .collect();
    context.print(output, json!(value), text);
    Ok(())
}

pub async fn view(
    context: &Context,
    output: &mut Output<'_>,
    number: u32,
    year: Option<&str>,
) -> Result<(), Failure> {
    let (start, vouchers) = vouchers(context, year).await?;
    let v = vouchers
        .iter()
        .find(|v| v.number == number)
        .ok_or_else(|| Failure::new("voucher_not_found"))?;
    let value = json!({
        "fiscal_year_start": start, "number": v.number, "date": v.date, "text": v.text,
        "lines": lines_json(&v.lines),
        "corrects": none_if_zero(v.corrects), "corrected_by": none_if_zero(v.corrected_by),
        "attachments": attachments_json(&v.attachments),
        "recorded_at": v.recorded_at, "recorded_by": recorded_by(v),
    });
    let column = |ore: i64| {
        if ore == 0 {
            String::new()
        } else {
            display(ore)
        }
    };
    let mut text = format!("Verifikation {}  {}  {}\n", v.number, v.date, v.text);
    for l in &v.lines {
        text.push_str(&format!(
            "  {}  {:>12}  {:>12}\n",
            l.account,
            column(l.debit),
            column(l.credit)
        ));
    }
    for a in &v.attachments {
        text.push_str(&format!("  Underlag: {} ({} byte)\n", a.file_name, a.size));
    }
    context.print(output, value, text);
    Ok(())
}

pub async fn new(
    context: &Context,
    output: &mut Output<'_>,
    args: &NewVoucher,
) -> Result<(), Failure> {
    let voucher = voucher_input(args)?;
    let files = read_attachments(&voucher.attachments)?;
    record(context, output, voucher, files).await
}

/// Books `voucher` with `files` as its underlag (or rehearses it).
pub(crate) async fn record(
    context: &Context,
    output: &mut Output<'_>,
    voucher: Voucher,
    files: Vec<lpb::NewAttachment>,
) -> Result<(), Failure> {
    let company = super::company(context).await?;
    let lines: Vec<_> = voucher
        .lines
        .iter()
        .map(|&(account, debit, credit)| lpb::VoucherLine {
            account,
            debit,
            credit,
        })
        .collect();
    let answer = context
        .doris
        .ledger()
        .record_voucher(context.doris.request(lpb::RecordVoucherRequest {
            company_id: company.id,
            date: voucher.date.clone(),
            text: voucher.text.clone(),
            lines: lines.clone(),
            attachments: files,
            dry_run: context.dry_run,
        }))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner();
    check_dry_run(
        context.dry_run,
        answer.dry_run,
        answer.number,
        &answer.fiscal_year_start,
        "Verifikationen",
    )?;
    let value = json!({
        "dry_run": answer.dry_run, "fiscal_year_start": answer.fiscal_year_start,
        "number": answer.number, "date": voucher.date, "text": voucher.text,
        "lines": lines_json(&lines), "attachments": attachments_json(&answer.attachments),
    });
    let summary = format!(
        "{}, {} kr, {} underlag",
        voucher.date,
        display(total(&lines)),
        answer.attachments.len()
    );
    let year = year_label(&answer.fiscal_year_start);
    let text = if answer.dry_run {
        format!(
            "Skulle bokföras som verifikation {} i räkenskapsåret {year} ({summary}). Ingenting sparades.\n",
            answer.number
        )
    } else {
        format!(
            "Verifikation {} i räkenskapsåret {year} bokförd ({summary}).\n",
            answer.number
        )
    };
    output.print(value, &text);
    Ok(())
}

pub async fn correct(
    context: &Context,
    output: &mut Output<'_>,
    number: u32,
    date: &str,
    year: Option<&str>,
) -> Result<(), Failure> {
    let company = super::company(context).await?;
    let start = super::fiscal_year(context, &company.id, year).await?;
    let answer = context
        .doris
        .ledger()
        .correct_voucher(context.doris.request(lpb::CorrectVoucherRequest {
            company_id: company.id,
            fiscal_year_start: start.clone(),
            number,
            date: date.into(),
            dry_run: context.dry_run,
        }))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner();
    check_dry_run(
        context.dry_run,
        answer.dry_run,
        answer.number,
        &start,
        "Rättelsen",
    )?;
    let value = json!({
        "dry_run": answer.dry_run, "fiscal_year_start": start,
        "number": answer.number, "corrects": number,
    });
    let text = if answer.dry_run {
        format!(
            "Skulle rättas med verifikation {}. Ingenting sparades.\n",
            answer.number
        )
    } else {
        format!(
            "Verifikation {number} rättad med verifikation {}.\n",
            answer.number
        )
    };
    output.print(value, &text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voucher_value_takes_amounts_as_strings_or_decimal_numbers() {
        let v = json!({
            "date": "2026-02-02", "text": "Kontorsmaterial",
            "lines": [
                {"account": 6110, "debit": 199.99},
                {"account": 1930, "credit": "199,99"}
            ]
        });

        let voucher = voucher_from_value(&v, "Ogiltigt argument").unwrap();

        assert_eq!(voucher.lines, vec![(6110, 19999, 0), (1930, 0, 19999)]);
        assert!(voucher.attachments.is_empty());
    }

    #[test]
    fn a_bad_voucher_value_names_the_field() {
        let f = voucher_from_value(
            &json!({"date": "2026-02-02", "lines": []}),
            "Ogiltigt argument",
        )
        .unwrap_err();

        assert_eq!(f.code, "usage");
        assert_eq!(f.message, "Ogiltigt argument: \"text\" saknas.");
    }

    #[test]
    fn a_server_that_ignored_dry_run_is_an_error_with_the_number() {
        assert!(check_dry_run(false, false, 3, "2026-01-01", "Verifikationen").is_ok());
        assert!(check_dry_run(true, true, 3, "2026-01-01", "Verifikationen").is_ok());
        let f = check_dry_run(true, false, 3, "2026-01-01", "Verifikationen").unwrap_err();
        assert_eq!((f.code.as_str(), f.exit), ("dry_run_unsupported", 1));
        assert_eq!(
            f.message,
            "Servern stöder inte --dry-run. Verifikationen bokfördes som nummer 3 i räkenskapsåret 2026; rätta den med doris-cli ver correct 3."
        );
        assert_eq!(
            f.details,
            Some(json!({"number": 3, "fiscal_year_start": "2026-01-01"}))
        );
        let f = check_dry_run(true, false, 4, "2026-07-01", "Rättelsen").unwrap_err();
        assert!(
            f.message
                .contains("Rättelsen bokfördes som nummer 4 i räkenskapsåret 2026-07-01."),
            "{}",
            f.message
        );
    }

    #[test]
    fn a_line_is_an_account_and_an_amount() {
        assert_eq!(line("6110=800").unwrap(), (6110, 80_000));
        assert_eq!(line("2641=200,50").unwrap(), (2641, 20_050));
        for bad in [
            "6110",
            "=800",
            "6110=",
            "61a0=800",
            "6110=-80",
            "6110=1.234",
        ] {
            assert_eq!(line(bad).unwrap_err().code, "usage", "{bad}");
        }
    }

    #[test]
    fn json_input_says_the_same_as_the_flags() {
        let from_json = voucher_from_json(
            r#"{"date":"2026-02-02","text":"Kontor","lines":[
            {"account":6110,"debit":"800"},{"account":2641,"debit":200},{"account":1930,"credit":"1000.00"}],
            "attachments":[]}"#,
        )
        .unwrap();
        let from_flags = voucher_from_flags(
            Some("2026-02-02".into()),
            Some("Kontor".into()),
            &["6110=800".into(), "2641=200".into()],
            &["1930=1000".into()],
            &[],
        )
        .unwrap();
        assert_eq!(from_json, from_flags);
        let too_fine = r#"{"date":"d","text":"t","lines":[{"account":1,"debit":1.234}]}"#;
        assert_eq!(voucher_from_json(too_fine).unwrap_err().code, "usage");
    }

    #[test]
    fn json_amounts_are_kronor_without_sign_and_with_two_decimals() {
        for bad in [r#""-80""#, "-80", r#""1.234""#, "1.234", "true"] {
            let raw =
                format!(r#"{{"date":"d","text":"t","lines":[{{"account":1,"debit":{bad}}}]}}"#);
            assert_eq!(voucher_from_json(&raw).unwrap_err().code, "usage", "{bad}");
        }
    }

    #[test]
    fn a_json_account_must_be_an_integer() {
        let raw = r#"{"date":"d","text":"t","lines":[{"account":"6110","debit":1}]}"#;
        let failure = voucher_from_json(raw).unwrap_err();
        assert_eq!(failure.code, "usage");
        assert!(
            failure.message.contains("måste vara ett heltal"),
            "{failure:?}"
        );
    }

    #[test]
    fn underlag_over_the_limits_are_refused_before_anything_is_sent() {
        let dir = std::env::temp_dir().join(format!("doris-cli-underlag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = |name: &str, size: usize| {
            let path = dir.join(name);
            std::fs::write(&path, vec![b'x'; size]).unwrap();
            path
        };
        let (big, a, b, c) = (
            file("big.pdf", FILE_LIMIT + 1),
            file("a.pdf", FILE_LIMIT),
            file("b.pdf", FILE_LIMIT),
            file("c.pdf", 1),
        );
        assert_eq!(
            read_attachments(&[big]).unwrap_err().code,
            "attachment_too_large"
        );
        assert_eq!(read_attachments(&[a.clone(), b.clone()]).unwrap().len(), 2);
        assert_eq!(
            read_attachments(&[a, b, c]).unwrap_err().code,
            "attachment_too_large"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn input_and_flags_do_not_mix() {
        let new = |attach: Vec<PathBuf>, text: Option<&str>| NewVoucher {
            date: None,
            text: text.map(String::from),
            debits: vec![],
            credits: vec![],
            attachments: attach,
            input: Some("finns-inte.json".into()),
            year: None,
        };
        for mixed in [new(vec![], Some("x")), new(vec!["a.pdf".into()], None)] {
            let failure = voucher_input(&mixed).unwrap_err();
            assert_eq!((failure.code.as_str(), failure.exit), ("usage", 2));
            assert!(failure.message.contains("--input kan inte"), "{failure:?}");
        }
        // Alone, --year does not count as a flag; the file is simply missing.
        let alone = voucher_input(&new(vec![], None)).unwrap_err();
        assert!(alone.message.contains("Kan inte läsa"), "{alone:?}");
    }
}
