use super::layout::fit_text_box;
use super::*;

/// Apply run formatting to the text selection (or, when not editing, to all text of the
/// selected shapes). A caret without a range formats the word it's in (PowerPoint behaviour),
/// or sets the formatting for what is typed next at a word boundary.
pub(crate) fn format_run(s: &mut Session, f: &(dyn Fn(&mut RunProps) + Send + Sync)) -> Result<Value> {
    format_run_with(s, &|r, _| f(r))
}

/// [`format_run`] whose setter also gets each run's effective props (inherited values resolved).
/// While editing that is the props at the selection.
pub(crate) fn format_run_with(s: &mut Session, f: &(dyn Fn(&mut RunProps, &RunProps) + Send + Sync)) -> Result<Value> {
    let st = s.doc()?;
    if let Some(t) = st.selection.text.clone() {
        let (a, b) = format_range(st, &t);
        let shown = crate::cmd::format::selection_run(s);
        edit_text(s, |body, _| ed::format(body, a, b, &|r| f(r, &shown)))?;
        return ok();
    }
    let ids = st.selection.shapes.clone();
    if ids.is_empty() {
        return Err(bad("format", SELECT_TEXT_OR_SHAPE));
    }
    let cells = st.selection.cells;
    // The text as it was, to resolve inherited props against.
    let old = st.doc.clone();
    let sel = st.selection.clone();
    let ctx = ctx_for(&old, &sel);
    let shapes = crate::cmd::current_shapes(&old, &sel);
    edit_shapes(s, &json!({"ids": ids}), "format", |sh| {
        let before = shapes.and_then(|v| deckcraft_model::find_shape(v, sh.id));
        format_shape(sh, before, ctx.as_ref(), cells, f, 0);
        Ok(())
    })?;
    for id in ids {
        fit_text_box(s, id)?;
    }
    ok()
}

/// Every text of a selected shape: its runs, end marks and own list style; a table's selected
/// cells; a group's members (their own text only).
fn format_shape(
    sh: &mut Shape,
    before: Option<&Shape>,
    ctx: Option<&Ctx>,
    cells: Option<(ShapeId, (usize, usize), (usize, usize))>,
    f: &dyn Fn(&mut RunProps, &RunProps),
    depth: usize,
) {
    let id = sh.id;
    match &mut sh.kind {
        ShapeKind::Table(t) => {
            let range = crate::cmd::table::selected_range(cells, id, t);
            let old = match before.map(|b| &b.kind) {
                Some(ShapeKind::Table(o)) => Some(o),
                _ => None,
            };
            for (r, row) in t.rows.iter_mut().enumerate() {
                for (c, cell) in row.cells.iter_mut().enumerate() {
                    if crate::cmd::table::in_range(range, r, c) {
                        let ob = old.and_then(|o| o.rows.get(r)).and_then(|row| row.cells.get(c)).map(|c| &c.text);
                        format_body_all(&mut cell.text, ob, before, ctx, f);
                    }
                }
            }
        }
        ShapeKind::Group { children, .. } => {
            if depth > 64 {
                return;
            }
            let old = match before.map(|b| &b.kind) {
                Some(ShapeKind::Group { children, .. }) => Some(children),
                _ => None,
            };
            for (i, child) in children.iter_mut().enumerate() {
                let ob = old.and_then(|o| o.get(i));
                if child.text.is_some() || matches!(child.kind, ShapeKind::Group { .. } | ShapeKind::Table(_)) {
                    format_shape(child, ob, ctx, None, f, depth + 1);
                }
            }
        }
        // A picture, chart or media clip holds no text of its own.
        ShapeKind::Picture { .. } | ShapeKind::Chart(_) | ShapeKind::Media(_) | ShapeKind::Model3d { .. } | ShapeKind::Opaque { .. }
            if sh.text.is_none() => {}
        _ => {
            let ob = before.and_then(|b| b.text.as_ref());
            let body = sh.text.get_or_insert_with(TextBody::default);
            // An empty placeholder keeps the formatting on an end mark, for the text typed in it.
            if body.paragraphs.is_empty() {
                body.paragraphs.push(Paragraph::default());
            }
            format_body_all(body, ob, before, ctx, f);
        }
    }
}

