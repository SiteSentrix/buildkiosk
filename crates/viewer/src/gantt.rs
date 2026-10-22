//! Dragging the bars of the schedule chart: grab a bar to move the task, grab the handle at its end to change
//! how long it is. The module only works out how many days the pointer moved and posts the page's own form
//! (`schedule.move`), so the engine decides what the new dates are (working days, holidays, weekends).

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlFormElement, HtmlInputElement, PointerEvent};

use crate::dom::{listen, ok};
use crate::words::Words;

/// The most days one drag may move a task by (the command's limit).
const MAX_DAYS: i64 = 3_650;
/// The chart is drawn 600 units wide whatever its size on screen.
const VIEWBOX: f64 = 600.0;

/// How many whole days a pointer move of `dx` pixels is, on a chart `width_px` wide that spans `days` days.
pub fn days_moved(dx: f64, width_px: f64, days: f64) -> i64 {
    if !(dx.is_finite() && width_px.is_finite() && days.is_finite()) || width_px <= 0.0 || days <= 0.0 {
        return 0;
    }
    ((dx / (width_px / days)).round() as i64).clamp(-MAX_DAYS, MAX_DAYS)
}

/// What a drag of `days` would do, for the hint (in English).
pub fn describe(days: i64, resize: bool) -> String {
    describe_in(&Words::default(), days, resize)
}

/// What a drag of `days` would do, in the page's language.
pub fn describe_in(words: &Words, days: i64, resize: bool) -> String {
    let n = days.unsigned_abs();
    let unit = words.t(if n == 1 { "day" } else { "days" });
    let n = n.to_string();
    match (days, resize) {
        (0, _) => words.t("Drag to move the task, or drag its end to change its length."),
        (d, false) if d > 0 => words.tf("Move {} {} later. Let go to save.", &[&n, &unit]),
        (_, false) => words.tf("Move {} {} earlier. Let go to save.", &[&n, &unit]),
        (d, true) if d > 0 => words.tf("Finish {} {} later. Let go to save.", &[&n, &unit]),
        (_, true) => words.tf("Finish {} {} earlier. Let go to save.", &[&n, &unit]),
    }
}

struct Drag {
    id: String,
    resize: bool,
    /// Dragging a link from this task's dot to another bar, rather than moving or resizing.
    link: bool,
    from_x: f64,
    svg_width: f64,
    days: i64,
    /// The elements being moved, with the attribute to put back.
    parts: Vec<(Element, Option<String>)>,
    pointer: i32,
}

fn hint(document: &Document, text: &str) {
    if let Some(h) = document.get_element_by_id("gantt-hint") {
        h.set_text_content(Some(text));
    }
}

/// The task whose bar is under the pointer, if any.
fn task_at(document: &Document, e: &PointerEvent) -> Option<String> {
    let el = document.element_from_point(e.client_x() as f32, e.client_y() as f32)?;
    el.closest("[data-task]").ok().flatten()?.get_attribute("data-task")
}

fn input(document: &Document, id: &str) -> Option<HtmlInputElement> {
    document.get_element_by_id(id)?.dyn_into().ok()
}

