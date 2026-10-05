use crate::api::{api, pb};
use crate::errors::describe;
use crate::format::date;
use crate::passkey;
use crate::ui::{Button, Card, ErrorAlert, Field, PageHeader};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn Passkeys() -> impl IntoView {
    let passkeys = RwSignal::new(Vec::<pb::Passkey>::new());
    let name = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let refresh = move || {
        spawn_local(async move {
            match api().list_passkeys(pb::ListPasskeysRequest {}).await {
                Ok(list) => passkeys.set(list.into_inner().passkeys),
                Err(status) => error.set(Some(describe(&status))),
            }
        })
    };
    refresh();

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match add_passkey(name.get_untracked()).await {
                Ok(()) => {
                    name.set(String::new());
                    refresh();
                }
                Err(message) => error.set(Some(message)),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <PageHeader title="Passkeys" />
            <Card title="Dina passkeys" description="Lägg till fler enheter så att du inte blir utelåst." narrow=true>
                <ul class="grid gap-2">
                    <For each=move || passkeys.get() key=|p| p.credential_id.clone() let(passkey)>
                        <li class="flex justify-between gap-2">
                            <span class="font-medium">{passkey.name}</span>
                            <span class="text-muted-foreground">
                                {match &passkey.last_used_at {
                                    Some(at) => format!("Senast använd {}", date(at)),
                                    None => format!("Tillagd {}", date(&passkey.added_at)),
                                }}
                            </span>
                        </li>
                    </For>
                </ul>
            </Card>
            <Card title="Lägg till passkey" narrow=true>
                <form class="grid gap-4" novalidate on:submit=submit>
                    <Field label="Passkeyns namn" id="passkey_name" placeholder="t.ex. iPhone" value=name />
                    <ErrorAlert message=error />
                    <Button disabled=busy>"Lägg till passkey"</Button>
                </form>
            </Card>
        </div>
    }
}

async fn add_passkey(passkey_name: String) -> Result<(), String> {
    let mut api = api();
    let begin = api
        .begin_add_passkey(pb::BeginAddPasskeyRequest { passkey_name })
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    let credential_json = passkey::create(&begin.options_json).await?;
    api.finish_add_passkey(pb::FinishAddPasskeyRequest {
        ceremony_id: begin.ceremony_id,
        credential_json,
    })
    .await
    .map_err(|s| describe(&s))?;
    Ok(())
}
