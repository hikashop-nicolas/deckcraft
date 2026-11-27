//! Character formatting on selected shapes (no text being edited): every run, the end marks and
//! the shape's own list style, in one undo step.

use deckcraft_model::text::RunProps;
use deckcraft_model::{Shape, ShapeId, ShapeKind};
use serde_json::json;

use crate::Session;

fn id(v: serde_json::Value) -> u32 {
    v["id"].as_u64().unwrap() as u32
}

/// A text box with "Hello world" where "world" is bold (two runs), not being edited.
fn two_runs(s: &mut Session, y: f64) -> u32 {
    let b = id(s.execute("insert.textBox", &json!({"rect": [40.0, y, 300.0, 40.0], "text": "Hello world"})).unwrap());
    s.execute("text.edit", &json!({"id": b})).unwrap();
    s.execute("text.select", &json!({"anchor": [0, 6], "caret": [0, 11]})).unwrap();
    s.execute("format.bold", &json!({"on": true})).unwrap();
    s.execute("text.exit", &json!({})).unwrap();
    b
}

fn shape(s: &Session, id: u32) -> Shape {
    s.doc().unwrap().shape(ShapeId(id)).unwrap().clone()
}

fn runs(sh: &Shape) -> Vec<RunProps> {
    let mut out = Vec::new();
    fn walk(sh: &Shape, out: &mut Vec<RunProps>) {
        if let Some(t) = &sh.text {
            for p in &t.paragraphs {
                out.extend(p.runs.iter().map(|r| r.props.clone()));
                out.push(p.end_props.clone());
            }
        }
        if let ShapeKind::Group { children, .. } = &sh.kind {
            children.iter().for_each(|c| walk(c, out));
        }
    }
    walk(sh, &mut out);
    out
}

fn blank() -> Session {
    let mut s = Session::with_new();
    s.execute("slide.new", &json!({"layout": "blank"})).unwrap();
    s
}