/// [`ed::format_all`] with each run's effective props, plus the body's own list style levels.
fn format_body_all(body: &mut TextBody, before: Option<&TextBody>, shape: Option<&Shape>, ctx: Option<&Ctx>, f: &dyn Fn(&mut RunProps, &RunProps)) {
    let blank = Paragraph::default();
    let shown = |pi: usize, r: Option<&RunProps>, end: bool| -> RunProps {
        let para = before.and_then(|b| b.paragraphs.get(pi)).unwrap_or(&blank);
        let own = r.cloned().unwrap_or_else(|| if end { para.end_props.clone() } else { RunProps::default() });
        match (ctx, shape) {
            (Some(c), Some(sh)) => resolve::run(c, sh, para, &own),
            _ => own,
        }
    };
    for (pi, p) in body.paragraphs.iter_mut().enumerate() {
        let old = before.and_then(|b| b.paragraphs.get(pi));
        for (ri, r) in p.runs.iter_mut().enumerate() {
            let res = shown(pi, old.and_then(|o| o.runs.get(ri)).map(|x| &x.props), false);
            f(&mut r.props, &res);
        }
        let res = shown(pi, None, true);
        f(&mut p.end_props, &res);
        p.normalize();
    }
    for lv in body.list_style.levels.iter_mut().flatten() {
        let own = lv.run.clone();
        f(&mut lv.run, &own);
    }
}

/// What `format_run` formats while editing: the selection, or for a bare caret the word it's in
/// (without its trailing space). Empty when the caret is at a word boundary. Insert Hyperlink
/// edits this same span, so the dialog reads it too.
pub fn format_range(st: &DocState, t: &TextSel) -> (Pos, Pos) {
    let body = body_of(st, t).cloned().unwrap_or_default();
    let (mut a, mut b) = t.ordered();
    if a == b {
        let (wa, wb) = ed::word_at(&body, a);
        let inside = wa.1 < a.1 && a.1 < wb.1;
        if inside {
            a = wa;
            b = wb;
            // Don't include the trailing space.
            let line: Vec<char> = body.paragraphs.get(b.0).map(|p| p.text().chars().collect()).unwrap_or_default();
            while b.1 > a.1 && line.get(b.1 - 1) == Some(&' ') {
                b.1 -= 1;
            }
        }
    }
    (a, b)
}

/// Link the text being edited: the selection or the word at the caret, as `format_run` would.
/// With nothing to link (caret at a word boundary, empty paragraph) `display` is inserted at the
/// caret as linked text in the surrounding formatting, as PowerPoint's Insert Hyperlink does.
pub(crate) fn link_text(s: &mut Session, link: Hyperlink, display: &str) -> Result<Value> {
    let st = s.doc()?;
    let Some(t) = st.selection.text.clone() else { return Err(bad("insert.hyperlink", "not editing text")) };
    let (a, b) = format_range(st, &t);
    if a != b {
        return format_run(s, &move |rp| rp.link = Some(link.clone()));
    }
    if display.is_empty() {
        return Err(bad("insert.hyperlink", "nothing to link: select text or give `text`"));
    }
    let pos = edit_text(s, |body, t| {
        let mut props = ed::props_at(body, a);
        props.link = Some(link);
        let c = ed::insert(body, a, display, Some(props));
        t.anchor = c;
        t.caret = c;
        c
    })?;
    Ok(json!({"caret": [pos.0, pos.1]}))
}

/// Apply paragraph changes to the paragraphs touched by the selection (or all paragraphs of the
/// selected shapes).
pub(crate) fn format_para(s: &mut Session, f: &(dyn Fn(&mut deckcraft_model::text::Paragraph) + Send + Sync)) -> Result<Value> {
    let st = s.doc()?;
    if let Some(t) = st.selection.text.clone() {
        let (a, b) = t.ordered();
        edit_text(s, |body, _| {
            if body.paragraphs.is_empty() {
                body.paragraphs.push(Default::default());
            }
            let last = body.paragraphs.len().saturating_sub(1);
            for pi in a.0..=b.0.min(last) {
                if let Some(p) = body.paragraphs.get_mut(pi) {
                    f(p);
                }
            }
        })?;
        return ok();
    }
    let ids = st.selection.shapes.clone();
    if ids.is_empty() {
        return Err(bad("format", SELECT_TEXT_OR_SHAPE));
    }
    let cells = st.selection.cells;
    edit_shapes(s, &json!({"ids": ids}), "format", |sh| {
        let id = sh.id;
        match &mut sh.kind {
            ShapeKind::Table(t) => {
                let range = crate::cmd::table::selected_range(cells, id, t);
                for (r, row) in t.rows.iter_mut().enumerate() {
                    for (c, cell) in row.cells.iter_mut().enumerate() {
                        if crate::cmd::table::in_range(range, r, c) {
                            cell.text.paragraphs.iter_mut().for_each(f);
                        }
                    }
                }
            }
            _ => {
                let body = sh.text.get_or_insert_with(TextBody::default);
                if body.paragraphs.is_empty() {
                    body.paragraphs.push(Default::default());
                }
                body.paragraphs.iter_mut().for_each(f);
            }
        }
        Ok(())
    })?;
    for id in ids {
        fit_text_box(s, id)?;
    }
    ok()
}

