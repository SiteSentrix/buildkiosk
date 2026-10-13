//! The browser side: fetches the sheet, draws it on a canvas and handles the mouse, touch and keys.
//!
//! Errors from the canvas calls are ignored on purpose (`ok`): a failed draw call leaves the last
//! picture on screen, and there is nothing better to do with it.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    CanvasRenderingContext2d, Document, Element, Event, EventTarget, HtmlCanvasElement, HtmlElement, HtmlInputElement, ImageData, KeyboardEvent,
    PointerEvent, Response, WheelEvent, Window,
};

use crate::geom::{self, Camera, Stage};
use crate::shapes::{self, Mark, Shape};
use crate::sheet::{MAX_FILE, Sheet};

const RED: &str = "#c62828";
const GREY: &str = "#7b8794";
const BLUE: &str = "#1565c0";
/// Pointer movement under this many pixels is still a click.
const CLICK_SLOP: f64 = 4.0;
/// How long after the last zoom or pan the page is drawn again, sharper.
const SHARPEN_MS: i32 = 180;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Move,
    Pin,
    Box,
    Arrow,
    Text,
}

impl Tool {
    fn from_key(key: &str) -> Option<Tool> {
        Some(match key {
            "move" => Tool::Move,
            "pin" => Tool::Pin,
            "rect" => Tool::Box,
            "arrow" => Tool::Arrow,
            "text" => Tool::Text,
            _ => return None,
        })
    }

    /// The markup kind the engine knows it as.
    fn kind(self) -> &'static str {
        match self {
            Tool::Move => "",
            Tool::Pin => "pin",
            Tool::Box => "rect",
            Tool::Arrow => "arrow",
            Tool::Text => "text",
        }
    }

    fn two_points(self) -> bool {
        matches!(self, Tool::Box | Tool::Arrow)
    }

    fn hint(self, started: bool) -> &'static str {
        match (self, started) {
            (Tool::Move, _) => "Drag to move the sheet. Scroll or pinch to zoom.",
            (Tool::Pin, _) => "Click the sheet where the pin goes.",
            (Tool::Text, _) => "Click where the words go.",
            (Tool::Box, false) => "Click one corner of the box.",
            (Tool::Box, true) => "Click the opposite corner.",
            (Tool::Arrow, false) => "Click where the arrow starts.",
            (Tool::Arrow, true) => "Click where the arrow points.",
        }
    }
}

struct Finger {
    id: i32,
    x: f64,
    y: f64,
}

struct Press {
    x: f64,
    y: f64,
    moved: bool,
}

struct Viewer {
    window: Window,
    document: Document,
    root: Element,
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    /// The page as drawn by the renderer, kept off screen and scaled onto the canvas.
    picture: HtmlCanvasElement,
    picture_w: u32,
    sheet: Sheet,
    page: usize,
    stage: Stage,
    cam: Camera,
    ratio: f64,
    marks: Vec<Mark>,
    can_mark: bool,
    tool: Tool,
    /// The first corner of a box or arrow, as fractions of the page.
    first: Option<(f64, f64)>,
    /// Where the pointer is, in the window (to show the shape being drawn).
    hover: Option<(f64, f64)>,
    fingers: Vec<Finger>,
    press: Option<Press>,
    timer: Option<i32>,
    sharpen: Option<Closure<dyn FnMut()>>,
}

type Shared = Rc<RefCell<Viewer>>;

/// Runs `f` on the viewer unless it is already in use (events can't nest, but a panic here would
/// stop the whole page).
fn with(shared: &Shared, f: impl FnOnce(&mut Viewer)) {
    if let Ok(mut v) = shared.try_borrow_mut() {
        f(&mut v);
    }
}

fn ok<T>(r: Result<T, JsValue>) {
    let _ = r;
}

/// For a `dyn_into` that found a different kind of element than the page promised.
fn wrong<T>(_: T) -> String {
    "The page is not what the viewer expects.".to_string()
}

fn js(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| "The browser refused.".to_string())
}

fn listen<E: JsCast + 'static>(target: &EventTarget, name: &str, mut f: impl FnMut(E) + 'static) {
    let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
        if let Ok(e) = e.dyn_into::<E>() {
            f(e);
        }
    });
    ok(target.add_event_listener_with_callback(name, cb.as_ref().unchecked_ref()));
    // The listener lives as long as the page does.
    cb.forget();
}

