//! doris-cli's commands as MCP tools: the same functions, the same JSON.
//! The server's `/mcp` (`crates/server/src/mcp.rs`) speaks the protocol
//! and hands each call here with a transport into its own process.

use crate::client::Doris;
use crate::commands::{self, Context, check_year};
use crate::output::{Failure, Output};
use serde_json::{Map, Value, json};

const COMPANY: &str =
    "Organisation number (with or without hyphen) or id. Omitted: the token's only company.";
const YEAR: &str = "The räkenskapsår: a year (\"2026\") or its start date (\"2026-07-01\"). Omitted: the year containing today.";
const DRY_RUN: &str = "true: run every rule, then roll back; nothing is saved.";

fn string(description: &str) -> Value {
    json!({"type": "string", "description": description})
}
fn integer(description: &str) -> Value {
    json!({"type": "integer", "description": description})
}

fn tool(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
    writes: Option<bool>,
) -> Value {
    let annotations = match writes {
        None => json!({"readOnlyHint": true, "openWorldHint": false}),
        Some(destructive) => json!({
            "readOnlyHint": false, "destructiveHint": destructive,
            "idempotentHint": false, "openWorldHint": false
        }),
    };
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object", "properties": properties,
            "required": required, "additionalProperties": false
        },
        "annotations": annotations,
    })
}

/// Every tool: name, description, input schema and annotations.
pub fn list() -> Vec<Value> {
    let company = || json!({"company": string(COMPANY)});
    let company_year = |extra: Value| {
        let mut p = json!({"company": string(COMPANY), "year": string(YEAR)});
        p.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        p
    };
    let line = json!({
        "type": "object",
        "properties": {
            "account": {"type": "integer"},
            "debit": {"type": ["string", "number"], "description": "Kronor, e.g. \"800.00\"."},
            "credit": {"type": ["string", "number"], "description": "Kronor, e.g. \"800.00\"."}
        },
        "required": ["account"], "additionalProperties": false
    });
    vec![
        tool(
            "whoami",
            "Who the token belongs to (doris-cli auth status).",
            json!({}),
            &[],
            None,
        ),
        tool(
            "list_companies",
            "The companies the token reaches (company list).",
            json!({}),
            &[],
            None,
        ),
        tool(
            "get_company",
            "One company's details and fiscal year (company view).",
            company(),
            &[],
            None,
        ),
        tool(
            "list_fiscal_years",
            "Fiscal years, open or closed (year list).",
            company(),
            &[],
            None,
        ),
        tool(
            "list_accounts",
            "The chart of accounts (account list).",
            company(),
            &[],
            None,
        ),
        tool(
            "list_vouchers",
            "The fiscal year's vouchers, newest first (ver list).",
            company_year(json!({})),
            &[],
            None,
        ),
        tool(
            "get_voucher",
            "One voucher with its lines and underlag (ver view).",
            company_year(json!({"number": integer("Voucher number.")})),
            &["number"],
            None,
        ),
        tool(
            "record_voucher",
            "Book a voucher (ver new). Rehearse with dry_run: true first. A booked voucher can never be changed or removed.",
            json!({
                "company": string(COMPANY),
                "date": string("YYYY-MM-DD; decides the fiscal year."),
                "text": string("Voucher text."),
                "lines": {"type": "array", "items": line, "minItems": 1},
                "dry_run": {"type": "boolean", "description": DRY_RUN}
            }),
            &["date", "text", "lines"],
            Some(false),
        ),
        tool(
            "correct_voucher",
            "Reverse every line of a voucher with a new rättelse voucher (ver correct). Rehearse with dry_run: true first.",
            company_year(json!({
                "number": integer("The voucher to correct."),
                "date": string("Date of the rättelse, YYYY-MM-DD: today."),
                "dry_run": {"type": "boolean", "description": DRY_RUN}
            })),
            &["number", "date"],
            Some(true),
        ),
        tool(
            "trial_balance",
            "Saldobalans for the fiscal year (report trial-balance).",
            company_year(json!({})),
            &[],
            None,
        ),
        tool(
            "account_ledger",
            "Huvudbok for one account (report ledger).",
            company_year(json!({"account": integer("Account number.")})),
            &["account"],
            None,
        ),
        tool(
            "financial_statements",
            "Resultat- och balansräkning with the year before (report statements).",
            company_year(json!({})),
            &[],
            None,
        ),
    ]
}