#[test]
fn every_text_control_formats_the_whole_selected_shape_in_one_undo() {
    let mut s = blank();
    let b = two_runs(&mut s, 40.0);
    assert!(runs(&shape(&s, b)).len() >= 3, "two runs and the end mark");
    let checks: Vec<(&str, serde_json::Value, fn(&RunProps) -> bool)> = vec![
        ("format.font", json!({"family": "Ubuntu"}), |r| r.font.as_deref() == Some("Ubuntu")),
        ("format.size", json!({"size": 30}), |r| r.size == Some(30.0)),
        ("format.italic", json!({"on": true}), |r| r.italic == Some(true)),
        ("format.underline", json!({"on": true}), |r| r.underline.as_deref() == Some("sng")),
        ("format.strikethrough", json!({"on": true}), |r| r.strike.is_some()),
        ("format.color", json!({"color": "#ff0000"}), |r| r.fill.is_some()),
        ("format.highlight", json!({"color": "#ffff00"}), |r| r.highlight.is_some()),
        ("format.superscript", json!({"on": true}), |r| r.baseline == Some(0.3)),
        ("format.caps", json!({"caps": "all"}), |r| r.caps.is_some()),
        ("format.spacing", json!({"pt": 3}), |r| r.spacing == Some(3.0)),
    ];
    for (cmd, p, ok) in checks {
        s.execute("edit.select", &json!({"ids": [b]})).unwrap();
        assert!(s.doc().unwrap().selection.text.is_none());
        let undo = s.doc().unwrap().history.undo.len();
        let before = shape(&s, b);
        s.execute(cmd, &p).unwrap();
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{cmd}: one undo step");
        let all = runs(&shape(&s, b));
        assert!(all.iter().all(ok), "{cmd} on every run and end mark: {all:?}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(shape(&s, b).text, before.text, "{cmd}: undo puts the text back");
        s.execute(cmd, &p).unwrap();
    }
}

#[test]
fn bold_on_a_shape_with_mixed_runs_makes_every_run_bold() {
    let mut s = blank();
    let b = two_runs(&mut s, 40.0);
    s.execute("edit.select", &json!({"ids": [b]})).unwrap();
    s.execute("format.bold", &json!({})).unwrap();
    assert!(runs(&shape(&s, b)).iter().all(|r| r.bold == Some(true)));
}

#[test]
fn several_selected_shapes_change_together() {
    let mut s = blank();
    let a = two_runs(&mut s, 40.0);
    let b = two_runs(&mut s, 120.0);
    let r = id(s.execute("shape.insert", &json!({"preset": "rect", "rect": [40.0, 200.0, 200.0, 80.0], "text": "Rect"})).unwrap());
    s.execute("text.exit", &json!({})).unwrap();
    s.execute("edit.select", &json!({"ids": [a, b, r]})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("format.font", &json!({"family": "Ubuntu"})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    for x in [a, b, r] {
        assert!(runs(&shape(&s, x)).iter().all(|p| p.font.as_deref() == Some("Ubuntu")), "shape {x}");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    for x in [a, b, r] {
        assert!(runs(&shape(&s, x)).iter().all(|p| p.font.is_none()), "shape {x} after undo");
    }
}

#[test]
fn a_selected_group_formats_its_members_text() {
    let mut s = blank();
    let a = two_runs(&mut s, 40.0);
    let b = two_runs(&mut s, 120.0);
    let pic = id(s.execute("shape.insert", &json!({"preset": "line", "rect": [40.0, 300.0, 200.0, 0.0]})).unwrap());
    let g = id(s.execute("arrange.group", &json!({"ids": [a, b, pic]})).unwrap());
    s.execute("edit.select", &json!({"ids": [g]})).unwrap();
    s.execute("format.font", &json!({"family": "Ubuntu"})).unwrap();
    s.execute("format.size", &json!({"size": 12})).unwrap();
    let group = shape(&s, g);
    let all = runs(&group);
    assert!(all.len() >= 6);
    assert!(all.iter().all(|r| r.font.as_deref() == Some("Ubuntu") && r.size == Some(12.0)), "{all:?}");
    assert!(group.text.is_none(), "the group itself gets no text body");
    let ShapeKind::Group { children, .. } = &group.kind else { panic!("group") };
    assert!(children.iter().find(|c| c.id == ShapeId(pic)).unwrap().text.is_none(), "a line in the group gets no text");
    let st: serde_json::Value = s.execute("format.state", &json!({})).unwrap();
    assert_eq!(st["font"], "Ubuntu");
}

#[test]
fn an_empty_placeholder_keeps_the_font_for_its_text() {
    let mut s = Session::with_new();
    let sub = s.doc().unwrap().doc.slides[0].shapes.iter().find(|x| x.name.starts_with("Subtitle")).unwrap().id;
    s.execute("edit.select", &json!({"ids": [sub.0]})).unwrap();
    s.execute("format.font", &json!({"family": "Ubuntu"})).unwrap();
    s.execute("format.bold", &json!({"on": true})).unwrap();
    let st: serde_json::Value = s.execute("format.state", &json!({})).unwrap();
    assert_eq!(st["font"], "Ubuntu");
    assert_eq!(st["bold"], true);
    // Typed text takes it.
    s.execute("text.edit", &json!({"id": sub.0})).unwrap();
    s.execute("text.insert", &json!({"text": "Typed"})).unwrap();
    s.execute("text.exit", &json!({})).unwrap();
    let sh = shape(&s, sub.0);
    let r = &sh.text.as_ref().unwrap().paragraphs[0].runs[0];
    assert_eq!(r.text, "Typed");
    assert_eq!(r.props.font.as_deref(), Some("Ubuntu"));
    assert_eq!(r.props.bold, Some(true));
}

#[test]
fn the_shapes_own_list_style_follows() {
    let mut s = blank();
    let b = two_runs(&mut s, 40.0);
    s.edit(|doc, sel| {
        let sh = deckcraft_model::find_shape_mut(crate::shapes_mut(doc, sel).unwrap(), ShapeId(b)).unwrap();
        let ls = &mut sh.text.as_mut().unwrap().list_style;
        ls.levels = vec![Some(Default::default())];
        Ok(())
    })
    .unwrap();
    s.execute("edit.select", &json!({"ids": [b]})).unwrap();
    s.execute("format.font", &json!({"family": "Ubuntu"})).unwrap();
    let sh = shape(&s, b);
    let lv = sh.text.as_ref().unwrap().list_style.levels[0].as_ref().unwrap();
    assert_eq!(lv.run.font.as_deref(), Some("Ubuntu"));
}

#[test]
fn grow_steps_each_shape_from_its_own_inherited_size() {
    let mut s = Session::with_new();
    let shapes = s.doc().unwrap().doc.slides[0].shapes.clone();
    let (t, sub) = (shapes[0].id, shapes[1].id);
    for (x, text) in [(t, "Title"), (sub, "Sub")] {
        s.execute("text.edit", &json!({"id": x.0})).unwrap();
        s.execute("text.insert", &json!({"text": text})).unwrap();
        s.execute("text.exit", &json!({})).unwrap();
    }
    let size = |s: &mut Session, x: ShapeId| -> f64 {
        s.execute("edit.select", &json!({"ids": [x.0]})).unwrap();
        s.execute("format.state", &json!({})).unwrap()["size"].as_f64().unwrap()
    };
    let (t0, s0) = (size(&mut s, t), size(&mut s, sub));
    assert!(t0 > s0, "title {t0} larger than subtitle {s0}");
    s.execute("edit.select", &json!({"ids": [t.0, sub.0]})).unwrap();
    s.execute("format.grow", &json!({})).unwrap();
    let (t1, s1) = (size(&mut s, t), size(&mut s, sub));
    assert!(t1 > t0 && s1 > s0 && s1 < t1, "title {t0}→{t1}, subtitle {s0}→{s1}");
}

#[test]
fn editing_text_still_formats_only_the_selection() {
    let mut s = blank();
    let b = two_runs(&mut s, 40.0);
    s.execute("text.edit", &json!({"id": b})).unwrap();
    s.execute("text.select", &json!({"anchor": [0, 0], "caret": [0, 5]})).unwrap();
    s.execute("format.font", &json!({"family": "Ubuntu"})).unwrap();
    let sh = shape(&s, b);
    let p = &sh.text.as_ref().unwrap().paragraphs[0];
    assert_eq!(p.runs[0].props.font.as_deref(), Some("Ubuntu"));
    assert!(p.runs.iter().skip(1).all(|r| r.props.font.is_none()));
}
