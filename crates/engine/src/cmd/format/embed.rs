//! File ▸ Options ▸ Save ▸ Embed fonts in the file, with the font files from the host: each face
//! becomes an `application/x-fontdata` part (Embedded OpenType, as PowerPoint writes them) linked
//! from `p:embeddedFontLst`.

use deckcraft_fonts::{EmbedError, Embedding};
use deckcraft_model::{EmbeddedFont, MediaId};

use super::*;

/// The four face slots: parameter name and setter.
const SLOTS: [&str; 4] = ["regular", "bold", "italic", "boldItalic"];

fn slot_mut(e: &mut EmbeddedFont, i: usize) -> &mut Option<MediaId> {
    match i {
        0 => &mut e.regular,
        1 => &mut e.bold,
        2 => &mut e.italic,
        _ => &mut e.bold_italic,
    }
}

fn reason(e: EmbedError) -> &'static str {
    match e {
        EmbedError::NotAFont => "notAFont",
        EmbedError::BitmapOnly => "bitmapOnly",
        EmbedError::NotAllowed(Embedding::PreviewPrint) => "previewPrint",
        EmbedError::NotAllowed(_) => "restricted",
    }
}

fn family_param<'a>(id: &str, p: &'a Value) -> Result<&'a str> {
    let f = str_param(p, "family").map(str::trim).unwrap_or("");
    if f.is_empty() || f.chars().count() > 255 {
        return Err(bad(id, "`family` is the typeface name the text uses"));
    }
    Ok(f)
}

/// Drop font parts nothing references any more.
fn prune(doc: &mut deckcraft_model::Presentation, dropped: &[MediaId]) {
    if dropped.is_empty() {
        return;
    }
    let used = crate::media_refs::presentation_media(doc);
    doc.media.retain(|m| !dropped.contains(&m.id) || used.contains(&m.id));
}

/// `{family, regular?, bold?, italic?, boldItalic?}`: base64 TrueType or OpenType files.
pub(super) fn font_embed(s: &mut Session, p: &Value) -> Result<Value> {
    let family = family_param("font.embed", p)?.to_string();
    let mut faces: Vec<(usize, Vec<u8>, Vec<u8>)> = Vec::new();
    let mut skipped = Vec::new();
    for (i, key) in SLOTS.iter().enumerate() {
        let Some(v) = p.get(*key).filter(|v| !v.is_null()) else { continue };
        let data = v.as_str().and_then(base64_decode).ok_or_else(|| bad("font.embed", format!("`{key}` is not base64 font data")))?;
        if data.len() > 32 << 20 {
            skipped.push(json!({"face": key, "reason": "tooLarge"}));
            continue;
        }
        // An EOT part from another file is unwrapped first, so its rights are checked again; a
        // collection gives the face `<slot>Index` names.
        let index = p.get(format!("{key}Index")).and_then(Value::as_u64).unwrap_or(0).min(u64::from(u32::MAX)) as u32;
        let raw = deckcraft_fonts::embedded_sfnt(&data).unwrap_or(&data);
        let Some(sfnt) = deckcraft_fonts::single_font(raw, index) else {
            skipped.push(json!({"face": key, "reason": "notAFont"}));
            continue;
        };
        match deckcraft_fonts::eot_wrap(&sfnt) {
            Ok(eot) => faces.push((i, sfnt, eot)),
            Err(e) => skipped.push(json!({"face": key, "reason": reason(e)})),
        }
    }
    if faces.is_empty() {
        let why = skipped.first().and_then(|x| x["reason"].as_str()).unwrap_or("none");
        return Err(bad("font.embed", format!("no face to embed ({why})")));
    }
    let names: Vec<&str> = faces.iter().map(|(i, ..)| SLOTS[*i]).collect();
    let parts: Vec<(usize, Vec<u8>)> = faces.iter().map(|(i, _, eot)| (*i, eot.clone())).collect();
    let fam = family.clone();
    s.edit(move |doc, _| {
        let mut dropped = Vec::new();
        let at = match doc.embedded_fonts.iter().position(|e| e.typeface.eq_ignore_ascii_case(&fam)) {
            Some(i) => i,
            None => {
                if doc.embedded_fonts.len() >= 64 {
                    return Err(bad("font.embed", "a presentation holds at most 64 embedded fonts"));
                }
                doc.embedded_fonts.push(EmbeddedFont { typeface: fam.clone(), ..Default::default() });
                doc.embedded_fonts.len() - 1
            }
        };
        for (i, eot) in parts {
            let id = doc.add_media("font.fntdata", "application/x-fontdata", eot).ok_or_else(|| bad("font.embed", "too many parts"))?;
            let e = &mut doc.embedded_fonts[at];
            if let Some(old) = slot_mut(e, i).replace(id)
                && old != id
            {
                dropped.push(old);
            }
        }
        prune(doc, &dropped);
        Ok(())
    })?;
    let db = deckcraft_fonts::FontDb::global();
    for (_, sfnt, _) in faces {
        db.add_font(sfnt);
    }
    Ok(json!({"family": family, "faces": names, "skipped": skipped}))
}

/// `{family}`: the file stops carrying that font.
pub(super) fn font_unembed(s: &mut Session, p: &Value) -> Result<Value> {
    let family = family_param("font.unembed", p)?.to_string();
    if !s.doc()?.doc.embedded_fonts.iter().any(|e| e.typeface.eq_ignore_ascii_case(&family)) {
        return Ok(json!({"removed": false}));
    }
    s.edit(move |doc, _| {
        let mut dropped = Vec::new();
        doc.embedded_fonts.retain(|e| {
            let hit = e.typeface.eq_ignore_ascii_case(&family);
            if hit {
                dropped.extend(e.faces().into_iter().flatten());
            }
            !hit
        });
        prune(doc, &dropped);
        Ok(())
    })?;
    Ok(json!({"removed": true}))
}

/// `{}` → `[{family, faces, drawable}]`: the fonts the file carries, and whether their faces could
/// be read (a MicroType Express subset can't, and is only kept for the round trip).
pub(super) fn font_embedded(s: &mut Session, _p: &Value) -> Result<Value> {
    let doc = &s.doc()?.doc;
    let list: Vec<Value> = doc
        .embedded_fonts
        .iter()
        .map(|e| {
            let faces: Vec<&str> = e.faces().iter().enumerate().filter(|(_, x)| x.is_some()).map(|(i, _)| SLOTS[i]).collect();
            let drawable = e.faces().into_iter().flatten().any(|id| doc.media(id).is_some_and(|m| deckcraft_fonts::embedded_sfnt(&m.data).is_some()));
            json!({"family": e.typeface, "faces": faces, "drawable": drawable})
        })
        .collect();
    Ok(Value::Array(list))
}
