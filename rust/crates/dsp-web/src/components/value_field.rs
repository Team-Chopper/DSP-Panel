//! Inline number field: shows the current value and can be typed into directly
//! (Enter or leaving the field applies, Escape reverts).

use leptos::html;
use leptos::prelude::*;
use web_sys::KeyboardEvent;

#[component]
pub fn ValueField(
    /// Formatted current value.
    #[prop(into)]
    text: Signal<String>,
    /// Called with the typed text; returns false if it was not accepted.
    on_submit: Callback<String, bool>,
    #[prop(optional)] width: Option<u32>,
    #[prop(default = 26)] height: u32,
    #[prop(default = 11.0)] font: f64,
    #[prop(into, default = "var(--text)".to_string())] color: String,
    #[prop(into, default = "center".to_string())] align: String,
    #[prop(optional, into)] title: Option<String>,
) -> impl IntoView {
    let input = NodeRef::<html::Input>::new();
    let focused = RwSignal::new(false);
    let invalid = RwSignal::new(false);

    // Follow the device while the user is not typing.
    Effect::new(move |_| {
        let t = text.get();
        if let Some(el) = input.get()
            && !focused.get_untracked() && el.value() != t {
                el.set_value(&t);
            }
    });

    let apply = move || {
        let Some(el) = input.get_untracked() else { return };
        let typed = el.value().trim().to_string();
        let current = text.get_untracked();
        if typed == current || typed.is_empty() {
            el.set_value(&current);
            invalid.set(false);
            return;
        }
        let ok = on_submit.run(typed);
        invalid.set(!ok);
        if !ok {
            el.set_value(&current);
        }
    };

    let on_key = move |ev: KeyboardEvent| match ev.key().as_str() {
        "Enter" => {
            if let Some(el) = input.get_untracked() {
                let _ = el.blur();
            }
        }
        "Escape" => {
            if let Some(el) = input.get_untracked() {
                el.set_value(&text.get_untracked());
                invalid.set(false);
                let _ = el.blur();
            }
        }
        _ => {}
    };

    let style = format!(
        "{}height:{height}px;font-size:{font}px;color:{color};text-align:{align}",
        width.map(|w| format!("width:{w}px;")).unwrap_or_default(),
    );
    view! {
        <input
            class="value-field"
            class:invalid=move || invalid.get()
            node_ref=input
            type="text"
            inputmode="decimal"
            spellcheck="false"
            autocomplete="off"
            style=style
            title=title
            prop:value=move || text.get_untracked()
            on:focus=move |_| {
                focused.set(true);
                if let Some(el) = input.get_untracked() {
                    el.select();
                }
            }
            on:blur=move |_| {
                focused.set(false);
                apply();
            }
            on:keydown=on_key
        />
    }
}