/// The agent's rules for these tools.
pub fn instructions() -> &'static str {
    include_str!("tools.md")
}

/// Checks `args` against the tool's schema: an object, only known fields,
/// required ones present, top-level types right.
fn check(schema: &Value, args: &Value) -> Result<(), Failure> {
    let Some(args) = args.as_object() else {
        return Err(Failure::usage("Argumenten ska vara ett JSON-objekt."));
    };
    let properties = schema["properties"]
        .as_object()
        .expect("schema has properties");
    if let Some(unknown) = args.keys().find(|k| !properties.contains_key(*k)) {
        return Err(Failure::usage(format!("Okänt argument \"{unknown}\".")));
    }
    for required in schema["required"].as_array().into_iter().flatten() {
        let name = required.as_str().unwrap();
        if args.get(name).is_none_or(Value::is_null) {
            return Err(Failure::usage(format!("Argumentet \"{name}\" saknas.")));
        }
    }
    for (name, value) in args {
        let ok = match properties[name]["type"].as_str() {
            Some("string") => value.is_string(),
            Some("integer") => value.as_u64().is_some_and(|n| n <= u32::MAX as u64),
            Some("boolean") => value.is_boolean(),
            Some("array") => value.is_array(),
            _ => true,
        };
        if !ok && !value.is_null() {
            return Err(Failure::usage(format!(
                "Argumentet \"{name}\" har fel typ."
            )));
        }
    }
    Ok(())
}

/// Runs tool `name` as `doris-cli --json` would run its command. `None`
/// when there is no such tool.
pub async fn call(doris: Doris, name: &str, args: Value) -> Option<Result<Value, Value>> {
    let tool = list().into_iter().find(|t| t["name"] == name)?;
    let args = if args.is_null() {
        Value::Object(Map::new())
    } else {
        args
    };
    Some(
        run(doris, name, &tool["inputSchema"], &args)
            .await
            .map_err(|f| f.to_json()),
    )
}