pub fn start() {
    std::panic::set_hook(Box::new(|info| web_sys::console::error_1(&JsValue::from_str(&info.to_string()))));
    let Some(window) = web_sys::window() else { return };
    let Some(document) = window.document() else { return };
    let Ok(Some(root)) = document.query_selector("#wasm-viewer") else { return };
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = run(window, document, root).await {
            // The page keeps its server-drawn viewer.
            web_sys::console::warn_1(&JsValue::from_str(&format!("Sheet viewer not used: {e}")));
        }
    });
}

async fn fetch_bytes(window: &Window, url: &str) -> Result<Vec<u8>, String> {
    let resp: Response = JsFuture::from(window.fetch_with_str(url)).await.map_err(js)?.dyn_into().map_err(wrong)?;
    if !resp.ok() {
        return Err(format!("The server answered {}.", resp.status()));
    }
    let buf = JsFuture::from(resp.array_buffer().map_err(js)?).await.map_err(js)?;
    let bytes = js_sys::Uint8Array::new(&buf);
    if bytes.length() as usize > MAX_FILE {
        return Err("This file is too big to open here.".into());
    }
    Ok(bytes.to_vec())
}

async fn run(window: Window, document: Document, root: Element) -> Result<(), String> {
    let attr = |name: &str| root.get_attribute(name);
    let url = attr("data-file").ok_or("The page names no file.")?;
    let page = attr("data-page").and_then(|p| p.trim().parse::<usize>().ok()).unwrap_or(1).max(1) - 1;
    let marks = shapes::parse(&attr("data-marks").unwrap_or_default());
    let can_mark = attr("data-can-mark").as_deref() == Some("1");

    let sheet = Sheet::open(fetch_bytes(&window, &url).await?)?;
    let aspect = sheet.aspect(page).ok_or("This page doesn't exist.")?;

    let canvas: HtmlCanvasElement = root.query_selector("canvas").map_err(js)?.ok_or("The page has no canvas.")?.dyn_into().map_err(wrong)?;
    let ctx: CanvasRenderingContext2d = canvas.get_context("2d").map_err(js)?.ok_or("No 2D canvas.")?.dyn_into().map_err(wrong)?;
    let picture: HtmlCanvasElement = document.create_element("canvas").map_err(js)?.dyn_into().map_err(wrong)?;
    let body: HtmlElement = document.body().ok_or("The page has no body.")?;

    // The canvas has no size until it is shown.
    ok(body.class_list().add_1("wasm-viewer-on"));
    let mut v = Viewer {
        window: window.clone(),
        document: document.clone(),
        root: root.clone(),
        canvas,
        ctx,
        picture,
        picture_w: 0,
        sheet,
        page,
        stage: Stage { w: 1.0, h: 1.0 },
        cam: Camera::fit(Stage { w: 1.0, h: 1.0 }, aspect),
        ratio: 1.0,
        marks,
        can_mark,
        tool: Tool::Move,
        first: None,
        hover: None,
        fingers: Vec::new(),
        press: None,
        timer: None,
        sharpen: None,
    };
    measure(&mut v);
    v.cam = Camera::fit(v.stage, aspect);
    let width = geom::render_width(&v.cam, v.ratio);
    if let Err(e) = make_picture(&mut v, width) {
        ok(body.class_list().remove_1("wasm-viewer-on"));
        return Err(e);
    }
    paint(&v);
    set_tool(&mut v, Tool::Move);

    let shared: Shared = Rc::new(RefCell::new(v));
    let again = shared.clone();
    let cb = Closure::<dyn FnMut()>::new(move || with(&again, sharpen));
    with(&shared, |v| v.sharpen = Some(cb));
    wire(&shared, aspect);
    Ok(())
}

/// Sizes the canvas to the window it sits in.
fn measure(v: &mut Viewer) {
    let (w, h) = (f64::from(v.canvas.client_width().max(1)), f64::from(v.canvas.client_height().max(1)));
    v.ratio = v.window.device_pixel_ratio().clamp(1.0, 4.0);
    v.canvas.set_width((w * v.ratio) as u32);
    v.canvas.set_height((h * v.ratio) as u32);
    // Resizing the canvas resets what it remembers.
    ok(v.ctx.set_transform(v.ratio, 0.0, 0.0, v.ratio, 0.0, 0.0));
    v.stage = Stage { w, h };
}

