//! Size of an element as a signal (ResizeObserver), for graphs that draw in
//! real pixels.

use leptos::html::ElementType;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{ResizeObserver, ResizeObserverEntry};

/// (width, height) in CSS pixels of the element behind [node].
pub fn use_size<E>(node: NodeRef<E>) -> RwSignal<(f64, f64)>
where
    E: ElementType,
    E::Output: JsCast + Clone + 'static,
{
    let size = RwSignal::new((0.0, 0.0));
    let observer = StoredValue::new_local(None::<(ResizeObserver, Closure<dyn FnMut(js_sys::Array)>)>);
    Effect::new(move |_| {
        let Some(el) = node.get() else { return };
        let element: web_sys::Element = el.unchecked_into();
        let rect = element.get_bounding_client_rect();
        size.set((rect.width(), rect.height()));
        let callback = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
            if let Some(entry) = entries.get(0).dyn_ref::<ResizeObserverEntry>() {
                let r = entry.content_rect();
                let next = (r.width(), r.height());
                if size.get_untracked() != next {
                    size.set(next);
                }
            }
        });
        if let Ok(ro) = ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            ro.observe(&element);
            observer.set_value(Some((ro, callback)));
        }
    });
    on_cleanup(move || {
        observer.update_value(|o| {
            if let Some((ro, _)) = o.take() {
                ro.disconnect();
            }
        });
    });
    size
}
