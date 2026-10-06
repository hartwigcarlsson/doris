//! The signed-in user's API tokens: list and revoke, and create one with
//! scopes per company. The secret is shown once, right after creation.

use crate::active_company::Companies;
use crate::api::{api, pb};
use crate::errors::describe;
use crate::format::{date, plus_days, today};
use crate::task::spawn_local;
use crate::ui::{
    Badge, BadgeVariant, Button, Card, Checkbox, ErrorAlert, Field, IconName, LinkButton,
    PageHeader, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table, TableCard,
    Variant,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;

/// The form's areas in order: what reading and (if any) writing grant.
type Area = (
    &'static str,
    &'static str,
    Option<(&'static str, &'static str)>,
);
const AREAS: [Area; 5] = [
    (
        "Läsa bokföring",
        "ledger:read",
        Some(("Skriva bokföring", "ledger:write")),
    ),
    (
        "Läsa fakturor",
        "invoicing:read",
        Some(("Skriva fakturor", "invoicing:write")),
    ),
    (
        "Läsa lön",
        "payroll:read",
        Some(("Skriva lön", "payroll:write")),
    ),
    ("Läsa moms", "vat:read", Some(("Skriva moms", "vat:write"))),
    ("Läsa bolagsuppgifter", "company:read", None),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenStatus {
    Active,
    Expired,
    Revoked,
}

/// A token's status at `now` (RFC 3339, UTC, as `Date.toISOString` gives).
pub fn status_of(token: &pb::ApiToken, now: &str) -> TokenStatus {
    if token.revoked_at.is_some() {
        TokenStatus::Revoked
    } else if token.expires_at.as_str() <= now {
        TokenStatus::Expired
    } else {
        TokenStatus::Active
    }
}

/// One company's grant from its (read, write) boxes in `AREAS` order.
/// Writing brings reading; a company with nothing ticked gets no grant.
pub fn grant(company_id: &str, boxes: &[(bool, bool)]) -> Option<pb::TokenGrant> {
    let mut scopes = Vec::new();
    for ((_, read, write), &(r, w)) in AREAS.iter().zip(boxes) {
        let write = write.filter(|_| w);
        if r || write.is_some() {
            scopes.push(read.to_string());
        }
        if let Some((_, scope)) = write {
            scopes.push(scope.to_string());
        }
    }
    (!scopes.is_empty()).then(|| pb::TokenGrant {
        company_id: company_id.into(),
        scopes,
    })
}

fn now_utc() -> String {
    js_sys::Date::new_0().to_iso_string().into()
}

#[component]
pub fn ApiTokens() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let tokens = RwSignal::new(Vec::<pb::ApiToken>::new());
    let error = RwSignal::new(None::<String>);

    let refresh = move || {
        spawn_local(async move {
            match api().list_api_tokens(pb::ListApiTokensRequest {}).await {
                Ok(list) => tokens.set(list.into_inner().tokens),
                Err(status) => error.set(Some(describe(&status))),
            }
        })
    };
    refresh();

    let revoke = move |token: pb::ApiToken| {
        let question = format!("Återkalla token {}?", token.name);
        if !window().confirm_with_message(&question).unwrap_or(false) {
            return;
        }
        error.set(None);
        spawn_local(async move {
            let request = pb::RevokeApiTokenRequest { token_id: token.id };
            match api().revoke_api_token(request).await {
                Ok(_) => refresh(),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    let company_names = move |grants: &[pb::TokenGrant]| {
        let list = companies.list.get();
        grants
            .iter()
            .map(|g| {
                list.iter()
                    .find(|c| c.id == g.company_id)
                    .map_or("Okänt bolag".to_owned(), |c| c.name.clone())
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    view! {
        <div class="grid gap-6">
            <PageHeader title="API-tokens" description="För doris-cli och andra program som arbetar åt dig.">
                <LinkButton href="/settings/tokens/new" icon=IconName::Plus>"Ny token"</LinkButton>
            </PageHeader>
            <ErrorAlert message=error />
            <TableCard><Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=TABLE_HEADER_CELL>"Bolag"</th>
                        <th class=TABLE_HEADER_CELL>"Giltig till"</th>
                        <th class=TABLE_HEADER_CELL>"Senast använd"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For each=move || tokens.get() key=|t| (t.id.clone(), t.revoked_at.clone()) let(token)>
                        {
                            let status = status_of(&token, &now_utc());
                            let names = company_names(&token.grants);
                            let row = token.clone();
                            view! {
                                <tr class=TABLE_ROW>
                                    <td class=TABLE_CELL>{token.name.clone()}</td>
                                    <td class=TABLE_CELL>{names}</td>
                                    <td class=TABLE_CELL>{date(&token.expires_at).to_owned()}</td>
                                    <td class=TABLE_CELL>
                                        {token.last_used_at.as_deref().map_or("Aldrig".to_owned(), |at| date(at).to_owned())}
                                    </td>
                                    <td class=TABLE_CELL>
                                        {match status {
                                            TokenStatus::Active => view! { <Badge>"Aktiv"</Badge> }.into_any(),
                                            TokenStatus::Expired => view! { <Badge variant=BadgeVariant::Outline>"Utgången"</Badge> }.into_any(),
                                            TokenStatus::Revoked => view! { <Badge variant=BadgeVariant::Outline>"Återkallad"</Badge> }.into_any(),
                                        }}
                                    </td>
                                    <td class=TABLE_CELL>
                                        <Show when=move || status == TokenStatus::Active>
                                            {
                                                let row = row.clone();
                                                view! {
                                                    <Button variant=Variant::Ghost kind="button" on:click=move |_| revoke(row.clone())>
                                                        "Återkalla"
                                                    </Button>
                                                }
                                            }
                                        </Show>
                                    </td>
                                </tr>
                            }
                        }
                    </For>
                </tbody>
            </Table></TableCard>
        </div>
    }
}

/// One company's boxes in the form: (read, write) per area.
#[derive(Clone)]
struct CompanyBoxes {
    id: String,
    name: String,
    boxes: Vec<(RwSignal<bool>, RwSignal<bool>)>,
}

#[component]
pub fn NewApiToken() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let name = RwSignal::new(String::new());
    let last_day = RwSignal::new(plus_days(&today(), 90).unwrap_or_default());
    let rows = RwSignal::new(Vec::<CompanyBoxes>::new());
    let secret = RwSignal::new(None::<String>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    Effect::new(move |_| {
        let list = companies.list.get();
        rows.set(
            list.into_iter()
                .map(|c| CompanyBoxes {
                    id: c.id,
                    name: c.name,
                    boxes: AREAS
                        .iter()
                        .map(|_| (RwSignal::new(false), RwSignal::new(false)))
                        .collect(),
                })
                .collect(),
        );
    });

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        let grants = rows
            .get_untracked()
            .iter()
            .filter_map(|row| {
                let boxes: Vec<_> = row
                    .boxes
                    .iter()
                    .map(|(r, w)| (r.get_untracked(), w.get_untracked()))
                    .collect();
                grant(&row.id, &boxes)
            })
            .collect();
        spawn_local(async move {
            let request = pb::CreateApiTokenRequest {
                name: name.get_untracked(),
                expires_on: last_day.get_untracked(),
                grants,
            };
            match api().create_api_token(request).await {
                Ok(created) => secret.set(Some(created.into_inner().secret)),
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <PageHeader title="Ny token" />
            {move || match secret.get() {
                Some(value) => view! {
                    <Card title="Din token" description="Token visas bara nu. Spara den på ett säkert ställe.">
                        <div class="grid gap-4">
                            <div class="grid gap-2">
                                <label for="api_token_secret" class="text-xs/relaxed font-medium">"Token"</label>
                                <input id="api_token_secret" readonly value=value class="h-7 w-full rounded-md border border-input bg-muted px-2 font-mono text-xs/relaxed" />
                            </div>
                            <div><LinkButton href="/settings/tokens">"Klar"</LinkButton></div>
                        </div>
                    </Card>
                }.into_any(),
                None => view! {
                    <Card title="Behörigheter" description="En token kan aldrig mer än du själv. Skriva innefattar läsa.">
                        <form class="grid gap-4" novalidate on:submit=submit>
                            <div class="grid gap-4 sm:grid-cols-2 sm:max-w-xl">
                                <Field label="Namn" id="token_name" placeholder="t.ex. doris-cli på laptopen" value=name />
                                <Field label="Giltig till och med" id="token_last_day" kind="date" value=last_day hint=Signal::derive(|| Some("Högst ett år.")) />
                            </div>
                            <For each=move || rows.get() key=|row| row.id.clone() let(row)>
                                <fieldset class="grid gap-2 rounded-md border border-border p-3">
                                    <legend class="px-1 text-xs/relaxed font-medium">{row.name.clone()}</legend>
                                    <div class="grid gap-2 sm:grid-cols-2">
                                        {AREAS.iter().zip(row.boxes.clone()).map(|((read_label, read, write), (r, w))| {
                                            Effect::new(move |_| if w.get() { r.set(true) });
                                            view! {
                                                <Checkbox label=read_label.to_string() id=format!("{}-{read}", row.id) checked=r />
                                                {match write {
                                                    Some((write_label, scope)) => view! {
                                                        <Checkbox label=write_label.to_string() id=format!("{}-{scope}", row.id) checked=w />
                                                    }.into_any(),
                                                    None => view! { <span></span> }.into_any(),
                                                }}
                                            }
                                        }).collect_view()}
                                    </div>
                                </fieldset>
                            </For>
                            <p class="text-xs/relaxed text-muted-foreground">
                                "Läsa lön ger också AGI-filen, som innehåller de anställdas personnummer."
                            </p>
                            <ErrorAlert message=error />
                            <div><Button disabled=busy>"Skapa token"</Button></div>
                        </form>
                    </Card>
                }.into_any(),
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(expires_at: &str, revoked_at: Option<&str>) -> pb::ApiToken {
        pb::ApiToken {
            expires_at: expires_at.into(),
            revoked_at: revoked_at.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn a_token_is_active_until_it_expires_or_is_revoked() {
        let now = "2026-10-06T10:00:00.000Z";
        assert_eq!(
            status_of(&token("2026-10-06T22:00:00Z", None), now),
            TokenStatus::Active
        );
        assert_eq!(
            status_of(&token("2026-10-06T09:00:00Z", None), now),
            TokenStatus::Expired
        );
        assert_eq!(
            status_of(
                &token("2026-10-07T22:00:00Z", Some("2026-10-05T08:00:00Z")),
                now
            ),
            TokenStatus::Revoked
        );
    }

    #[test]
    fn writing_brings_reading_and_empty_companies_are_left_out() {
        // Bokföring skriva, Lön läsa, Bolag läsa (its write box does nothing).
        let boxes = [
            (false, true),
            (false, false),
            (true, false),
            (false, false),
            (true, true),
        ];
        assert_eq!(
            grant("c1", &boxes),
            Some(pb::TokenGrant {
                company_id: "c1".into(),
                scopes: [
                    "ledger:read",
                    "ledger:write",
                    "payroll:read",
                    "company:read"
                ]
                .map(String::from)
                .to_vec(),
            })
        );
        assert_eq!(grant("c2", &[(false, false); 5]), None);
    }
}
