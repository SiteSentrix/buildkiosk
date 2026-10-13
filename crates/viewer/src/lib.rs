//! The parts of the app that run in the browser, built as WebAssembly (`cargo xtask viewer`): the sheet viewer
//! and dragging on the schedule chart.
//!
//! It improves the server-drawn viewer page: when this module loads and can draw the sheet, the
//! page shows a canvas that pans and zooms smoothly (the PDF page is drawn here, by the same
//! renderer the server uses), and markups are placed by clicking the sheet. Saving a markup is
//! still the plain form that posts the `drawing.markup_add` command, so every rule is enforced in
//! the engine. When the module is missing, can't load, or can't draw the file, the page keeps
//! working without it.

pub mod diff;
mod dom;
pub mod gantt;
pub mod geom;
pub mod shapes;
pub mod sheet;

use wasm_bindgen::prelude::*;

/// Runs when the module loads.
#[wasm_bindgen(start)]
pub fn start() {
    dom::start();
}
