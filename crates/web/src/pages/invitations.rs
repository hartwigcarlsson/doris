use crate::api::{api, pb};
use crate::errors::describe;
use crate::ui::{Button, Card, ErrorAlert, Field};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

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
            let request = pb::CreateInvitationRequest {
                email: email.get_untracked(),
            };
            match api().create_invitation(request).await {
                Ok(created) => {
                    let origin = window().location().origin().unwrap_or_default();
                    let token = created.into_inner().token;
                    link.set(Some(format!("{origin}/register?invitation={token}")));
                    email.set(String::new());
                    refresh();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <Card title="Bjud in" description="Länken gäller i 7 dagar och kan användas en gång.">
                <form class="grid gap-3" on:submit=submit>
                    <Field label="E-post" id="email" kind="email" value=email />
                    <ErrorAlert message=error />
                    <Button disabled=busy>"Skapa inbjudan"</Button>
                </form>
                {move || {
                    link.get().map(|url| {
                        view! {
                            <div class="mt-4 grid gap-1.5">
                                <label for="invitation_link" class="text-xs/relaxed font-medium">"Inbjudningslänk"</label>
                                <input id="invitation_link" readonly value=url class="h-7 w-full rounded-md border border-input bg-muted px-2 text-xs/relaxed" />
                            </div>
                        }
                    })
                }}
            </Card>
            <Card title="Inbjudningar">
                <ul class="grid gap-2">
                    <For each=move || invitations.get() key=|i| i.id.clone() let(invitation)>
                        <li class="flex justify-between gap-2">
                            <span>{invitation.email}</span>
                            <span class="text-muted-foreground">
                                {if invitation.accepted {
                                    "Använd".to_owned()
                                } else {
                                    format!("Giltig till {}", &invitation.expires_at[..10])
                                }}
                            </span>
                        </li>
                    </For>
                </ul>
            </Card>
        </div>
    }
}