async fn run(doris: Doris, name: &str, schema: &Value, args: &Value) -> Result<Value, Failure> {
    check(schema, args)?;
    let text = |field: &str| args[field].as_str().map(str::to_owned);
    let number = |field: &str| args[field].as_u64().map(|n| n as u32).unwrap_or_default();
    let year = text("year");
    check_year(year.as_deref(), "year")?;
    let context = Context {
        doris,
        company: text("company"),
        dry_run: args["dry_run"].as_bool().unwrap_or(false),
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut output = Output {
        json: true,
        out: &mut out,
        err: &mut err,
    };
    let year = year.as_deref();
    use commands::{account, auth, company, report, ver, year as years};
    match name {
        "whoami" => auth::status(&context, &mut output).await,
        "list_companies" => company::list(&context, &mut output).await,
        "get_company" => company::view(&context, &mut output).await,
        "list_fiscal_years" => years::list(&context, &mut output).await,
        "list_accounts" => account::list(&context, &mut output).await,
        "list_vouchers" => ver::list(&context, &mut output, year).await,
        "get_voucher" => ver::view(&context, &mut output, number("number"), year).await,
        "record_voucher" => {
            let voucher = ver::voucher_from_value(args, "Ogiltigt argument")?;
            ver::record(&context, &mut output, voucher, Vec::new()).await
        }
        "correct_voucher" => {
            let date = text("date").unwrap_or_default();
            ver::correct(&context, &mut output, number("number"), &date, year).await
        }
        "trial_balance" => report::trial_balance(&context, &mut output, year).await,
        "account_ledger" => report::ledger(&context, &mut output, number("account"), year).await,
        "financial_statements" => report::statements(&context, &mut output, year).await,
        _ => unreachable!("every listed tool is dispatched"),
    }?;
    serde_json::from_slice(&out).map_err(|_| Failure::new("internal"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The command each tool runs, as `doris-cli <area> <action>`.
    const COMMANDS: &[(&str, &str)] = &[
        ("whoami", "auth status"),
        ("list_companies", "company list"),
        ("get_company", "company view"),
        ("list_fiscal_years", "year list"),
        ("list_accounts", "account list"),
        ("list_vouchers", "ver list"),
        ("get_voucher", "ver view"),
        ("record_voucher", "ver new"),
        ("correct_voucher", "ver correct"),
        ("trial_balance", "report trial-balance"),
        ("account_ledger", "report ledger"),
        ("financial_statements", "report statements"),
    ];

    fn names() -> BTreeSet<String> {
        list()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    }

    fn cli_commands() -> BTreeSet<String> {
        use clap::CommandFactory;
        crate::Cli::command()
            .get_subcommands()
            .filter(|area| area.get_name() != "skill")
            .flat_map(|area| {
                area.get_subcommands()
                    .map(|a| format!("{} {}", area.get_name(), a.get_name()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn every_command_but_skill_is_a_tool_and_back() {
        let mapped: BTreeSet<String> = COMMANDS.iter().map(|(_, c)| c.to_string()).collect();
        let tools: BTreeSet<String> = COMMANDS.iter().map(|(t, _)| t.to_string()).collect();

        assert_eq!(mapped, cli_commands());
        assert_eq!(tools, names());
    }

    #[test]
    fn every_tool_has_a_closed_schema_and_says_what_it_changes() {
        for tool in list() {
            let name = tool["name"].as_str().unwrap();
            assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
            assert_eq!(tool["inputSchema"]["additionalProperties"], false, "{name}");
            assert_eq!(tool["annotations"]["openWorldHint"], false, "{name}");
            let writes = matches!(name, "record_voucher" | "correct_voucher");
            assert_eq!(tool["annotations"]["readOnlyHint"], !writes, "{name}");
        }
        let correct = list()
            .into_iter()
            .find(|t| t["name"] == "correct_voucher")
            .unwrap();
        assert_eq!(correct["annotations"]["destructiveHint"], true);
    }

    fn doris() -> Doris {
        // Never called: these arguments are refused before any request.
        let unreachable = tower::service_fn(|_| async {
            Err::<http::Response<tonic::body::Body>, _>(tower::BoxError::from("no server"))
        });
        Doris::with_transport("doris_x".into(), crate::client::Transport::new(unreachable)).unwrap()
    }

    async fn usage(name: &str, args: Value) -> String {
        let err = call(doris(), name, args).await.unwrap().unwrap_err();
        assert_eq!(err["error"]["code"], "usage", "{name}: {err}");
        err["error"]["message"].as_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn bad_arguments_are_refused_by_field_before_any_call() {
        assert!(
            usage("list_vouchers", json!({"yaer": "2026"}))
                .await
                .contains("yaer")
        );
        assert!(usage("get_voucher", json!({})).await.contains("number"));
        assert!(
            usage("get_voucher", json!({"number": "7"}))
                .await
                .contains("number")
        );
        assert!(
            usage("list_vouchers", json!({"year": "26"}))
                .await
                .contains("year")
        );
        assert!(usage("record_voucher", json!({"date": "2026-02-02", "text": "x", "lines": [{"account": 1930}], "attachments": []})).await.contains("attachments"));
        assert!(usage("whoami", json!([1])).await.contains("objekt"));
    }

    #[tokio::test]
    async fn an_unknown_tool_is_none() {
        assert!(call(doris(), "delete_voucher", json!({})).await.is_none());
    }

    #[test]
    fn the_instructions_name_only_tools_that_exist() {
        let text = instructions();
        let mentioned: BTreeSet<String> = text
            .split(|c: char| !(c.is_ascii_lowercase() || c == '_'))
            .filter(|w| w.contains('_') && !w.starts_with('_') && !w.ends_with('_'))
            .map(str::to_owned)
            .filter(|w| {
                !matches!(
                    w.as_str(),
                    "dry_run" | "voucher_unbalanced" | "fiscal_year_start" | "corrected_by"
                )
            })
            .collect();
        assert!(!mentioned.is_empty());
        assert!(
            mentioned.is_subset(&names()),
            "{:?}",
            mentioned.difference(&names())
        );
        assert!(
            !text.contains("crates/") && !text.contains("doris-cli "),
            "points at the repo or the CLI"
        );
    }
}