/// Apply body (text box) changes to the edited or selected shapes.
pub(crate) fn format_body(s: &mut Session, f: &(dyn Fn(&mut deckcraft_model::text::BodyProps) + Send + Sync)) -> Result<Value> {
    let st = s.doc()?;
    let ids: Vec<ShapeId> = match &st.selection.text {
        Some(t) if !t.notes => vec![t.shape],
        _ => st.selection.shapes.clone(),
    };
    if ids.is_empty() {
        return Err(bad("format", "select a shape"));
    }
    edit_shapes(s, &json!({"ids": ids}), "format", |sh| {
        f(&mut sh.text.get_or_insert_with(TextBody::default).body);
        Ok(())
    })?;
    for id in ids {
        fit_text_box(s, id)?;
    }
    ok()
}

pub(super) fn fonts_used(s: &mut Session, _: &Value) -> Result<Value> {
    let mut v = crate::fonts::used_fonts(&s.doc()?.doc);
    v.sort_by_key(|f| f.to_lowercase());
    Ok(json!(v))
}

pub(super) fn replace_font(s: &mut Session, p: &Value) -> Result<Value> {
    let from = str_param(p, "from").map(str::trim).filter(|x| !x.is_empty()).ok_or_else(|| bad("font.replace", "missing `from`"))?.to_string();
    let to = str_param(p, "to").map(str::trim).filter(|x| !x.is_empty()).ok_or_else(|| bad("font.replace", "missing `to`"))?.to_string();
    if to.len() > 256 || to.chars().any(char::is_control) {
        return Err(bad("font.replace", "`to` is not a font name"));
    }
    // Count on a copy first so a replace that finds nothing leaves no undo step.
    let mut probe = (*s.doc()?.doc).clone();
    if crate::fonts::replace_font(&mut probe, &from, &to) == 0 {
        return Ok(json!({"replaced": 0}));
    }
    let n = s.edit(|doc, _| Ok(crate::fonts::replace_font(doc, &from, &to)))?;
    let st = s.doc_mut()?;
    st.missing_fonts = crate::fonts::missing_fonts(&st.doc);
    Ok(json!({"replaced": n}))
}

#[cfg(test)]
mod replace_font_tests {
    use serde_json::json;

    use crate::Session;

    fn fonts(s: &mut Session) -> Vec<String> {
        serde_json::from_value(s.execute("font.used", &json!({})).unwrap()).unwrap()
    }

    #[test]
    fn replaces_runs_tables_notes_and_theme_and_undoes() {
        let mut s = Session::with_new();
        s.execute("slide.new", &json!({"layout": "titleAndContent", "title": "T", "body": "B"})).unwrap();
        let theme = s.doc().unwrap().doc.masters[0].theme.fonts.minor.latin.clone();
        s.execute("insert.textBox", &json!({"rect": [10, 10, 200, 50], "text": "Hello"})).unwrap();
        s.execute("text.exit", &json!({})).unwrap();
        s.execute("format.font", &json!({"family": "Comic Neue"})).unwrap();
        assert!(fonts(&mut s).iter().any(|f| f == "Comic Neue"), "{:?}", fonts(&mut s));
        let r = s.execute("font.replace", &json!({"from": "comic neue", "to": "Lato"})).unwrap();
        assert!(r["replaced"].as_u64().unwrap() >= 1, "{r}");
        let used = fonts(&mut s);
        assert!(!used.iter().any(|f| f == "Comic Neue") && used.iter().any(|f| f == "Lato"), "{used:?}");
        // The theme's body font is replaced too, so placeholders follow.
        s.execute("font.replace", &json!({"from": theme, "to": "Lato"})).unwrap();
        assert_eq!(s.doc().unwrap().doc.masters[0].theme.fonts.minor.latin, "Lato");
        assert!(s.doc().unwrap().doc.validate().is_empty());
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(fonts(&mut s).iter().any(|f| f == "Comic Neue"));
    }

    #[test]
    fn nothing_to_replace_is_no_undo_step_and_bad_names_fail() {
        let mut s = Session::with_new();
        let undo = s.doc().unwrap().history.undo.len();
        assert_eq!(s.execute("font.replace", &json!({"from": "Nope Sans", "to": "Inter"})).unwrap()["replaced"], 0);
        assert_eq!(s.execute("font.replace", &json!({"from": "+mn-lt", "to": "Inter"})).unwrap()["replaced"], 0);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
        assert!(s.execute("font.replace", &json!({"from": "", "to": "Inter"})).is_err());
        assert!(s.execute("font.replace", &json!({"from": "Inter", "to": "a\nb"})).is_err());
        assert!(s.execute("font.replace", &json!({"from": "Inter"})).is_err());
    }
}
