//! DeckCraft's engine for a host UI that draws its own chrome: the engine's JSON commands and pointer tools, slide
//! rendering to pixels, and what an overlay needs (selection boxes, handles, caret and selected text).

use deckcraft_engine::{Session, TextSel, tools::PointerEvent};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

fn js(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

/// One presentation session: the open documents, their undo history and the active tool.
#[wasm_bindgen]
pub struct Deck {
    session: Session,
}

/// A rendered slide: straight RGBA8 pixels.
#[wasm_bindgen]
pub struct Rendered {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[wasm_bindgen]
impl Rendered {
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u32 {
        self.width
    }
    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn take_rgba(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.rgba)
    }
}

#[wasm_bindgen]
impl Deck {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Deck {
        Deck { session: Session::new() }
    }

    /// The ids and parameters of every command, one JSON array.
    pub fn commands(&self) -> String {
        let list: Vec<Value> = self.session.commands().iter().map(|c| json!({"id": c.id, "label": c.label, "params": c.params, "shortcut": c.shortcut})).collect();
        Value::Array(list).to_string()
    }

    /// Runs command `id` with JSON `params`; the JSON result.
    pub fn execute(&mut self, id: &str, params: &str) -> Result<String, JsError> {
        let v: Value = if params.trim().is_empty() { json!({}) } else { serde_json::from_str(params).map_err(js)? };
        self.session.execute(id, &v).map(|r| r.to_string()).map_err(js)
    }

    /// Opens a .pptx (or DeckCraft) file; the JSON the engine answers with.
    pub fn open(&mut self, name: &str, bytes: &[u8]) -> Result<String, JsError> {
        let data = base64(bytes);
        self.execute("file.openBytes", &json!({"name": name, "data": data}).to_string())
    }

    /// The presentation as .pptx bytes.
    pub fn save(&mut self) -> Result<Vec<u8>, JsError> {
        let r = self.session.execute("file.saveBytes", &json!({"format": "pptx"})).map_err(js)?;
        let text = r.get("data").and_then(Value::as_str).ok_or_else(|| js("the engine returned no data"))?;
        unbase64(text).ok_or_else(|| js("the engine's data is not base64"))
    }

    /// Adds a font (TTF, OTF or TTC) for this session's rendering.
    pub fn add_font(&mut self, bytes: &[u8]) -> Result<String, JsError> {
        self.execute("font.add", &json!({"data": base64(bytes)}).to_string())
    }

    /// Slide `index` at `scale` pixels per point; `edit` draws the editor view (placeholder prompts and outlines).
    pub fn render(&self, index: usize, scale: f64, edit: bool) -> Result<Rendered, JsError> {
        let st = self.session.doc().map_err(js)?;
        let opts = deckcraft_render::RenderOpts { scale: scale.clamp(0.05, 8.0), edit, ..Default::default() };
        let img = deckcraft_render::render_slide(&st.doc, index, &opts);
        Ok(Rendered { width: img.width, height: img.height, rgba: img.to_straight() })
    }

    /// The slide's animation steps for the show: `{"steps": n, "auto": [bool], "durations": [seconds]}`. Step `i` is
    /// started by a click unless `auto[i]`.
    pub fn show_info(&self, index: usize) -> Result<String, JsError> {
        let st = self.session.doc().map_err(js)?;
        let slide = st.doc.slides.get(index).ok_or_else(|| JsError::new("no such slide"))?;
        let tl = deckcraft_anim::Timeline::new(slide, st.doc.slide_size.width, st.doc.slide_size.height);
        let n = tl.steps();
        Ok(json!({"steps": n, "auto": (0..n).map(|i| tl.step_is_auto(i)).collect::<Vec<_>>(), "durations": (0..n).map(|i| tl.step_duration(i)).collect::<Vec<_>>()}).to_string())
    }

    /// Slide `index` with its animations at main step `step`, `t` seconds into it (`step` past the last step: all built).
    pub fn render_step(&self, index: usize, step: usize, t: f64, scale: f64) -> Result<Rendered, JsError> {
        let st = self.session.doc().map_err(js)?;
        let slide = st.doc.slides.get(index).ok_or_else(|| JsError::new("no such slide"))?;
        let tl = deckcraft_anim::Timeline::new(slide, st.doc.slide_size.width, st.doc.slide_size.height);
        let state = |id: deckcraft_model::ShapeId| {
            let a = tl.state(id, None, step, t);
            deckcraft_render::ShapeState {
                visible: a.visible,
                opacity: a.opacity,
                offset: deckcraft_geom::Vec2::new(a.offset_x, a.offset_y),
                scale: (a.scale_x, a.scale_y),
                rotate: a.rotate,
                clip: a.clip,
                tint: a.tint,
                paras: tl
                    .paragraph_targets(id)
                    .into_iter()
                    .map(|p| {
                        let a = tl.state(id, Some(p), step, t);
                        (p, deckcraft_render::ParaState { visible: a.visible, opacity: a.opacity, offset: deckcraft_geom::Vec2::new(a.offset_x, a.offset_y) })
                    })
                    .collect(),
            }
        };
        let opts = deckcraft_render::RenderOpts { scale: scale.clamp(0.05, 8.0), state: Some(&state), ..Default::default() };
        let img = deckcraft_render::render_slide(&st.doc, index, &opts);
        Ok(Rendered { width: img.width, height: img.height, rgba: img.to_straight() })
    }

    /// The frame of slide `index`'s transition at progress `t` (0 to 1): the previous slide going out, this one coming in.
    pub fn render_transition(&self, index: usize, t: f64, scale: f64) -> Result<Rendered, JsError> {
        let st = self.session.doc().map_err(js)?;
        let opts = deckcraft_render::RenderOpts { scale: scale.clamp(0.05, 8.0), ..Default::default() };
        let img = deckcraft_render::render_transition(&st.doc, index, t, &opts);
        Ok(Rendered { width: img.width, height: img.height, rgba: img.to_straight() })
    }

    /// The presentation as a PDF (`params` as for `file.export` with `format: "pdf"`: layout, perPage, slides, ...).
    pub fn export_pdf(&self, params: &str) -> Result<Vec<u8>, JsError> {
        let p: Value = serde_json::from_str(params).map_err(js)?;
        let st = self.session.doc().map_err(js)?;
        deckcraft_engine::cmd::file::pdf_bytes(&st.doc, &p).map_err(js)
    }

    /// Slide `index` as a PNG at `scale` pixels per point.
    pub fn export_png(&self, index: usize, scale: f64) -> Result<Vec<u8>, JsError> {
        let st = self.session.doc().map_err(js)?;
        if index >= st.doc.slides.len() {
            return Err(JsError::new("no such slide"));
        }
        let scale = deckcraft_engine::cmd::file::image_scale(&st.doc, scale);
        let img = deckcraft_render::render_slide(&st.doc, index, &deckcraft_render::RenderOpts { scale, ..Default::default() });
        Ok(img.to_png_at(deckcraft_render::dpi_for_scale(scale)))
    }

    /// Picks the pointer tool: `{"kind":"select"}`, `{"kind":"shape","preset":"rect"}`, `{"kind":"textBox"}` or an ink tool.
    pub fn set_tool(&mut self, tool: &str) -> Result<(), JsError> {
        let kind: deckcraft_engine::tools::ToolKind = serde_json::from_str(tool).map_err(js)?;
        self.session.set_tool(kind);
        Ok(())
    }

    /// A pointer event (see the engine's `PointerEvent`) in slide points; what the tool did.
    pub fn pointer(&mut self, event: &str) -> Result<String, JsError> {
        let ev: PointerEvent = serde_json::from_str(event).map_err(js)?;
        self.session.pointer(ev).map(|r| r.to_string()).map_err(js)
    }

    /// What the point `(x, y)` hits, with a hit tolerance of `tol` points.
    pub fn hit(&self, x: f64, y: f64, tol: f64) -> String {
        serde_json::to_string(&self.session.hit_test(deckcraft_geom::Point::new(x, y), tol)).unwrap_or_else(|_| "null".into())
    }

    /// Everything the host draws over the slide, as JSON: the slide's index and size, the selection (boxes, handles),
    /// the text being edited (caret, selected text) and the document's state.
    pub fn overlay(&self) -> String {
        overlay(&self.session).to_string()
    }
}

fn overlay(s: &Session) -> Value {
    let Ok(st) = s.doc() else { return json!({"open": false}) };
    let size = deckcraft_engine::page_size(st);
    let boxes: Vec<Value> = st
        .selection
        .shapes
        .iter()
        .filter_map(|id| st.shape(*id))
        .map(|sh| {
            let x = deckcraft_engine::cmd::xfrm_of(&st.doc, &st.selection, sh);
            let (pts, rot) = deckcraft_engine::tools::box_handles(&x);
            let a = x.affine();
            let corners: Vec<[f64; 2]> = [(0.0, 0.0), (x.w, 0.0), (x.w, x.h), (0.0, x.h)]
                .iter()
                .map(|(px, py)| {
                    let q = a * deckcraft_geom::Point::new(*px, *py);
                    [q.x, q.y]
                })
                .collect();
            json!({
                "id": sh.id,
                "x": x.x, "y": x.y, "w": x.w, "h": x.h, "rot": x.rot,
                "locked": sh.locked,
                "line": sh.is_line(),
                "handles": pts.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
                "rotate": [rot.x, rot.y],
                "corners": corners,
            })
        })
        .collect();
    json!({
        "open": true,
        "title": st.title(),
        "dirty": st.is_dirty(),
        "revision": st.revision,
        "slide": st.selection.slide,
        "slides": st.doc.slides.len(),
        "width": size.width,
        "height": size.height,
        "selection": boxes,
        "text": text_overlay(st),
        "canUndo": !st.history.undo.is_empty(),
        "canRedo": !st.history.redo.is_empty(),
        "missingFonts": st.missing_fonts,
    })
}

fn text_overlay(st: &deckcraft_engine::DocState) -> Value {
    let Some(t) = st.selection.text.as_ref() else { return Value::Null };
    let caret = deckcraft_engine::cmd::text::layout::caret_line(st, t).map(|(a, b)| json!([[a.x, a.y], [b.x, b.y]]));
    let rects: Vec<Value> = selected_rects(st, t);
    json!({"shape": t.shape, "caret": caret, "rects": rects, "anchor": [t.anchor.0, t.anchor.1], "end": [t.caret.0, t.caret.1]})
}

fn selected_rects(st: &deckcraft_engine::DocState, t: &TextSel) -> Vec<Value> {
    if !t.is_range() {
        return Vec::new();
    }
    let Some((layout, to_slide)) = deckcraft_engine::cmd::text::layout::text_frame(st, t) else { return Vec::new() };
    let a = deckcraft_text::Pos { para: t.anchor.0, ch: t.anchor.1 };
    let b = deckcraft_text::Pos { para: t.caret.0, ch: t.caret.1 };
    layout
        .selection_rects(a, b)
        .iter()
        .map(|r| {
            let p = [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)].map(|(x, y)| to_slide * deckcraft_geom::Point::new(x, y));
            json!(p.iter().map(|q| [q.x, q.y]).collect::<Vec<_>>())
        })
        .collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for b in text.bytes().filter(|b| !b.is_ascii_whitespace() && *b != b'=') {
        let v = B64.iter().position(|c| *c == b)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}