/// Draws the page `width` pixels wide into the off-screen picture.
fn make_picture(v: &mut Viewer, width: u32) -> Result<(), String> {
    let raster = v.sheet.draw(v.page, width)?;
    let image = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&raster.rgba), raster.width, raster.height).map_err(js)?;
    v.picture.set_width(raster.width);
    v.picture.set_height(raster.height);
    let ctx: CanvasRenderingContext2d = v.picture.get_context("2d").map_err(js)?.ok_or("No 2D canvas.")?.dyn_into().map_err(wrong)?;
    ctx.put_image_data(&image, 0.0, 0.0).map_err(js)?;
    v.picture_w = raster.width;
    Ok(())
}

/// Draws the page again, sharper, once the view has settled.
fn sharpen(v: &mut Viewer) {
    v.timer = None;
    let want = geom::render_width(&v.cam, v.ratio);
    if geom::needs_redraw(v.picture_w, want) && make_picture(v, want).is_ok() {
        paint(v);
    }
}

fn moved(v: &mut Viewer) {
    v.cam = v.cam.keep_in_view(v.stage);
    paint(v);
    if let Some(t) = v.timer.take() {
        v.window.clear_timeout_with_handle(t);
    }
    if let Some(cb) = &v.sharpen {
        v.timer = v.window.set_timeout_with_callback_and_timeout_and_arguments_0(cb.as_ref().unchecked_ref(), SHARPEN_MS).ok();
    }
}

fn paint(v: &Viewer) {
    let c = &v.ctx;
    c.set_fill_style_str("#dfe3e8");
    c.fill_rect(0.0, 0.0, v.stage.w, v.stage.h);
    let (pw, ph) = v.cam.size();
    c.set_image_smoothing_enabled(true);
    ok(c.draw_image_with_html_canvas_element_and_dw_and_dh(&v.picture, v.cam.x, v.cam.y, pw, ph));
    for (shape, resolved) in shapes::shapes(&v.marks, &v.cam) {
        draw_shape(c, &shape, if resolved { GREY } else { RED });
    }
    // The markup being placed.
    if let Some((fx, fy)) = v.first {
        let (x, y) = v.cam.to_screen(fx, fy);
        c.set_fill_style_str(BLUE);
        c.begin_path();
        ok(c.arc(x, y, 5.0, 0.0, std::f64::consts::TAU));
        c.fill();
        if let Some((hx, hy)) = v.hover.filter(|_| v.tool.two_points()) {
            let shape = if v.tool == Tool::Box {
                Shape::Box { x: x.min(hx), y: y.min(hy), w: (x - hx).abs(), h: (y - hy).abs(), label: String::new() }
            } else {
                Shape::Arrow { x1: x, y1: y, x2: hx, y2: hy, label: String::new() }
            };
            draw_shape(c, &shape, BLUE);
        }
    }
}

fn draw_shape(c: &CanvasRenderingContext2d, shape: &Shape, color: &str) {
    c.set_line_width(2.0);
    c.set_stroke_style_str(color);
    c.set_fill_style_str(color);
    c.set_font("bold 12px sans-serif");
    match shape {
        Shape::Pin { x, y, label } => {
            c.begin_path();
            ok(c.arc(*x, *y, 11.0, 0.0, std::f64::consts::TAU));
            c.set_fill_style_str("#ffffff");
            c.fill();
            c.stroke();
            c.set_fill_style_str(color);
            c.set_text_align("center");
            c.set_text_baseline("middle");
            ok(c.fill_text(label, *x, *y + 1.0));
        }
        Shape::Box { x, y, w, h, label } => {
            c.stroke_rect(*x, *y, *w, *h);
            label_at(c, label, *x + 3.0, *y - 6.0, color);
        }
        Shape::Arrow { x1, y1, x2, y2, label } => {
            c.begin_path();
            c.move_to(*x1, *y1);
            c.line_to(*x2, *y2);
            c.stroke();
            let [a, b] = shapes::arrow_head(*x1, *y1, *x2, *y2, 12.0);
            c.begin_path();
            c.move_to(*x2, *y2);
            c.line_to(a.0, a.1);
            c.line_to(b.0, b.1);
            c.close_path();
            c.fill();
            label_at(c, label, *x1 + 4.0, *y1 - 6.0, color);
        }
        Shape::Text { x, y, label } => {
            c.set_font("bold 16px sans-serif");
            label_at(c, label, *x, *y, color);
        }
    }
}

/// Words with a white edge so they can be read over a drawing.
fn label_at(c: &CanvasRenderingContext2d, text: &str, x: f64, y: f64, color: &str) {
    if text.is_empty() {
        return;
    }
    c.set_text_align("left");
    c.set_text_baseline("alphabetic");
    c.set_line_width(4.0);
    c.set_stroke_style_str("#ffffff");
    ok(c.stroke_text(text, x, y));
    c.set_fill_style_str(color);
    ok(c.fill_text(text, x, y));
    c.set_line_width(2.0);
}

