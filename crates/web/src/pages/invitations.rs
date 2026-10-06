use crate::api::{api, pb};
use crate::errors::describe;
use crate::format::date;
use crate::passkey;
use crate::task::spawn_local;
use crate::ui::{Button, Card, ErrorAlert, Field, PageHeader};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;

/// Begins the invitation, has one of the admin's passkeys confirm it and
/// finishes; returns the invitation's token.
async fn invite_with_passkey(email: String) -> Result<String, String> {
    let mut api = api();
    let begin = api
        .begin_create_invitation(pb::CreateInvitationRequest { email })
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    let credential_json = passkey::get(&begin.options_json).await?;
    let finish = pb::FinishConfirmationRequest {
        ceremony_id: begin.ceremony_id,
        credential_json,
    };
    Ok(api
        .finish_create_invitation(finish)
        .await
        .map_err(|s| describe(&s))?
        .into_inner()
        .token)
}

#[component]
pub fn Invitations() -> impl IntoView {
    let invitations = RwSignal::new(Vec::<pb::Invitation>::new());
    let email = RwSignal::new(String::new());
    let link = RwSignal::new(None::<String>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let refresh = move || {
        spawn_local(async move {
            match api().list_invitations(pb::ListInvitationsRequest {}).await {
                Ok(list) => invitations.set(list.into_inner().invitations),
                Err(status) => error.set(Some(describe(&status))),
            }
        })
    };
    refresh();

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        link.set(None);
        spawn_local(async move {
            match invite_with_passkey(email.get_untracked()).await {
                Ok(token) => {
                    let origin = window().location().origin().unwrap_or_default();
                    link.set(Some(format!("{origin}/register?invitation={token}")));
                    email.set(String::new());
                    refresh();
                }
                Err(message) => error.set(Some(message)),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <PageHeader title="Inbjudningar" />
            <Card title="Bjud in" description="Länken gäller i 7 dagar och kan användas en gång. Du bekräftar med din passkey." narrow=true>
                <form class="grid gap-4" novalidate on:submit=submit>
                    <Field label="E-post" id="email" kind="email" value=email />
                    <ErrorAlert message=error />
                    <Button disabled=busy>"Skapa inbjudan"</Button>
                </form>
                {move || {
                    link.get().map(|url| {
                        view! {
                            <div class="mt-4 grid gap-2">
                                <label for="invitation_link" class="text-xs/relaxed font-medium">"Inbjudningslänk"</label>
                                <input id="invitation_link" readonly value=url class="h-7 w-full rounded-md border border-input bg-muted px-2 text-xs/relaxed" />
                            </div>
                        }
                    })
                }}
            </Card>
            <Show when=move || !invitations.get().is_empty()>
                <Card title="Skickade inbjudningar" narrow=true>
                    <ul class="grid gap-2">
                        <For each=move || invitations.get() key=|i| i.id.clone() let(invitation)>
                            <li class="flex justify-between gap-2">
                                <span>{invitation.email}</span>
                                <span class="text-muted-foreground">
                                    {if invitation.accepted {
                                        "Använd".to_owned()
                                    } else {
                                        format!("Giltig till {}", date(&invitation.expires_at))
                                    }}
                                </span>
                            </li>
                        </For>
                    </ul>
                </Card>
            </Show>
        </div>
    }
}
