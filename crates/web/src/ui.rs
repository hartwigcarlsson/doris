//! Components in the style of shadcn preset b1Gdz9bFY (radix-mira). Class
//! lists are copied from the generated shadcn components; only what the app
//! uses is here.
use leptos::prelude::*;

const BUTTON: &str = "inline-flex shrink-0 items-center justify-center gap-1 rounded-md border border-transparent bg-clip-padding text-xs/relaxed font-medium whitespace-nowrap transition-all outline-none select-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30 active:translate-y-px disabled:pointer-events-none disabled:opacity-50 h-7 px-2 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg]:size-3.5";
const BUTTON_DEFAULT: &str = "bg-primary text-primary-foreground hover:bg-primary/80";
const BUTTON_GHOST: &str = "hover:bg-muted hover:text-foreground dark:hover:bg-muted/50";
const INPUT: &str = "h-7 w-full min-w-0 rounded-md border border-input bg-input/20 px-2 py-0.5 text-sm transition-colors outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30 disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 read-only:cursor-default read-only:border-dashed read-only:focus-visible:border-input read-only:bg-muted read-only:text-muted-foreground md:text-xs/relaxed dark:bg-input/30";
const LABEL: &str = "flex items-center gap-2 text-xs/relaxed leading-none font-medium select-none";
/// shadcn NativeSelectOption: keeps the dropdown readable in dark mode.
pub const SELECT_OPTION: &str = "bg-[Canvas] text-[CanvasText]";
const SELECT: &str = "h-7 w-full min-w-0 appearance-none rounded-md border border-input bg-input/20 py-0.5 pr-6 pl-2 text-xs/relaxed transition-colors outline-none select-none selection:bg-primary selection:text-primary-foreground placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30 disabled:pointer-events-none disabled:cursor-not-allowed aria-invalid:border-destructive aria-invalid:ring-2 aria-invalid:ring-destructive/20 dark:bg-input/30 dark:hover:bg-input/50 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40";
// Radix's RadioGroupItem classes, with `data-checked` turned into `checked:`
// for a native input; the indicator dot is drawn with an inset shadow.
const RADIO: &str = "relative flex aspect-square size-4 shrink-0 appearance-none rounded-full border border-input outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:bg-input/30 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40 checked:border-primary checked:bg-primary-foreground checked:shadow-[inset_0_0_0_3px_var(--color-primary)] dark:checked:bg-primary-foreground";
// Radix's Checkbox classes, likewise on a native input; the lucide check is
// drawn over it while checked.
const CHECKBOX: &str = "peer relative flex size-4 shrink-0 appearance-none items-center justify-center rounded-[4px] border border-input transition-shadow outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-2 aria-invalid:ring-destructive/20 aria-invalid:checked:border-primary dark:bg-input/30 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40 checked:border-primary checked:bg-primary checked:text-primary-foreground dark:checked:bg-primary";
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
        <div class="grid gap-2">
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

/// A labelled native select bound to `value` (the option's `value`).
/// `hide_label` keeps the label for screen readers only.
#[component]
pub fn Select(
    label: &'static str,
    id: &'static str,
    #[prop(optional)] hide_label: bool,
    value: RwSignal<String>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="grid gap-2">
            <label for=id class=if hide_label { "sr-only" } else { LABEL }>{label}</label>
            <div class="relative">
                <select
                    id=id
                    name=id
                    class=SELECT
                    prop:value=move || value.get()
                    on:change=move |ev| value.set(event_target_value(&ev))
                >
                    {children()}
                </select>
                <svg
                    class="pointer-events-none absolute top-1/2 right-1.5 size-3.5 -translate-y-1/2 text-muted-foreground select-none"
                    xmlns="http://www.w3.org/2000/svg"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                >
                    <path d="m6 9 6 6 6-6" />
                </svg>
            </div>
        </div>
    }
}

/// One labelled radio button; `on_select` runs when it is chosen.
#[component]
pub fn Radio(
    label: &'static str,
    name: &'static str,
    #[prop(into)] checked: Signal<bool>,
    on_select: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <label class=LABEL>
            <input
                type="radio"
                name=name
                class=RADIO
                prop:checked=checked
                on:change=move |_| on_select()
            />
            {label}
        </label>
    }
}

/// A labelled checkbox bound to `checked`.
#[component]
pub fn Checkbox(label: &'static str, id: &'static str, checked: RwSignal<bool>) -> impl IntoView {
    view! {
        <label class=LABEL>
            <span class="relative flex size-4 shrink-0">
                <input
                    type="checkbox"
                    id=id
                    name=id
                    class=CHECKBOX
                    prop:checked=move || checked.get()
                    on:change=move |ev| checked.set(event_target_checked(&ev))
                />
                <svg
                    class="pointer-events-none absolute inset-0 m-auto hidden size-3.5 text-primary-foreground peer-checked:block"
                    xmlns="http://www.w3.org/2000/svg"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                >
                    <path d="M20 6 9 17l-5-5" />
                </svg>
            </span>
            {label}
        </label>
    }
}

// Table classes, as generated by `npx shadcn add table` for the preset.
const TABLE_CONTAINER: &str = "relative w-full overflow-x-auto";
const TABLE: &str = "w-full caption-bottom text-xs";
pub const TABLE_HEAD: &str = "[&_tr]:border-b";
pub const TABLE_BODY: &str = "[&_tr:last-child]:border-0";
pub const TABLE_ROW: &str = "border-b transition-colors hover:bg-muted/50 has-aria-expanded:bg-muted/50 data-[state=selected]:bg-muted";
pub const TABLE_HEADER_CELL: &str = "h-10 px-2 text-left align-middle font-medium whitespace-nowrap text-foreground [&:has([role=checkbox])]:pr-0";
pub const TABLE_CELL: &str = "p-2 align-middle whitespace-nowrap [&:has([role=checkbox])]:pr-0";

/// A table in the preset's style. Children are `<thead class=TABLE_HEAD>`
/// and `<tbody class=TABLE_BODY>` with `TABLE_ROW` rows and
/// `TABLE_HEADER_CELL`/`TABLE_CELL` cells.
#[component]
pub fn Table(children: Children) -> impl IntoView {
    view! {
        <div class=TABLE_CONTAINER>
            <table class=TABLE>{children()}</table>
        </div>
    }
}

/// An input without a visible label, for tables and line editors. `label`
/// is its accessible name.
#[component]
pub fn TextInput(
    #[prop(into)] label: String,
    value: RwSignal<String>,
    #[prop(default = "text")] kind: &'static str,
    #[prop(optional)] inputmode: &'static str,
    #[prop(optional)] list: &'static str,
) -> impl IntoView {
    view! {
        <input
            type=kind
            class=INPUT
            aria-label=label
            inputmode=(!inputmode.is_empty()).then_some(inputmode)
            list=(!list.is_empty()).then_some(list)
            bind:value=value
        />
    }
}
