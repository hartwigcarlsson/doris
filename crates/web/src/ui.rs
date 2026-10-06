//! Components in the style of shadcn preset b1Gdz9bFY (radix-mira). Class
//! lists are copied from the generated shadcn components; only what the app
//! uses is here.

use leptos::prelude::*;
use leptos_router::components::A;

const BUTTON: &str = "inline-flex shrink-0 items-center justify-center gap-1 rounded-md border border-transparent bg-clip-padding text-xs/relaxed font-medium whitespace-nowrap transition-all outline-none select-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30 active:translate-y-px disabled:pointer-events-none disabled:opacity-50 h-7 px-2 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg]:size-3.5";
const BUTTON_DEFAULT: &str = "bg-primary text-primary-foreground hover:bg-primary/80";
const BUTTON_GHOST: &str = "hover:bg-muted hover:text-foreground dark:hover:bg-muted/50";
const BUTTON_OUTLINE: &str = "border-border hover:bg-muted hover:text-foreground dark:bg-input/30";
const BADGE: &str = "inline-flex h-5 w-fit shrink-0 items-center justify-center gap-1 overflow-hidden rounded-full border border-transparent px-2 py-0.5 text-[0.625rem] font-medium whitespace-nowrap";
/// A form card's width in the preset: 352px.
pub const NARROW: &str = "w-full max-w-[22rem]";
pub const INPUT: &str = "h-7 w-full min-w-0 rounded-md border border-input bg-input/20 px-2 py-0.5 text-sm transition-colors outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30 disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 read-only:cursor-default read-only:border-dashed read-only:focus-visible:border-input read-only:bg-muted read-only:text-muted-foreground md:text-xs/relaxed dark:bg-input/30";
const INPUT_FILE: &str = "file:inline-flex file:h-6 file:border-0 file:bg-transparent file:text-xs/relaxed file:font-medium file:text-foreground";
const LABEL: &str = "flex items-center gap-2 text-xs/relaxed leading-none font-medium select-none";
/// shadcn NativeSelectOption: keeps the dropdown readable in dark mode.
pub const SELECT_OPTION: &str = "bg-[Canvas] text-[CanvasText]";
pub const SELECT: &str = "h-7 w-full min-w-0 appearance-none rounded-md border border-input bg-input/20 py-0.5 pr-6 pl-2 text-xs/relaxed transition-colors outline-none select-none selection:bg-primary selection:text-primary-foreground placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30 disabled:pointer-events-none disabled:cursor-not-allowed aria-invalid:border-destructive aria-invalid:ring-2 aria-invalid:ring-destructive/20 dark:bg-input/30 dark:hover:bg-input/50 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40";
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
    Outline,
}

impl Variant {
    fn class(self) -> &'static str {
        match self {
            Variant::Default => BUTTON_DEFAULT,
            Variant::Ghost => BUTTON_GHOST,
            Variant::Outline => BUTTON_OUTLINE,
        }
    }
}

#[component]
pub fn Button(
    #[prop(optional)] variant: Variant,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(default = "submit")] kind: &'static str,
    children: Children,
) -> impl IntoView {
    view! {
        <button type=kind class=format!("{BUTTON} {}", variant.class()) disabled=disabled>
            {children()}
        </button>
    }
}