pub fn start(document: Document, root: Element) {
    let total: f64 = root.get_attribute("data-days").and_then(|d| d.parse().ok()).unwrap_or(0.0);
    let drag: Rc<RefCell<Option<Drag>>> = Rc::new(RefCell::new(None));
    let lang = document.document_element().and_then(|e| e.get_attribute("lang")).unwrap_or_default();
    let words = Rc::new(Words::for_language(&lang));
    hint(&document, &describe_in(&words, 0, false));

    let (d, state, w) = (document.clone(), drag.clone(), words.clone());
    listen(&root, "pointerdown", move |e: PointerEvent| {
        let Some(target) = e.target().and_then(|t| t.dyn_into::<Element>().ok()) else { return };
        if let Ok(Some(dot)) = target.closest("[data-link]") {
            let Some(id) = dot.get_attribute("data-link") else { return };
            ok(dot.set_pointer_capture(e.pointer_id()));
            e.prevent_default();
            *state.borrow_mut() =
                Some(Drag { id, resize: false, link: true, from_x: 0.0, svg_width: 0.0, days: 0, parts: Vec::new(), pointer: e.pointer_id() });
            hint(&d, &w.t("Drop on another bar to make that task follow this one."));
            return;
        }
        let Ok(Some(grabbed)) = target.closest("[data-task]") else { return };
        let Ok(Some(svg)) = grabbed.closest("svg") else { return };
        let Some(id) = grabbed.get_attribute("data-task") else { return };
        let resize = grabbed.get_attribute("data-edge").is_some();
        let mut parts = Vec::new();
        // A move takes everything of the task in this row along; a resize stretches the bar only.
        if let Ok(list) = svg.query_selector_all(&format!("[data-task=\"{id}\"]")) {
            for i in 0..list.length() {
                if let Some(el) = list.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
                    let is_bar = el.class_list().contains("bar");
                    let is_handle = el.get_attribute("data-edge").is_some();
                    if !resize || is_bar || is_handle {
                        let old = el.get_attribute(if resize && is_bar { "width" } else { "transform" });
                        parts.push((el, old));
                    }
                }
            }
        }
        ok(grabbed.set_pointer_capture(e.pointer_id()));
        e.prevent_default();
        *state.borrow_mut() = Some(Drag {
            id,
            resize,
            link: false,
            from_x: f64::from(e.client_x()),
            svg_width: svg.get_bounding_client_rect().width(),
            days: 0,
            parts,
            pointer: e.pointer_id(),
        });
        hint(&d, &describe_in(&w, 0, resize));
    });

    let (d, state, w) = (document.clone(), drag.clone(), words.clone());
    listen(&root, "pointermove", move |e: PointerEvent| {
        let mut slot = state.borrow_mut();
        let Some(g) = slot.as_mut().filter(|g| g.pointer == e.pointer_id()) else { return };
        if g.link {
            let over = task_at(&d, &e);
            let text = match over {
                Some(t) if t != g.id => w.t("Let go to link: that task will follow this one."),
                _ => w.t("Drop on another bar to make that task follow this one."),
            };
            hint(&d, &text);
            return;
        }
        let dx = f64::from(e.client_x()) - g.from_x;
        g.days = days_moved(dx, g.svg_width, total);
        // The same move in the chart's own units, so it matches what the days will be only roughly until saved.
        let units = if g.svg_width > 0.0 { dx * VIEWBOX / g.svg_width } else { 0.0 };
        for (el, old) in &g.parts {
            if g.resize && el.class_list().contains("bar") {
                let w = old.as_deref().and_then(|w| w.parse::<f64>().ok()).unwrap_or(0.0);
                ok(el.set_attribute("width", &format!("{:.1}", (w + units).max(3.0))));
            } else {
                ok(el.set_attribute("transform", &format!("translate({units:.1} 0)")));
            }
        }
        hint(&d, &describe_in(&w, g.days, g.resize));
    });

    for name in ["pointerup", "pointercancel"] {
        let (d, state, w) = (document.clone(), drag.clone(), words.clone());
        let save = name == "pointerup";
        listen(&root, name, move |e: PointerEvent| {
            let Some(g) = state.borrow_mut().take_if(|g| g.pointer == e.pointer_id()) else { return };
            if g.link {
                let target = if save { task_at(&d, &e) } else { None };
                let form = d
                    .get_element_by_id("gantt-link")
                    .and_then(|c| c.query_selector("form").ok().flatten())
                    .and_then(|f| f.dyn_into::<HtmlFormElement>().ok());
                match (target, input(&d, "gl-id"), input(&d, "gl-from"), form) {
                    (Some(next), Some(id), Some(from), Some(form)) if next != g.id => {
                        id.set_value(&next);
                        from.set_value(&g.id);
                        hint(&d, &w.t("Saving…"));
                        ok(form.submit());
                    }
                    _ => hint(&d, &describe_in(&w, 0, false)),
                }
                return;
            }
            // Put the picture back; the page shows the new dates once the server has them.
            for (el, old) in &g.parts {
                let attr = if g.resize && el.class_list().contains("bar") { "width" } else { "transform" };
                match old {
                    Some(v) => ok(el.set_attribute(attr, v)),
                    None => ok(el.remove_attribute(attr)),
                }
            }
            if !save || g.days == 0 {
                hint(&d, &describe_in(&w, 0, false));
                return;
            }
            let (shift, other) = if g.resize { ("gm-fshift", "gm-shift") } else { ("gm-shift", "gm-fshift") };
            if let (Some(id), Some(on), Some(off)) = (input(&d, "gm-id"), input(&d, shift), input(&d, other)) {
                id.set_value(&g.id);
                on.set_value(&g.days.to_string());
                on.set_disabled(false);
                off.set_disabled(true);
                if let Some(form) = d
                    .get_element_by_id("gantt-move")
                    .and_then(|c| c.query_selector("form").ok().flatten())
                    .and_then(|f| f.dyn_into::<HtmlFormElement>().ok())
                {
                    hint(&d, &w.t("Saving…"));
                    ok(form.submit());
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_moves_become_whole_days() {
        // 600 px wide chart over 60 days: 10 px a day.
        assert_eq!(days_moved(0.0, 600.0, 60.0), 0);
        assert_eq!(days_moved(34.0, 600.0, 60.0), 3);
        assert_eq!(days_moved(36.0, 600.0, 60.0), 4);
        assert_eq!(days_moved(-26.0, 600.0, 60.0), -3);
        assert_eq!(days_moved(1e12, 600.0, 60.0), MAX_DAYS);
        for (dx, w, d) in [(f64::NAN, 600.0, 60.0), (10.0, 0.0, 60.0), (10.0, 600.0, 0.0), (10.0, f64::INFINITY, 60.0), (10.0, 600.0, f64::NAN)] {
            assert_eq!(days_moved(dx, w, d), 0);
        }
    }

    #[test]
    fn the_hint_says_what_will_happen() {
        assert!(describe(3, false).starts_with("Move 3 days later"));
        assert!(describe(-1, false).starts_with("Move 1 day earlier"));
        assert!(describe(2, true).starts_with("Finish 2 days later"));
        assert!(describe(0, true).starts_with("Drag"));
        // In another language, with the unit's name and the number put where that language wants them.
        let nl = Words::for_language("nl");
        assert_eq!(describe_in(&nl, 3, false), "3 dagen later verplaatsen. Laat los om op te slaan.");
        assert_eq!(describe_in(&nl, -1, true), "Einde 1 dag eerder. Laat los om op te slaan.");
        assert_eq!(describe_in(&Words::for_language("es"), 2, true), "Terminar 2 días más tarde. Suelte para guardar.");
    }
}
