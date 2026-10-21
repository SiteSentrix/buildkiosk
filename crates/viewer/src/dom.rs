//! The browser side: fetches the sheet, draws it on a canvas and handles the mouse, touch and keys.
//!
//! Errors from the canvas calls are ignored on purpose (`ok`): a failed draw call leaves the last
//! picture on screen, and there is nothing better to do with it.

use std::cell::RefCell;
use std::rc::Rc;

use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    CanvasRenderingContext2d, Document, Element, Event, EventTarget, HtmlCanvasElement, HtmlCollection, HtmlElement, HtmlInputElement, ImageData,
    KeyboardEvent, PointerEvent, Response, WheelEvent, Window,
};

use buildkiosk_model::drawing::{MAX_ASPECT, MIN_ASPECT, PageScale};

use crate::diff;
use crate::geom::{self, Camera, Stage};
use crate::shapes::{self, Mark, Shape};
use crate::sheet::{MAX_FILE, Raster, Sheet};

const RED: &str = "#c62828";
const GREY: &str = "#7b8794";
const BLUE: &str = "#1565c0";
/// Pointer movement under this many pixels is still a click.
const CLICK_SLOP: f64 = 4.0;
/// How long after the last zoom or pan the page is drawn again, sharper.
const SHARPEN_MS: i32 = 180;
/// At most this many revisions are read from a page.
const MAX_REVS: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Move,
    Pin,
    Box,
    Arrow,
    Text,
    /// One click per item counted.
    Count,
    /// Two clicks on a stretch of known length set the page's scale.
    Scale,
    Length,
    Area,
    /// Any number of clicks along a path, measured end to end.
    Path,
    /// Any number of clicks round an outline, whose area is measured.
    Outline,
}

impl Tool {
    fn from_key(key: &str) -> Option<Tool> {
        Some(match key {
            "move" => Tool::Move,
            "pin" => Tool::Pin,
            "rect" => Tool::Box,
            "arrow" => Tool::Arrow,
            "text" => Tool::Text,
            "count" => Tool::Count,
            "scale" => Tool::Scale,
            "dim" => Tool::Length,
            "area" => Tool::Area,
            "polyline" => Tool::Path,
            "polygon" => Tool::Outline,
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
            Tool::Count => "count",
            Tool::Length => "dim",
            Tool::Area => "area",
            Tool::Path => "polyline",
            Tool::Outline => "polygon",
            Tool::Scale => "",
        }
    }

    fn two_points(self) -> bool {
        matches!(self, Tool::Box | Tool::Arrow | Tool::Scale | Tool::Length | Tool::Area)
    }

    /// Whether the tool takes any number of clicks and ends with a finishing click or Enter.
    fn many_points(self) -> bool {
        matches!(self, Tool::Path | Tool::Outline)
    }

    /// The fewest corners the tool needs.
    fn least_points(self) -> usize {
        if self == Tool::Outline { 3 } else { 2 }
    }

    fn hint(self, started: bool) -> &'static str {
        match (self, started) {
            (Tool::Path, false) => "Click where the path starts.",
            (Tool::Path, true) => "Click each bend. Click the last point again, or press Enter, to finish; Backspace undoes a point.",
            (Tool::Outline, false) => "Click a corner of the outline.",
            (Tool::Outline, true) => "Click each corner. Click the first corner again, or press Enter, to close it; Backspace undoes a point.",
            (Tool::Move, _) => "Drag to move the sheet. Scroll or pinch to zoom.",
            (Tool::Pin, _) => "Click the sheet where the pin goes.",
            (Tool::Text, _) => "Click where the words go.",
            (Tool::Count, _) => "Click each item to count; name what you are counting in the note.",
            (Tool::Box, false) => "Click one corner of the box.",
            (Tool::Box, true) => "Click the opposite corner.",
            (Tool::Arrow, false) => "Click where the arrow starts.",
            (Tool::Arrow, true) => "Click where the arrow points.",
            (Tool::Scale, false) => "Click one end of a stretch whose real length you know.",
            (Tool::Scale, true) => "Click the other end of the stretch.",
            (Tool::Length, false) => "Click where the length starts.",
            (Tool::Length, true) => "Click where it ends.",
            (Tool::Area, false) => "Click one corner of the area.",
            (Tool::Area, true) => "Click the opposite corner.",
        }
    }
}