fn set_hint(v: &Viewer) {
    if let Ok(Some(hint)) = v.root.query_selector("#wasm-hint") {
        hint.set_text_content(Some(v.tool.hint(v.first.is_some())));
    }
}

fn set_tool(v: &mut Viewer, tool: Tool) {
    v.tool = tool;
    v.first = None;
    ok(v.canvas.class_list().toggle_with_force("marking", tool != Tool::Move));
    let buttons = v.root.get_elements_by_class_name("tool-btn");
    for i in 0..buttons.length() {
        if let Some(b) = buttons.item(i) {
            let on = b.get_attribute("data-tool").as_deref().and_then(Tool::from_key) == Some(tool);
            ok(b.class_list().toggle_with_force("active", on));
        }
    }
    close_form(v);
    set_hint(v);
    paint(v);
}

fn input(v: &Viewer, id: &str) -> Option<HtmlInputElement> {
    v.document.get_element_by_id(id)?.dyn_into().ok()
}

fn close_form(v: &Viewer) {
    if let Some(form) = v.document.get_element_by_id("wasm-markup") {
        ok(form.set_attribute("hidden", ""));
    }
}

/// Fills in the markup form with the place clicked and shows it.
fn open_form(v: &mut Viewer, a: (f64, f64), b: Option<(f64, f64)>) {
    let set = |id: &str, value: String, enabled: bool| {
        if let Some(i) = input(v, id) {
            i.set_value(&value);
            i.set_disabled(!enabled);
        }
    };
    set("mk-kind", v.tool.kind().to_string(), true);
    set("mk-x", geom::to_units(a.0).to_string(), true);
    set("mk-y", geom::to_units(a.1).to_string(), true);
    // A pin or a piece of text has one point; the far corner is only sent for a box or arrow.
    let far = b.unwrap_or(a);
    set("mk-x2", geom::to_units(far.0).to_string(), b.is_some());
    set("mk-y2", geom::to_units(far.1).to_string(), b.is_some());
    if let Some(form) = v.document.get_element_by_id("wasm-markup") {
        ok(form.remove_attribute("hidden"));
        form.scroll_into_view();
    }
    if let Some(text) = input(v, "mk-text") {
        text.set_required(v.tool == Tool::Text);
        ok(text.focus());
    }
    v.first = None;
    set_hint(v);
}

fn place(v: &mut Viewer, sx: f64, sy: f64) {
    if !v.can_mark || v.tool == Tool::Move {
        return;
    }
    let Some(p) = v.cam.to_page(sx, sy) else { return };
    if v.tool.two_points() {
        match v.first.take() {
            None => {
                v.first = Some(p);
                v.hover = Some((sx, sy));
                set_hint(v);
            }
            Some(a) => open_form(v, a, Some(p)),
        }
    } else {
        open_form(v, p, None);
    }
    paint(v);
}

fn local(canvas: &HtmlCanvasElement, e: &PointerEvent) -> (f64, f64) {
    let r = canvas.get_bounding_client_rect();
    (f64::from(e.client_x()) - r.left(), f64::from(e.client_y()) - r.top())
}