/// A link that looks like a button, for "Ny …" actions.
#[component]
pub fn LinkButton(
    #[prop(into)] href: String,
    #[prop(optional)] variant: Variant,
    #[prop(optional)] icon: Option<IconName>,
    children: Children,
) -> impl IntoView {
    view! {
        <A href=href attr:class=format!("{BUTTON} {}", variant.class())>
            {icon.map(|name| view! { <Icon name=name /> })}
            {children()}
        </A>
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
    /// The preset's form width, left-aligned.
    #[prop(optional)]
    narrow: bool,
    /// The title is the page's `<h1>`: for pages without a `PageHeader`.
    #[prop(optional)]
    page_title: bool,
    children: Children,
) -> impl IntoView {
    let class = if narrow {
        format!("{CARD} {NARROW}")
    } else {
        CARD.to_owned()
    };
    view! {
        <section class=class>
            <header class="grid gap-1 px-4">
                {if page_title {
                    view! { <h1 class="text-sm font-medium">{title}</h1> }.into_any()
                } else {
                    view! { <h2 class="text-sm font-medium">{title}</h2> }.into_any()
                }}
                {(!description.is_empty())
                    .then(|| view! { <p class="text-xs/relaxed text-muted-foreground">{description}</p> })}
            </header>
            <div class="px-4">{children()}</div>
        </section>
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub enum BadgeVariant {
    #[default]
    Secondary,
    Outline,
    Destructive,
}

impl BadgeVariant {
    fn class(self) -> &'static str {
        match self {
            BadgeVariant::Secondary => "bg-secondary text-secondary-foreground",
            BadgeVariant::Outline => "border-border text-muted-foreground",
            BadgeVariant::Destructive => {
                "bg-destructive/10 text-destructive dark:bg-destructive/20"
            }
        }
    }
}

/// A status label. The text carries the meaning; the colour only helps.
#[component]
pub fn Badge(#[prop(optional)] variant: BadgeVariant, children: Children) -> impl IntoView {
    view! { <span class=format!("{BADGE} {}", variant.class())>{children()}</span> }
}

/// The page's one `<h1>`, an optional line under it, and actions to the right.
#[component]
pub fn PageHeader(
    #[prop(into)] title: Signal<String>,
    #[prop(optional, into)] description: Signal<String>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <div class="flex flex-wrap items-end justify-between gap-4">
            <div>
                <h1 class="text-sm font-medium">{move || title.get()}</h1>
                <Show when=move || !description.get().is_empty()>
                    <p class="text-xs/relaxed text-muted-foreground">{move || description.get()}</p>
                </Show>
            </div>
            {children.map(|actions| view! { <div class="flex flex-wrap items-center gap-2">{actions()}</div> })}
        </div>
    }
}

/// A plain card for a wide form or a list. `class` adds to it, e.g. a width.
#[component]
pub fn Panel(#[prop(optional)] class: &'static str, children: Children) -> impl IntoView {
    view! {
        <section class=format!("overflow-x-auto rounded-lg bg-card p-4 text-xs/relaxed text-card-foreground ring-1 ring-foreground/10 {class}")>
            {children()}
        </section>
    }
}

/// A card around a `Table`, with an optional row above it for a search
/// field and filters.
#[component]
pub fn TableCard(
    #[prop(optional, into)] toolbar: Option<ViewFn>,
    children: Children,
) -> impl IntoView {
    view! {
        <section class="overflow-hidden rounded-lg bg-card px-2 py-2 text-xs/relaxed text-card-foreground ring-1 ring-foreground/10">
            {toolbar.map(|toolbar| view! { <div class="flex flex-wrap items-center gap-4 px-2 pt-2 pb-1">{toolbar.run()}</div> })}
            {children()}
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
                <Icon name=IconName::ChevronDown class="pointer-events-none absolute top-1/2 right-1.5 size-3.5 -translate-y-1/2 text-muted-foreground select-none" />
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
pub fn Checkbox(
    #[prop(into)] label: String,
    #[prop(into)] id: String,
    checked: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <label class=LABEL>
            <span class="relative flex size-4 shrink-0">
                <input
                    type="checkbox"
                    id=id.clone()
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
/// `TABLE_CELL` for amounts: right-aligned with tabular digits.
pub const TABLE_AMOUNT_CELL: &str =
    "p-2 align-middle whitespace-nowrap [&:has([role=checkbox])]:pr-0 text-right tabular-nums";

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
    #[prop(optional, into)] placeholder: String,
) -> impl IntoView {
    view! {
        <input
            type=kind
            class=INPUT
            aria-label=label
            placeholder=(!placeholder.is_empty()).then_some(placeholder)
            inputmode=(!inputmode.is_empty()).then_some(inputmode)
            list=(!list.is_empty()).then_some(list)
            bind:value=value
        />
    }
}

/// A labelled native file picker for underlag (PDF, JPEG, PNG). On mobile
/// it offers the camera.
#[component]
pub fn FileInput(
    #[prop(into)] label: String,
    #[prop(into)] id: String,
    on_pick: impl Fn(web_sys::HtmlInputElement) + 'static,
) -> impl IntoView {
    view! {
        <div class="grid gap-2">
            <label for=id.clone() class=LABEL>
                {label}
            </label>
            <input
                id=id
                type="file"
                multiple
                accept="application/pdf,image/jpeg,image/png"
                class=format!("{INPUT} {INPUT_FILE}")
                on:change=move |ev| on_pick(event_target::<web_sys::HtmlInputElement>(&ev))
            />
        </div>
    }
}

/// The lucide icons the app uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconName {
    ReceiptText,
    Scale,
    ChartColumn,
    ListTree,
    CalendarRange,
    FileText,
    Building2,
    Banknote,
    Users,
    Building,
    KeyRound,
    MailPlus,
    LogOut,
    Contact,
    Landmark,
    CircleAlert,
    Clock,
    Search,
    Plus,
    ChevronDown,
    ChevronRight,
    Paperclip,
}

impl IconName {
    #[cfg(test)]
    const ALL: [IconName; 22] = [
        IconName::ReceiptText,
        IconName::Scale,
        IconName::ChartColumn,
        IconName::ListTree,
        IconName::CalendarRange,
        IconName::FileText,
        IconName::Building2,
        IconName::Banknote,
        IconName::Users,
        IconName::Building,
        IconName::KeyRound,
        IconName::MailPlus,
        IconName::LogOut,
        IconName::Contact,
        IconName::Landmark,
        IconName::CircleAlert,
        IconName::Clock,
        IconName::Search,
        IconName::Plus,
        IconName::ChevronDown,
        IconName::ChevronRight,
        IconName::Paperclip,
    ];

    /// Shapes from lucide-static 1.52.0, with closing tags written out.
    fn shapes(self) -> &'static str {
        match self {
            IconName::ReceiptText => {
                r#"<path d="M13 16H8"></path><path d="M14 8H8"></path><path d="M16 12H8"></path><path d="M4 3a1 1 0 0 1 1-1 1.3 1.3 0 0 1 .7.2l.933.6a1.3 1.3 0 0 0 1.4 0l.934-.6a1.3 1.3 0 0 1 1.4 0l.933.6a1.3 1.3 0 0 0 1.4 0l.933-.6a1.3 1.3 0 0 1 1.4 0l.934.6a1.3 1.3 0 0 0 1.4 0l.933-.6A1.3 1.3 0 0 1 19 2a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1 1.3 1.3 0 0 1-.7-.2l-.933-.6a1.3 1.3 0 0 0-1.4 0l-.934.6a1.3 1.3 0 0 1-1.4 0l-.933-.6a1.3 1.3 0 0 0-1.4 0l-.933.6a1.3 1.3 0 0 1-1.4 0l-.934-.6a1.3 1.3 0 0 0-1.4 0l-.933.6a1.3 1.3 0 0 1-.7.2 1 1 0 0 1-1-1z"></path>"#
            }
            IconName::Scale => {
                r#"<path d="M12 3v18"></path><path d="m19 8 3 8a5 5 0 0 1-6 0zV7"></path><path d="M3 7h1a17 17 0 0 0 8-2 17 17 0 0 0 8 2h1"></path><path d="m5 8 3 8a5 5 0 0 1-6 0zV7"></path><path d="M7 21h10"></path>"#
            }
            IconName::ChartColumn => {
                r#"<path d="M3 3v16a2 2 0 0 0 2 2h16"></path><path d="M18 17V9"></path><path d="M13 17V5"></path><path d="M8 17v-3"></path>"#
            }
            IconName::ListTree => {
                r#"<path d="M8 5h13"></path><path d="M13 12h8"></path><path d="M13 19h8"></path><path d="M3 10a2 2 0 0 0 2 2h3"></path><path d="M3 5v12a2 2 0 0 0 2 2h3"></path>"#
            }
            IconName::CalendarRange => {
                r#"<rect x="3" y="3" width="18" height="18" rx="2"></rect><path d="M16 2v3"></path><path d="M3 9h18"></path><path d="M8 2v3"></path><path d="M17 13h-6"></path><path d="M13 17H7"></path><path d="M7 13h.01"></path><path d="M17 17h.01"></path>"#
            }
            IconName::FileText => {
                r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 9H8"></path><path d="M16 13H8"></path><path d="M16 17H8"></path>"#
            }
            IconName::Building2 => {
                r#"<path d="M10 12h4"></path><path d="M10 8h4"></path><path d="M14 21v-3a2 2 0 0 0-4 0v3"></path><path d="M6 10H4a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-2"></path><path d="M6 21V5a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v16"></path>"#
            }
            IconName::Banknote => {
                r#"<rect width="20" height="12" x="2" y="6" rx="2"></rect><circle cx="12" cy="12" r="2"></circle><path d="M6 12h.01M18 12h.01"></path>"#
            }
            IconName::Users => {
                r#"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"></path><path d="M16 3.128a4 4 0 0 1 0 7.744"></path><path d="M22 21v-2a4 4 0 0 0-3-3.87"></path><circle cx="9" cy="7" r="4"></circle>"#
            }
            IconName::Building => {
                r#"<path d="M12 10h.01"></path><path d="M12 14h.01"></path><path d="M12 6h.01"></path><path d="M16 10h.01"></path><path d="M16 14h.01"></path><path d="M16 6h.01"></path><path d="M8 10h.01"></path><path d="M8 14h.01"></path><path d="M8 6h.01"></path><path d="M9 22v-3a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v3"></path><rect x="4" y="2" width="16" height="20" rx="2"></rect>"#
            }
            IconName::KeyRound => {
                r#"<path d="M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z"></path><circle cx="16.5" cy="7.5" r=".5" fill="currentColor"></circle>"#
            }
            IconName::MailPlus => {
                r#"<path d="M22 13V6a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v12c0 1.1.9 2 2 2h8"></path><path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"></path><path d="M19 16v6"></path><path d="M16 19h6"></path>"#
            }
            IconName::LogOut => {
                r#"<path d="m16 17 5-5-5-5"></path><path d="M21 12H9"></path><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"></path>"#
            }
            IconName::Contact => {
                r#"<path d="M16 2v2"></path><path d="M7 21v-2a2 2 0 012-2h6a2 2 0 012 2v2"></path><path d="M8 2v2"></path><circle cx="12" cy="10" r="3"></circle><rect x="3" y="3" width="18" height="18" rx="2"></rect>"#
            }
            IconName::Landmark => {
                r#"<path d="M10 18v-7"></path><path d="M11.119 2.205a2 2 0 0 1 1.762 0l7.84 3.846A.5.5 0 0 1 20.5 7h-17a.5.5 0 0 1-.22-.949z"></path><path d="M14 18v-7"></path><path d="M18 18v-7"></path><path d="M3 22h18"></path><path d="M6 18v-7"></path>"#
            }
            IconName::CircleAlert => {
                r#"<circle cx="12" cy="12" r="10"></circle><line x1="12" x2="12" y1="8" y2="12"></line><line x1="12" x2="12.01" y1="16" y2="16"></line>"#
            }
            IconName::Clock => {
                r#"<circle cx="12" cy="12" r="10"></circle><path d="M12 6v6l4 2"></path>"#
            }
            IconName::Search => {
                r#"<path d="m21 21-4.34-4.34"></path><circle cx="11" cy="11" r="8"></circle>"#
            }
            IconName::Plus => r#"<path d="M5 12h14"></path><path d="M12 5v14"></path>"#,
            IconName::ChevronDown => r#"<path d="m6 9 6 6 6-6"></path>"#,
            IconName::ChevronRight => r#"<path d="m9 18 6-6-6-6"></path>"#,
            IconName::Paperclip => {
                r#"<path d="m16 6-8.414 8.586a2 2 0 0 0 2.829 2.829l8.414-8.586a4 4 0 1 0-5.657-5.657l-8.379 8.551a6 6 0 1 0 8.485 8.485l8.379-8.551"></path>"#
            }
        }
    }
}

/// An inlined lucide icon. Decorative: the text next to it names the thing.
#[component]
pub fn Icon(name: IconName, #[prop(default = "size-3.5")] class: &'static str) -> impl IntoView {
    view! {
        <svg
            class=format!("shrink-0 {class}")
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
            inner_html=name.shapes()
        />
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_shapes() {
        for name in IconName::ALL {
            let shapes = name.shapes();
            assert!(shapes.starts_with('<'), "{name:?}");
            // Leptos' `inner_html` takes the markup as is: no self-closing tags.
            assert!(!shapes.contains("/>"), "{name:?}");
        }
    }

    #[test]
    fn badge_variants_have_distinct_looks() {
        let looks = [
            BadgeVariant::Secondary.class(),
            BadgeVariant::Outline.class(),
            BadgeVariant::Destructive.class(),
        ];
        assert_ne!(looks[0], looks[1]);
        assert_ne!(looks[1], looks[2]);
        assert!(looks[2].contains("text-destructive"));
    }

    #[test]
    fn the_outline_button_has_a_border() {
        assert!(Variant::Outline.class().contains("border-border"));
        assert!(Variant::Default.class().contains("bg-primary"));
    }
}
