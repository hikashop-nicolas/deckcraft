use super::*;

pub(super) fn crop(s: &mut Session, p: &Value) -> Result<Value> {
    let c = [f64_param(p, "left"), f64_param(p, "top"), f64_param(p, "right"), f64_param(p, "bottom")];
    edit_geometry(s, p, "picture.crop", |sh| {
        if let deckcraft_model::ShapeKind::Picture { fill } = &mut sh.kind {
            for (i, v) in c.iter().enumerate() {
                if let (Some(v), Some(slot)) = (v, fill.crop.get_mut(i)) {
                    *slot = v.clamp(-10.0, 0.95);
                }
            }
        }
        Ok(())
    })
}

pub(super) fn picture_adjust(s: &mut Session, p: &Value) -> Result<Value> {
    edit_shapes(s, p, "picture.adjust", |sh| {
        if let deckcraft_model::ShapeKind::Picture { fill } = &mut sh.kind {
            let a = &mut fill.adjust;
            if bool_or(p, "reset", false) {
                *a = Default::default();
            }
            if let Some(v) = f64_param(p, "brightness") {
                a.brightness = v.clamp(-1.0, 1.0);
            }
            if let Some(v) = f64_param(p, "contrast") {
                a.contrast = v.clamp(-1.0, 1.0);
            }
            if let Some(v) = f64_param(p, "saturation") {
                a.saturation = Some(v.clamp(0.0, 4.0));
            }
            if let Some(v) = bool_param(p, "grayscale") {
                a.grayscale = v;
            }
            if let Some(v) = f64_param(p, "transparency") {
                fill.alpha = Some((1.0 - v).clamp(0.0, 1.0));
            }
        }
        Ok(())
    })
}

pub(super) fn picture_reset(s: &mut Session, p: &Value) -> Result<Value> {
    edit_shapes(s, p, "picture.reset", |sh| {
        if let deckcraft_model::ShapeKind::Picture { fill } = &mut sh.kind {
            fill.adjust = Default::default();
            fill.crop = [0.0; 4];
            fill.alpha = None;
            if let Some(a) = fill.artistic.take() {
                fill.media = a.original;
            }
        }
        sh.effects = None;
        sh.line = None;
        Ok(())
    })
}

pub(super) fn picture_change(s: &mut Session, p: &Value) -> Result<Value> {
    let (name, bytes) = crate::cmd::insert::media_bytes(p, "picture.change")?;
    let ct = crate::cmd::insert::content_type(&name, &bytes);
    let media = s.edit(|doc, _| doc.add_media(&name, ct, bytes).ok_or_else(|| bad("picture.change", NO_ID_SPACE)))?;
    edit_shapes(s, p, "picture.change", |sh| {
        if let deckcraft_model::ShapeKind::Picture { fill } = &mut sh.kind {
            fill.media = media;
            fill.crop = [0.0; 4];
            fill.artistic = None;
        }
        Ok(())
    })
}

pub(super) fn media_options(s: &mut Session, p: &Value) -> Result<Value> {
    edit_shapes(s, p, "media.options", |sh| {
        if let deckcraft_model::ShapeKind::Media(m) = &mut sh.kind {
            if let Some(v) = bool_param(p, "autoplay") {
                m.autoplay = v;
            }
            if let Some(v) = bool_param(p, "loop") {
                m.loop_play = v;
            }
            if let Some(v) = bool_param(p, "rewind") {
                m.rewind = v;
            }
            if let Some(v) = bool_param(p, "acrossSlides") {
                m.play_across_slides = v;
            }
            if let Some(v) = bool_param(p, "hide") {
                m.hide_while_not_playing = v;
            }
            if let Some(v) = bool_param(p, "fullScreen") {
                m.full_screen = v;
            }
            if let Some(v) = f64_param(p, "volume") {
                m.volume = v.clamp(0.0, 1.0);
            }
            let ms = |k: &str| p.get(k).and_then(Value::as_u64).map(|v| v.min(u32::MAX as u64) as u32);
            if let Some(v) = ms("trimStart") {
                m.trim_start_ms = v;
            }
            if let Some(v) = ms("trimEnd") {
                m.trim_end_ms = v;
            }
            if let Some(v) = ms("fadeIn") {
                m.fade_in_ms = v;
            }
            if let Some(v) = ms("fadeOut") {
                m.fade_out_ms = v;
            }
        }
        Ok(())
    })?;
    if bool_param(p, "autoplay") == Some(false) {
        stop_autoplay_effects(s)?;
    }
    Ok(json!({}))
}

/// An audio or video that no longer autoplays loses the Play effect that starts it by itself (a file keeps autoplay as that effect).
fn stop_autoplay_effects(s: &mut Session) -> Result<()> {
    use deckcraft_model::{AnimClass, AnimStart, Animation, ShapeKind, Slide};
    let stale = |sl: &Slide, a: &Animation| {
        a.class == AnimClass::Media
            && a.effect == "play"
            && a.start != AnimStart::OnClick
            && a.trigger.is_none()
            && deckcraft_model::find_shape(&sl.shapes, a.shape).is_some_and(|sh| matches!(&sh.kind, ShapeKind::Media(m) if !m.autoplay))
    };
    if !s.doc()?.doc.slides.iter().any(|sl| sl.animations.iter().any(|a| stale(sl, a))) {
        return Ok(());
    }
    s.edit(|doc, _| {
        for sl in doc.slides.iter_mut() {
            let keep: Vec<bool> = sl.animations.iter().map(|a| !stale(sl, a)).collect();
            if keep.iter().all(|k| *k) {
                continue;
            }
            let mut i = 0;
            std::sync::Arc::make_mut(sl).animations.retain(|_| {
                i += 1;
                keep[i - 1]
            });
        }
        Ok(())
    })?;
    Ok(())
}