fn wire(shared: &Shared, aspect: f64) {
    let (canvas, root, window, document) = {
        let v = shared.borrow();
        (v.canvas.clone(), v.root.clone(), v.window.clone(), v.document.clone())
    };

    let s = shared.clone();
    listen(&canvas, "pointerdown", move |e: PointerEvent| {
        with(&s, |v| {
            let (x, y) = local(&v.canvas, &e);
            ok(v.canvas.set_pointer_capture(e.pointer_id()));
            v.fingers.retain(|f| f.id != e.pointer_id());
            v.fingers.push(Finger { id: e.pointer_id(), x, y });
            // A second finger is a pinch, never a click.
            v.press = if v.fingers.len() == 1 { Some(Press { x, y, moved: false }) } else { v.press.take().map(|p| Press { moved: true, ..p }) };
        });
    });

    let s = shared.clone();
    listen(&canvas, "pointermove", move |e: PointerEvent| {
        with(&s, |v| {
            let (x, y) = local(&v.canvas, &e);
            v.hover = Some((x, y));
            let Some(i) = v.fingers.iter().position(|f| f.id == e.pointer_id()) else {
                if v.first.is_some() {
                    paint(v);
                }
                return;
            };
            let before: Vec<(f64, f64)> = v.fingers.iter().map(|f| (f.x, f.y)).collect();
            if let Some(f) = v.fingers.get_mut(i) {
                (f.x, f.y) = (x, y);
            }
            match (before.as_slice(), v.fingers.as_slice()) {
                ([_], [f]) => {
                    if let Some(p) = v.press.as_mut() {
                        if (f.x - p.x).hypot(f.y - p.y) > CLICK_SLOP {
                            p.moved = true;
                        }
                        if p.moved {
                            let (dx, dy) = (f.x - before.first().map_or(f.x, |b| b.0), f.y - before.first().map_or(f.y, |b| b.1));
                            v.cam = v.cam.pan(dx, dy);
                            moved(v);
                            return;
                        }
                    }
                    if v.first.is_some() {
                        paint(v);
                    }
                }
                ([a0, b0], [a1, b1]) => {
                    let (d0, d1) = ((a0.0 - b0.0).hypot(a0.1 - b0.1), (a1.x - b1.x).hypot(a1.y - b1.y));
                    let (m0, m1) = (((a0.0 + b0.0) / 2.0, (a0.1 + b0.1) / 2.0), ((a1.x + b1.x) / 2.0, (a1.y + b1.y) / 2.0));
                    if d0 > 1.0 && d1 > 1.0 {
                        v.cam = v.cam.zoom_at(m1.0, m1.1, d1 / d0).pan(m1.0 - m0.0, m1.1 - m0.1);
                        moved(v);
                    }
                }
                _ => {}
            }
        });
    });

    for name in ["pointerup", "pointercancel"] {
        let s = shared.clone();
        listen(&canvas, name, move |e: PointerEvent| {
            with(&s, |v| {
                let (x, y) = local(&v.canvas, &e);
                v.fingers.retain(|f| f.id != e.pointer_id());
                let clicked = e.type_() == "pointerup" && v.fingers.is_empty() && v.press.as_ref().is_some_and(|p| !p.moved);
                if v.fingers.is_empty() {
                    v.press = None;
                }
                if clicked {
                    place(v, x, y);
                }
            });
        });
    }

    let s = shared.clone();
    listen(&canvas, "wheel", move |e: WheelEvent| {
        e.prevent_default();
        with(&s, |v| {
            let lines = if e.delta_mode() == 1 { 16.0 } else { 1.0 };
            let speed = if e.ctrl_key() { 0.01 } else { 0.0015 };
            let r = v.canvas.get_bounding_client_rect();
            let (x, y) = (f64::from(e.client_x()) - r.left(), f64::from(e.client_y()) - r.top());
            v.cam = v.cam.zoom_at(x, y, (-e.delta_y() * lines * speed).exp());
            moved(v);
        });
    });

    // The buttons of the toolbar and the form are in the page; clicks are caught at the top.
    let s = shared.clone();
    listen(&root, "click", move |e: Event| {
        let Some(target) = e.target().and_then(|t| t.dyn_into::<Element>().ok()) else { return };
        let find = |selector: &str| target.closest(selector).ok().flatten();
        with(&s, |v| {
            if let Some(b) = find("[data-tool]") {
                if let Some(tool) = b.get_attribute("data-tool").as_deref().and_then(Tool::from_key) {
                    set_tool(v, tool);
                }
            } else if let Some(b) = find("[data-zoom]") {
                let (cx, cy) = (v.stage.w / 2.0, v.stage.h / 2.0);
                match b.get_attribute("data-zoom").as_deref() {
                    Some("in") => v.cam = v.cam.zoom_at(cx, cy, 1.5),
                    Some("out") => v.cam = v.cam.zoom_at(cx, cy, 1.0 / 1.5),
                    Some("fit") => v.cam = Camera::fit(v.stage, aspect),
                    _ => return,
                }
                moved(v);
            } else if find("[data-action=cancel]").is_some() {
                e.prevent_default();
                v.first = None;
                close_form(v);
                set_hint(v);
                paint(v);
            }
        });
    });

    let s = shared.clone();
    listen(&document, "keydown", move |e: KeyboardEvent| {
        if e.key() == "Escape" {
            with(&s, |v| {
                v.first = None;
                close_form(v);
                set_hint(v);
                paint(v);
            });
        }
    });

    let s = shared.clone();
    listen(&window, "resize", move |_: Event| {
        with(&s, |v| {
            measure(v);
            v.cam = Camera::fit(v.stage, aspect);
            moved(v);
        });
    });
}
