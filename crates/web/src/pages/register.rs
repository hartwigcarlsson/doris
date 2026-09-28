use crate::api::{api, pb};
use crate::app::Session;
use crate::errors::describe;
use crate::passkey;
use crate::ui::{Button, Card, ErrorAlert, Field};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::{A, Redirect};
use leptos_router::hooks::use_query_map;

#[component]
pub fn Register() -> impl IntoView {
    let session = expect_context::<Session>();
    let query = use_query_map();
    let invitation = move || query.read().get("invitation");
    let email = RwSignal::new(String::new());
    let display_name = RwSignal::new(String::new());
    let passkey_name = RwSignal::new(String::new());
    let email_locked = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let invitation_failed = RwSignal::new(false);

    // An invitation decides the email; show it and lock the field.
    Effect::new(move |_| {
        if let Some(token) = invitation() {
            spawn_local(async move {
                match api()
                    .get_invitation(pb::GetInvitationRequest { token })
                    .await
                {
                    Ok(found) => {
                        email.set(found.into_inner().email);
                        email_locked.set(true);
                    }
                    Err(status) => {
                        error.set(Some(describe(&status)));
                        invitation_failed.set(true);
                    }
                }
            });
        }
    });

    let done = RwSignal::new(false);
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let request = pb::BeginRegistrationRequest {
                email: email.get_untracked(),
                display_name: display_name.get_untracked(),
                invitation_token: invitation(),
                passkey_name: passkey_name.get_untracked(),
            };
            match register(request).await {
                Ok(user) => {
                    session.signed_in(user);
                    done.set(true);
                }
                Err(message) => error.set(Some(message)),
            }
            busy.set(false);
        });
    };

    let open = move || {
        !invitation_failed.get() && (session.bootstrap_required.get() || invitation().is_some())
    };
    let title = if session.bootstrap_required.get_untracked() {
        "Skapa administratörskonto"
    } else {
        "Registrera dig"
    };
    view! {
        {move || done.get().then(|| view! { <Redirect path="/" /> })}
        <Card title=title description="Du loggar in med en passkey – inget lösenord behövs.">
            <Show
                when=open
                fallback=move || {
                    view! {
                        <ErrorAlert message=error />
                        <p class="text-muted-foreground">
                            {move || {
                                if invitation_failed.get() {
                                    ""
                                } else {
                                    "Registrering kräver en inbjudan. "
                                }
                            }} <A href="/login" attr:class="underline">"Logga in"</A>
                        </p>
                    }
                }
            >
                <form class="grid gap-3" on:submit=submit>
                    <Field label="E-post" id="email" kind="email" autocomplete="username" value=email readonly=email_locked />
                    <Field label="Namn" id="display_name" autocomplete="name" value=display_name />
                    <Field label="Passkeyns namn" id="passkey_name" placeholder="t.ex. MacBook" value=passkey_name />
                    <ErrorAlert message=error />
                    <Button disabled=busy>"Skapa konto med passkey"</Button>
                </form>
            </Show>
        </Card>
    }
}

async fn register(request: pb::BeginRegistrationRequest) -> Result<pb::User, String> {
    let mut api = api();
    let invitation_token = request.invitation_token.clone();
    let begin = api
        .begin_registration(request)
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    let credential_json = passkey::create(&begin.options_json).await?;
    let user = api
        .finish_registration(pb::FinishRegistrationRequest {
            ceremony_id: begin.ceremony_id,
            invitation_token,
            credential_json,
        })
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    Ok(user)
}