/// What the page tells the viewer about the sheet (`data-sheet`).
#[derive(Deserialize)]
struct Boot {
    /// The viewer page's own address, to build the address of a revision and page from.
    here: String,
    /// The label of the revision shown first, and the page (from 1).
    rev: String,
    page: usize,
    can_mark: bool,
    revisions: Vec<BootRev>,
}

#[derive(Deserialize)]
struct BootRev {
    label: String,
    file: String,
    #[serde(default)]
    marks: Vec<Mark>,
    #[serde(default)]
    scales: Vec<PageScale>,
}

/// One revision of the sheet. Its file is fetched and opened the first time it is needed.
struct Rev {
    label: String,
    file: String,
    sheet: Option<Rc<Sheet>>,
    marks: Vec<Mark>,
    scales: Vec<PageScale>,
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
    here: String,
    revs: Vec<Rev>,
    /// The revision shown, and the one laid over it to see what changed.
    rev: usize,
    compare: Option<usize>,
    /// The page shown, from 0.
    page: usize,
    /// Width over height of what is on the canvas (the page, or the comparison).
    aspect: f64,
    /// A revision is being fetched; further requests wait.
    busy: bool,
    stage: Stage,
    cam: Camera,
    ratio: f64,
    can_mark: bool,
    /// The wording the viewer writes into the page, in the page's language.
    words: crate::words::Words,
    tool: Tool,
    /// The first corner of a box or arrow, as fractions of the page.
    first: Option<(f64, f64)>,
    /// The corners clicked so far of a path or outline, as fractions of the page.
    path: Vec<(f64, f64)>,
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

pub(crate) fn ok<T>(r: Result<T, JsValue>) {
    let _ = r;
}

/// For a `dyn_into` that found a different kind of element than the page promised.
fn wrong<T>(_: T) -> String {
    "The page is not what the viewer expects.".to_string()
}

fn js(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| "The browser refused.".to_string())
}

pub(crate) fn listen<E: JsCast + 'static>(target: &EventTarget, name: &str, mut f: impl FnMut(E) + 'static) {
    let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
        if let Ok(e) = e.dyn_into::<E>() {
            f(e);
        }
    });
    ok(target.add_event_listener_with_callback(name, cb.as_ref().unchecked_ref()));
    // The listener lives as long as the page does.
    cb.forget();
}

fn each(list: HtmlCollection, mut f: impl FnMut(Element)) {
    for i in 0..list.length() {
        if let Some(e) = list.item(i) {
            f(e);
        }
    }
}

