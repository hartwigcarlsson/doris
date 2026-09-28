use crate::api::{api, pb};
use crate::app::Session;
use crate::errors::describe;
use crate::passkey;
use crate::ui::{Button, Card, ErrorAlert, Field};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::{A, Redirect};

#[component]
pub fn Login() -> impl IntoView {
    let session = expect_context::<Session>();
    let email = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let done = RwSignal::new(false);

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match log_in(email.get_untracked()).await {
                Ok(user) => {
                    session.signed_in(user);
                    done.set(true);
                }
                Err(message) => error.set(Some(message)),
            }
            busy.set(false);
        });
    };

    view! {
        {move || done.get().then(|| view! { <Redirect path="/" /> })}
        <Show when=move || !session.bootstrap_required.get() fallback=|| view! { <Redirect path="/register" /> }>
            <Card title="Logga in" description="Använd din passkey.">
                <form class="grid gap-4" novalidate on:submit=submit>
                    <Field label="E-post" id="email" kind="email" autocomplete="username" value=email />
                    <ErrorAlert message=error />
                    <Button disabled=busy>"Logga in med passkey"</Button>
                </form>
                <p class="mt-4 text-muted-foreground">
                    "Har du en inbjudan? Öppna länken du fått. "
                    <A href="/register" attr:class="underline">"Registrera"</A>
                </p>
            </Card>
        </Show>
    }
}

async fn log_in(email: String) -> Result<pb::User, String> {
    let mut api = api();
    let begin = api
        .begin_login(pb::BeginLoginRequest { email })
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    // Every login failure reads the same, whether or not the email exists.
    let credential_json = passkey::get(&begin.options_json)
        .await
        .map_err(|_| "Inloggningen misslyckades.".to_owned())?;
    let user = api
        .finish_login(pb::FinishLoginRequest {
            ceremony_id: begin.ceremony_id,
            credential_json,
        })
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    Ok(user)
}
