//! Components in the style of shadcn preset b1Gdz9bFY (radix-mira). Class
//! lists are copied from the generated shadcn components; only what the app
//! uses is here.

use leptos::prelude::*;

const BUTTON: &str = "inline-flex shrink-0 items-center justify-center gap-1 rounded-md border border-transparent bg-clip-padding text-xs/relaxed font-medium whitespace-nowrap transition-all outline-none select-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30 active:translate-y-px disabled:pointer-events-none disabled:opacity-50 h-7 px-2 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg]:size-3.5";
const BUTTON_DEFAULT: &str = "bg-primary text-primary-foreground hover:bg-primary/80";
const BUTTON_GHOST: &str = "hover:bg-muted hover:text-foreground dark:hover:bg-muted/50";
const INPUT: &str = "h-7 w-full min-w-0 rounded-md border border-input bg-input/20 px-2 py-0.5 text-sm transition-colors outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30 disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 read-only:cursor-default read-only:border-dashed read-only:bg-muted read-only:text-muted-foreground md:text-xs/relaxed dark:bg-input/30";
const LABEL: &str = "flex items-center gap-2 text-xs/relaxed leading-none font-medium select-none";
const CARD: &str = "flex flex-col gap-4 overflow-hidden rounded-lg bg-card py-4 text-xs/relaxed text-card-foreground ring-1 ring-foreground/10";
const ALERT: &str =
    "relative grid w-full gap-0.5 rounded-lg border px-2 py-1.5 text-left text-xs/relaxed";

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Variant {
    #[default]
    Default,
    Ghost,
}

#[component]
pub fn Button(
    #[prop(optional)] variant: Variant,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(default = "submit")] kind: &'static str,
    children: Children,
) -> impl IntoView {
    let look = match variant {
        Variant::Default => BUTTON_DEFAULT,
        Variant::Ghost => BUTTON_GHOST,
    };
    view! {
        <button type=kind class=format!("{BUTTON} {look}") disabled=disabled>
            {children()}
        </button>
    }
}

/// A labelled text input bound to `value`.
#[component]
pub fn Field(
    label: &'static str,
    id: &'static str,
    value: RwSignal<String>,
    #[prop(default = "text")] kind: &'static str,
    #[prop(optional)] autocomplete: &'static str,
    #[prop(optional)] placeholder: &'static str,
    #[prop(optional, into)] readonly: Signal<bool>,
    /// Explanation shown under the field while `Some`.
    #[prop(optional, into)]
    hint: Signal<Option<&'static str>>,
) -> impl IntoView {
    let hint_id = format!("{id}-hint");
    let described_by = {
        let hint_id = hint_id.clone();
        move || hint.get().map(|_| hint_id.clone())
    };
    view! {
        <div class="grid gap-1.5">
            <label for=id class=LABEL>
                {label}
            </label>
            <input
                id=id
                name=id
                type=kind
                class=INPUT
                required
                autocomplete=autocomplete
                placeholder=placeholder
                readonly=readonly
                aria-describedby=described_by
                bind:value=value
            />
            {move || {
                hint.get()
                    .map(|text| {
                        view! {
                            <p id=hint_id.clone() class="text-xs/relaxed text-muted-foreground">
                                {text}
                            </p>
                        }
                    })
            }}
        </div>
    }
}

#[component]
pub fn Card(
    title: &'static str,
    #[prop(optional)] description: &'static str,
    children: Children,
) -> impl IntoView {
    view! {
        <section class=CARD>
            <header class="grid gap-1 px-4">
                <h1 class="text-sm font-medium">{title}</h1>
                {(!description.is_empty())
                    .then(|| view! { <p class="text-xs/relaxed text-muted-foreground">{description}</p> })}
            </header>
            <div class="px-4">{children()}</div>
        </section>
    }
}

/// An error message, shown while `message` is `Some`.
#[component]
pub fn ErrorAlert(message: RwSignal<Option<String>>) -> impl IntoView {
    move || {
        message.get().map(|text| {
            view! {
                <div role="alert" class=format!("{ALERT} bg-card text-destructive")>
                    {text}
                </div>
            }
        })
    }
}