fn set_on(e: &Element, name: &str, on: bool) {
    if on {
        ok(e.set_attribute(name, ""));
    } else {
        ok(e.remove_attribute(name));
    }
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub fn start() {
    std::panic::set_hook(Box::new(|info| web_sys::console::error_1(&JsValue::from_str(&info.to_string()))));
    let Some(window) = web_sys::window() else { return };
    let Some(document) = window.document() else { return };
    // The schedule chart: bars that can be dragged.
    if let Ok(Some(chart)) = document.query_selector("#wasm-gantt") {
        crate::gantt::start(document.clone(), chart);
    }
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
    let mut boot: Boot = serde_json::from_str(&root.get_attribute("data-sheet").ok_or("The page describes no sheet.")?)
        .map_err(|_| "The page's sheet description is unreadable.")?;
    boot.revisions.truncate(MAX_REVS);
    for r in &mut boot.revisions {
        r.marks.truncate(shapes::MAX_MARKS);
    }
    let rev = boot.revisions.iter().position(|r| r.label == boot.rev).ok_or("The page shows a revision it doesn't list.")?;
    let page = boot.page.max(1) - 1;
    let file = boot.revisions.get(rev).map(|r| r.file.clone()).ok_or("The page shows a revision it doesn't list.")?;
    let first = Rc::new(Sheet::open(fetch_bytes(&window, &file).await?)?);
    let aspect = first.aspect(page).ok_or("This page doesn't exist.")?;

    let canvas: HtmlCanvasElement = root.query_selector("canvas").map_err(js)?.ok_or("The page has no canvas.")?.dyn_into().map_err(wrong)?;
    let ctx: CanvasRenderingContext2d = canvas.get_context("2d").map_err(js)?.ok_or("No 2D canvas.")?.dyn_into().map_err(wrong)?;
    let picture: HtmlCanvasElement = document.create_element("canvas").map_err(js)?.dyn_into().map_err(wrong)?;
    let body: HtmlElement = document.body().ok_or("The page has no body.")?;

    let revs: Vec<Rev> = boot
        .revisions
        .into_iter()
        .enumerate()
        .map(|(i, r)| Rev { label: r.label, file: r.file, sheet: (i == rev).then(|| first.clone()), marks: r.marks, scales: r.scales })
        .collect();

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
        here: boot.here,
        revs,
        rev,
        compare: None,
        page,
        aspect,
        busy: false,
        stage: Stage { w: 1.0, h: 1.0 },
        cam: Camera::fit(Stage { w: 1.0, h: 1.0 }, aspect),
        ratio: 1.0,
        can_mark: boot.can_mark,
        words: crate::words::Words::for_language(
            &document.document_element().map(|e| e.get_attribute("lang").unwrap_or_default()).unwrap_or_default(),
        ),
        tool: Tool::Move,
        first: None,
        path: Vec::new(),
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
    set_tool(&mut v, Tool::Move);
    sync_ui(&v);
    paint(&v);

    let shared: Shared = Rc::new(RefCell::new(v));
    let again = shared.clone();
    let cb = Closure::<dyn FnMut()>::new(move || with(&again, sharpen));
    with(&shared, |v| v.sharpen = Some(cb));
    wire(&shared);
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

/// What is shown at `width` pixels wide: the page, or, with a revision to compare, what differs.
fn raster(v: &Viewer, width: u32) -> Result<Raster, String> {
    let sheet = |i: usize| v.revs.get(i).and_then(|r| r.sheet.clone()).ok_or_else(|| "That revision isn't loaded.".to_string());
    let shown = sheet(v.rev)?.draw(v.page, width)?;
    match v.compare {
        None => Ok(shown),
        Some(other) => {
            let old = sheet(other)?.draw(v.page, shown.width)?;
            diff::overlay(&old, &shown)
        }
    }
}

/// Draws what is shown into the off-screen picture and remembers its shape.
fn make_picture(v: &mut Viewer, width: u32) -> Result<(), String> {
    let r = raster(v, width)?;
    let image = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&r.rgba), r.width, r.height).map_err(js)?;
    let ctx: CanvasRenderingContext2d = v.picture.get_context("2d").map_err(js)?.ok_or("No 2D canvas.")?.dyn_into().map_err(wrong)?;
    v.picture.set_width(r.width);
    v.picture.set_height(r.height);
    ctx.put_image_data(&image, 0.0, 0.0).map_err(js)?;
    v.picture_w = r.width;
    v.aspect = f64::from(r.width) / f64::from(r.height.max(1));
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

/// The scale set on the page shown, if there is one.
fn scale_here(v: &Viewer) -> Option<&PageScale> {
    let page = u32::try_from(v.page + 1).ok()?;
    v.revs.get(v.rev)?.scales.iter().find(|s| s.page == page)
}

/// What a length or area from `a` to `b` (fractions of the page) measures on this page, as text.
fn measure_text(v: &Viewer, tool: Tool, a: (f64, f64), b: (f64, f64)) -> Option<String> {
    let s = scale_here(v)?;
    let (x, y, x2, y2) = (geom::to_units(a.0), geom::to_units(a.1), geom::to_units(b.0), geom::to_units(b.1));
    match tool {
        Tool::Length => s.length(x, y, x2, y2).map(|um| s.unit.length_text(um)),
        Tool::Area => s.area(x, y, x2, y2).map(|um| s.unit.area_text(um)),
        _ => None,
    }
}

/// What a path or outline through `pts` (fractions of the page) measures on this page, as text.
fn measure_path(v: &Viewer, tool: Tool, pts: &[(f64, f64)]) -> Option<String> {
    let s = scale_here(v)?;
    let units: Vec<(u16, u16)> = pts.iter().map(|p| (geom::to_units(p.0), geom::to_units(p.1))).collect();
    match tool {
        Tool::Path => s.path_length(&units).map(|um| s.unit.length_text(um)),
        Tool::Outline => s.polygon_area(&units).map(|um| s.unit.area_text(um)),
        _ => None,
    }
}

fn paint(v: &Viewer) {
    let c = &v.ctx;
    c.set_fill_style_str("#dfe3e8");
    c.fill_rect(0.0, 0.0, v.stage.w, v.stage.h);
    let (pw, ph) = v.cam.size();
    c.set_image_smoothing_enabled(true);
    ok(c.draw_image_with_html_canvas_element_and_dw_and_dh(&v.picture, v.cam.x, v.cam.y, pw, ph));
    // Markups belong to one revision's page, so they aren't drawn over a comparison.
    if v.compare.is_none() {
        let page = u32::try_from(v.page + 1).unwrap_or(u32::MAX);
        let marks: Vec<Mark> = v.revs.get(v.rev).map(|r| r.marks.iter().filter(|m| m.page == page).cloned().collect()).unwrap_or_default();
        for (shape, resolved) in shapes::shapes(&marks, &v.cam) {
            draw_shape(c, &shape, if resolved { GREY } else { RED });
        }
    }
    // The stretch the page's scale was set from, faintly, while a measuring tool is in use.
    if v.compare.is_none()
        && matches!(v.tool, Tool::Scale | Tool::Length | Tool::Area | Tool::Path | Tool::Outline)
        && let Some(s) = scale_here(v)
    {
        let ((x1, y1), (x2, y2)) =
            (v.cam.to_screen(geom::from_units(s.x), geom::from_units(s.y)), v.cam.to_screen(geom::from_units(s.x2), geom::from_units(s.y2)));
        c.set_stroke_style_str(BLUE);
        c.set_line_width(1.5);
        ok(c.set_line_dash(&js_sys::Array::of2(&JsValue::from_f64(6.0), &JsValue::from_f64(4.0))));
        c.begin_path();
        c.move_to(x1, y1);
        c.line_to(x2, y2);
        c.stroke();
        ok(c.set_line_dash(&js_sys::Array::new()));
    }
    // The path or outline being placed: the corners so far and a line to the pointer.
    if !v.path.is_empty() {
        let screen: Vec<(f64, f64)> = v.path.iter().map(|p| v.cam.to_screen(p.0, p.1)).collect();
        let mut shown = screen.clone();
        let mut pts = v.path.clone();
        if let Some((hx, hy)) = v.hover {
            shown.push((hx, hy));
            if let Some(b) = v.cam.to_page(hx, hy) {
                pts.push(b);
            }
        }
        let reading = if pts.len() >= v.tool.least_points() { measure_path(v, v.tool, &pts).unwrap_or_default() } else { String::new() };
        draw_shape(c, &Shape::Poly { points: shown, closed: v.tool == Tool::Outline && pts.len() >= 3, label: String::new() }, BLUE);
        c.set_fill_style_str(BLUE);
        for (x, y) in &screen {
            c.begin_path();
            ok(c.arc(*x, *y, 4.0, 0.0, std::f64::consts::TAU));
            c.fill();
        }
        if let Some((hx, hy)) = v.hover {
            label_at(c, &reading, hx + 12.0, hy - 8.0, BLUE);
        }
    }
    // The markup being placed.
    if let Some((fx, fy)) = v.first {
        let (x, y) = v.cam.to_screen(fx, fy);
        c.set_fill_style_str(BLUE);
        c.begin_path();
        ok(c.arc(x, y, 5.0, 0.0, std::f64::consts::TAU));
        c.fill();
        if let Some((hx, hy)) = v.hover.filter(|_| v.tool.two_points()) {
            // What the stretch measures so far, beside the pointer.
            let reading = v.cam.to_page(hx, hy).and_then(|b| measure_text(v, v.tool, (fx, fy), b)).unwrap_or_default();
            let shape = match v.tool {
                Tool::Box | Tool::Area => Shape::Box { x: x.min(hx), y: y.min(hy), w: (x - hx).abs(), h: (y - hy).abs(), label: String::new() },
                Tool::Arrow => Shape::Arrow { x1: x, y1: y, x2: hx, y2: hy, label: String::new() },
                _ => Shape::Line { x1: x, y1: y, x2: hx, y2: hy, label: String::new() },
            };
            draw_shape(c, &shape, BLUE);
            label_at(c, &reading, hx + 12.0, hy - 8.0, BLUE);
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
        Shape::Line { x1, y1, x2, y2, label } => {
            c.begin_path();
            c.move_to(*x1, *y1);
            c.line_to(*x2, *y2);
            // A short tick across each end.
            let (dx, dy) = (x2 - x1, y2 - y1);
            let len = dx.hypot(dy).max(1.0);
            let (nx, ny) = (-dy / len * 6.0, dx / len * 6.0);
            for (px, py) in [(*x1, *y1), (*x2, *y2)] {
                c.move_to(px - nx, py - ny);
                c.line_to(px + nx, py + ny);
            }
            c.stroke();
            label_at(c, label, (x1 + x2) / 2.0 + 6.0, (y1 + y2) / 2.0 - 6.0, color);
        }
        Shape::Text { x, y, label } => {
            c.set_font("bold 16px sans-serif");
            label_at(c, label, *x, *y, color);
        }
        Shape::Poly { points, closed, label } => {
            if let Some(((fx, fy), rest)) = points.split_first() {
                c.begin_path();
                c.move_to(*fx, *fy);
                for (x, y) in rest {
                    c.line_to(*x, *y);
                }
                if *closed {
                    c.close_path();
                }
                c.stroke();
                label_at(c, label, *fx + 4.0, *fy - 6.0, color);
            }
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
    set_status(v, v.tool.hint(v.first.is_some() || !v.path.is_empty()));
}

fn set_status(v: &Viewer, text: &str) {
    if let Ok(Some(hint)) = v.root.query_selector("#wasm-hint") {
        hint.set_text_content(Some(&v.words.t(text)));
    }
}

fn set_tool(v: &mut Viewer, tool: Tool) {
    v.tool = tool;
    v.first = None;
    v.path.clear();
    ok(v.canvas.class_list().toggle_with_force("marking", tool != Tool::Move));
    each(v.root.get_elements_by_class_name("tool-btn"), |b| {
        let on = b.get_attribute("data-tool").as_deref().and_then(Tool::from_key) == Some(tool);
        ok(b.class_list().toggle_with_force("active", on));
    });
    close_form(v);
    set_hint(v);
    paint(v);
}

/// Brings the buttons, the lists of markups, the form and the address up to date with what is shown.
fn sync_ui(v: &Viewer) {
    let (Some(rev), page) = (v.revs.get(v.rev), v.page + 1) else { return };
    let compare = v.compare.and_then(|i| v.revs.get(i)).map(|r| r.label.as_str());
    each(v.root.get_elements_by_class_name("rev-btn"), |b| {
        ok(b.class_list().toggle_with_force("active", b.get_attribute("data-rev").as_deref() == Some(rev.label.as_str())));
    });
    each(v.root.get_elements_by_class_name("cmp-btn"), |b| {
        let label = b.get_attribute("data-compare");
        ok(b.class_list().toggle_with_force("active", compare.is_some() && label.as_deref() == compare));
        // A revision can't be compared with itself.
        set_on(&b, "disabled", label.as_deref() == Some(rev.label.as_str()));
    });
    let pages = rev.sheet.as_ref().map_or(1, |s| s.pages());
    if let Some(e) = v.document.get_element_by_id("wasm-page") {
        e.set_text_content(Some(&format!("{page} of {pages}")));
    }
    each(v.root.get_elements_by_class_name("page-btn"), |b| {
        let step = b.get_attribute("data-page-step").and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        set_on(&b, "disabled", (step < 0 && page <= 1) || (step > 0 && page >= pages));
    });
    each(v.root.get_elements_by_class_name("tool-btn"), |b| set_on(&b, "disabled", v.compare.is_some()));
    if let Some(e) = v.document.get_element_by_id("wasm-legend") {
        set_on(&e, "hidden", v.compare.is_none());
    }
    // The lists of markups below the sheet: only this revision's page, and none over a comparison.
    let mut any = false;
    each(v.document.get_elements_by_class_name("marklist"), |m| {
        let show = v.compare.is_none()
            && m.get_attribute("data-rev").as_deref() == Some(rev.label.as_str())
            && m.get_attribute("data-page").and_then(|p| p.parse::<usize>().ok()) == Some(page);
        any |= show;
        set_on(&m, "hidden", !show);
    });
    each(v.document.get_elements_by_class_name("marklist-none"), |m| set_on(&m, "hidden", any || v.compare.is_some()));
    // The page's scale, and the button that removes it.
    let has_scale = scale_here(v).is_some();
    if let Some(e) = v.document.get_element_by_id("wasm-scale-note") {
        let text = match scale_here(v) {
            Some(s) => {
                v.words.tf("Scale set from a stretch of {}. Lengths and areas drawn on this page use it.", &[&s.unit.length_text(s.length_um)])
            }
            None => v.words.t("No scale set on this page. Use the Scale tool to set it, then measure lengths and areas."),
        };
        e.set_text_content(Some(&text));
    }
    if let Some(e) = v.document.get_element_by_id("wasm-scale-clear") {
        set_on(&e, "hidden", !has_scale || !v.can_mark);
    }
    // Saving a markup goes back to this revision and page.
    let here = format!("{}?rev={}&page={page}", v.here, enc(&rev.label));
    let value = |id: &str, text: &str| {
        if let Some(i) = input(v, id) {
            i.set_value(text);
        }
    };
    for id in ["mk-revision", "sf-revision", "sc-revision"] {
        value(id, &rev.label);
    }
    for id in ["mk-page", "sf-page", "sc-page"] {
        value(id, &page.to_string());
    }
    for id in ["wasm-markup", "wasm-scale-form", "wasm-scale-clear"] {
        if let Some(form) = v.document.get_element_by_id(id) {
            for name in ["_next", "_back"] {
                if let Ok(Some(i)) = form.query_selector(&format!("input[name={name}]"))
                    && let Ok(i) = i.dyn_into::<HtmlInputElement>()
                {
                    i.set_value(&here);
                }
            }
        }
    }
    if let Ok(history) = v.window.history() {
        ok(history.replace_state_with_url(&JsValue::NULL, "", Some(&here)));
    }
}

fn input(v: &Viewer, id: &str) -> Option<HtmlInputElement> {
    v.document.get_element_by_id(id)?.dyn_into().ok()
}

fn close_form(v: &Viewer) {
    for id in ["wasm-markup", "wasm-scale-form"] {
        if let Some(form) = v.document.get_element_by_id(id) {
            ok(form.set_attribute("hidden", ""));
        }
    }
}

/// Fills in the form that sets the page's scale with the stretch drawn, and shows it.
fn open_scale_form(v: &mut Viewer, a: (f64, f64), b: (f64, f64)) {
    let aspect = v.revs.get(v.rev).and_then(|r| r.sheet.as_ref()).and_then(|s| s.aspect(v.page)).unwrap_or(1.0);
    let milli = (aspect * 1000.0).round().clamp(f64::from(MIN_ASPECT), f64::from(MAX_ASPECT)) as u32;
    for (id, value) in [
        ("sf-x", geom::to_units(a.0).to_string()),
        ("sf-y", geom::to_units(a.1).to_string()),
        ("sf-x2", geom::to_units(b.0).to_string()),
        ("sf-y2", geom::to_units(b.1).to_string()),
        ("sf-aspect", milli.to_string()),
    ] {
        if let Some(i) = input(v, id) {
            i.set_value(&value);
        }
    }
    if let Some(form) = v.document.get_element_by_id("wasm-scale-form") {
        ok(form.remove_attribute("hidden"));
        form.scroll_into_view();
    }
    if let Some(i) = input(v, "sf-length") {
        ok(i.focus());
    }
    v.first = None;
    set_hint(v);
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
    set("mk-points", String::new(), false);
    set("mk-x2", geom::to_units(far.0).to_string(), b.is_some());
    set("mk-y2", geom::to_units(far.1).to_string(), b.is_some());
    // What a length or area measures, so the person can check it before saving.
    let reading = b.and_then(|b| measure_text(v, v.tool, a, b)).map(|t| format!("Measures {t}")).unwrap_or_default();
    if let Some(e) = v.document.get_element_by_id("mk-reading") {
        e.set_text_content(Some(&reading));
    }
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

/// Ends a path or outline: opens the form with its corners, or says how many more are needed.
fn finish_path(v: &mut Viewer) {
    let pts = std::mem::take(&mut v.path);
    if pts.len() < v.tool.least_points() {
        set_status(v, "That needs more points. Keep clicking, or press Escape to start over.");
        v.path = pts;
        return;
    }
    let units: Vec<(u16, u16)> = pts.iter().map(|p| (geom::to_units(p.0), geom::to_units(p.1))).collect();
    // The engine refuses a path that crosses itself; say so here, before the form.
    if !buildkiosk_model::drawing::outline_is_simple(&units, v.tool == Tool::Outline) {
        set_status(v, "The sides cross or double back. Backspace removes the last point; draw it so the sides only meet at corners.");
        v.path = pts;
        return;
    }
    let set = |id: &str, value: String, enabled: bool| {
        if let Some(i) = input(v, id) {
            i.set_value(&value);
            i.set_disabled(!enabled);
        }
    };
    let first = units.first().copied().unwrap_or((0, 0));
    let second = units.get(1).copied().unwrap_or(first);
    set("mk-kind", v.tool.kind().to_string(), true);
    set("mk-x", first.0.to_string(), true);
    set("mk-y", first.1.to_string(), true);
    set("mk-x2", second.0.to_string(), false);
    set("mk-y2", second.1.to_string(), false);
    set("mk-points", units.iter().map(|p| format!("{},{}", p.0, p.1)).collect::<Vec<_>>().join(" "), true);
    let reading = measure_path(v, v.tool, &pts).map(|t| format!("Measures {t}")).unwrap_or_default();
    if let Some(e) = v.document.get_element_by_id("mk-reading") {
        e.set_text_content(Some(&reading));
    }
    if let Some(form) = v.document.get_element_by_id("wasm-markup") {
        ok(form.remove_attribute("hidden"));
        form.scroll_into_view();
    }
    if let Some(text) = input(v, "mk-text") {
        text.set_required(false);
        ok(text.focus());
    }
    set_hint(v);
}

fn place(v: &mut Viewer, sx: f64, sy: f64) {
    if !v.can_mark || v.tool == Tool::Move || v.compare.is_some() {
        return;
    }
    let Some(p) = v.cam.to_page(sx, sy) else { return };
    if matches!(v.tool, Tool::Length | Tool::Area | Tool::Path | Tool::Outline) && scale_here(v).is_none() {
        set_status(v, "Set this page's scale first: pick Scale, then click both ends of a stretch whose real length you know.");
        return;
    }
    if v.tool.many_points() {
        // Clicking on the last corner again (or, for an outline, the first) finishes; anything else adds a corner.
        let near = |a: (f64, f64)| {
            let (ax, ay) = v.cam.to_screen(a.0, a.1);
            (ax - sx).hypot(ay - sy) <= 10.0
        };
        let closing = match v.tool {
            Tool::Outline => v.path.len() >= 3 && v.path.first().is_some_and(|f| near(*f)),
            _ => v.path.last().is_some_and(|l| near(*l)) && v.path.len() >= 2,
        };
        if closing {
            finish_path(v);
        } else if v.path.last().is_none_or(|l| !near(*l)) {
            if v.path.len() >= buildkiosk_model::drawing::MAX_POINTS {
                set_status(v, "That is as many points as one path can have; press Enter to finish.");
            } else {
                v.path.push(p);
                v.hover = Some((sx, sy));
                set_hint(v);
            }
        }
    } else if v.tool.two_points() {
        match v.first.take() {
            None => {
                v.first = Some(p);
                v.hover = Some((sx, sy));
                set_hint(v);
            }
            Some(a) if v.tool == Tool::Scale => open_scale_form(v, a, p),
            Some(a) => open_form(v, a, Some(p)),
        }
    } else {
        open_form(v, p, None);
    }
    paint(v);
}

/// What a click on the revision, page or compare buttons asks for; `None` leaves it as it is.
#[derive(Clone, Copy)]
struct Go {
    rev: Option<usize>,
    page: Option<usize>,
    /// `Some(None)` switches the comparison off.
    compare: Option<Option<usize>>,
}

/// Fetches a revision's file the first time it is needed.
async fn load(shared: &Shared, idx: usize) -> Result<(), String> {
    let (window, url) = {
        let v = shared.try_borrow().map_err(|_| "Busy.")?;
        let r = v.revs.get(idx).ok_or("There is no such revision.")?;
        if r.sheet.is_some() {
            return Ok(());
        }
        (v.window.clone(), r.file.clone())
    };
    let sheet = Rc::new(Sheet::open(fetch_bytes(&window, &url).await?).map_err(|e| format!("That revision can't be shown here: {e}"))?);
    with(shared, |v| {
        if let Some(r) = v.revs.get_mut(idx) {
            r.sheet = Some(sheet);
        }
    });
    Ok(())
}

fn go(shared: &Shared, req: Go) {
    let shared = shared.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let want = {
            let Ok(mut v) = shared.try_borrow_mut() else { return };
            if v.busy {
                return;
            }
            v.busy = true;
            set_status(&v, "Loading…");
            (req.rev.unwrap_or(v.rev), req.compare.unwrap_or(v.compare))
        };
        let mut result = load(&shared, want.0).await;
        if let (Ok(()), Some(other)) = (&result, want.1) {
            result = load(&shared, other).await;
        }
        with(&shared, |v| {
            v.busy = false;
            let outcome = result.and_then(|()| switch(v, req));
            match outcome {
                Ok(()) => set_hint(v),
                Err(e) => set_status(v, &e),
            }
        });
    });
}

/// Shows another revision, page or comparison (the files are loaded already).
fn switch(v: &mut Viewer, req: Go) -> Result<(), String> {
    let before = (v.rev, v.page, v.compare, v.aspect);
    v.rev = req.rev.unwrap_or(v.rev);
    v.compare = req.compare.unwrap_or(v.compare).filter(|c| *c != v.rev);
    let pages = v.revs.get(v.rev).and_then(|r| r.sheet.as_ref()).map_or(1, |s| s.pages());
    v.page = req.page.unwrap_or(v.page).min(pages.saturating_sub(1));
    v.first = None;
    v.path.clear();
    // Fit the new picture; the one before stays if the new one can't be drawn.
    let width = geom::render_width(&Camera::fit(v.stage, v.aspect), v.ratio);
    if let Err(e) = make_picture(v, width) {
        (v.rev, v.page, v.compare, v.aspect) = before;
        return Err(e);
    }
    v.cam = Camera::fit(v.stage, v.aspect);
    sharpen(v);
    close_form(v);
    sync_ui(v);
    paint(v);
    Ok(())
}

fn local(canvas: &HtmlCanvasElement, e: &PointerEvent) -> (f64, f64) {
    let r = canvas.get_bounding_client_rect();
    (f64::from(e.client_x()) - r.left(), f64::from(e.client_y()) - r.top())
}

fn index_of(v: &Viewer, label: Option<String>) -> Option<usize> {
    let label = label?;
    v.revs.iter().position(|r| r.label == label)
}

fn wire(shared: &Shared) {
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
                if v.first.is_some() || !v.path.is_empty() {
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
        let mut ask: Option<Go> = None;
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
                    Some("fit") => v.cam = Camera::fit(v.stage, v.aspect),
                    _ => return,
                }
                moved(v);
            } else if let Some(b) = find("[data-rev]") {
                ask = index_of(v, b.get_attribute("data-rev")).map(|i| Go { rev: Some(i), page: None, compare: None });
            } else if let Some(b) = find("[data-compare]") {
                // An empty label switches the comparison off; pressing the one in use does too.
                let label = b.get_attribute("data-compare").filter(|l| !l.is_empty());
                let target = index_of(v, label).filter(|i| Some(*i) != v.compare);
                ask = Some(Go { rev: None, page: None, compare: Some(target) });
            } else if let Some(b) = find("[data-page-step]") {
                let step = b.get_attribute("data-page-step").and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                let page = i64::try_from(v.page).unwrap_or(0).saturating_add(step).max(0);
                ask = Some(Go { rev: None, page: Some(usize::try_from(page).unwrap_or(0)), compare: None });
            } else if find("[data-action=cancel]").is_some() {
                e.prevent_default();
                v.first = None;
                v.path.clear();
                close_form(v);
                set_hint(v);
                paint(v);
            }
        });
        if let Some(req) = ask {
            go(&s, req);
        }
    });

    let s = shared.clone();
    listen(&document, "keydown", move |e: KeyboardEvent| match e.key().as_str() {
        "Escape" => with(&s, |v| {
            v.first = None;
            v.path.clear();
            close_form(v);
            set_hint(v);
            paint(v);
        }),
        "Enter" => with(&s, |v| {
            if v.tool.many_points() && !v.path.is_empty() {
                e.prevent_default();
                finish_path(v);
                paint(v);
            }
        }),
        "Backspace" => with(&s, |v| {
            if v.tool.many_points() && !v.path.is_empty() {
                e.prevent_default();
                v.path.pop();
                set_hint(v);
                paint(v);
            }
        }),
        _ => {}
    });

    let s = shared.clone();
    listen(&window, "resize", move |_: Event| {
        with(&s, |v| {
            measure(v);
            v.cam = Camera::fit(v.stage, v.aspect);
            moved(v);
        });
    });
}
