//! Embedding font files from the host in the presentation.

use serde_json::json;

use crate::Session;
use crate::cmd::base64_encode;

const RAW: &[u8] = include_bytes!("../../../../render/tests/fixtures/colr-squares.ttf");
const FAMILY: &str = "DeckCraft Colr Squares";

/// The fixture (preview-and-print only) with another `fsType` (OS/2 at 328, fsType at +8).
fn with_fs_type(fs: u16) -> Vec<u8> {
    let mut f = RAW.to_vec();
    f[336..338].copy_from_slice(&fs.to_be_bytes());
    f
}

fn session() -> Session {
    let mut s = Session::with_new();
    let id = s.execute("insert.textBox", &json!({"rect": [40, 40, 300, 60], "text": "Hello"})).unwrap()["id"].clone();
    s.execute("text.exit", &json!({})).unwrap();
    s.execute("edit.select", &json!({"ids": [id]})).unwrap();
    s.execute("format.font", &json!({"family": FAMILY})).unwrap();
    s
}

fn save(s: &mut Session) -> Vec<u8> {
    crate::cmd::file::save_bytes(&s.doc().unwrap().doc, "pptx").unwrap()
}

#[test]
fn an_embedded_font_is_saved_as_eot_and_reopens() {
    let mut s = session();
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("font.embed", &json!({"family": FAMILY, "regular": base64_encode(&with_fs_type(0))})).unwrap();
    assert_eq!(r["faces"], json!(["regular"]));
    assert_eq!(r["skipped"], json!([]));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let list = s.execute("font.embedded", &json!({})).unwrap();
    assert_eq!(list, json!([{"family": FAMILY, "faces": ["regular"], "drawable": true}]));

    let bytes = save(&mut s);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut part = Vec::new();
    std::io::Read::read_to_end(&mut zip.by_name("ppt/fonts/font1.fntdata").unwrap(), &mut part).unwrap();
    assert_eq!(u16::from_le_bytes([part[34], part[35]]), 0x504C, "an Embedded OpenType part");
    assert_eq!(deckcraft_fonts::embedded_sfnt(&part), Some(&with_fs_type(0)[..]));
    let mut pres = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("ppt/presentation.xml").unwrap(), &mut pres).unwrap();
    assert!(pres.contains("embedTrueTypeFonts=\"1\""), "{pres}");
    assert!(pres.contains(&format!("<p:font typeface=\"{FAMILY}\"")), "{pres}");

    let mut again = Session::with_new();
    let r = again.execute("file.openBytes", &json!({"name": "e.pptx", "data": base64_encode(&bytes)})).unwrap();
    assert_eq!(r["missingFonts"], json!([]));
    assert_eq!(again.execute("font.embedded", &json!({})).unwrap()[0]["family"], FAMILY);
    // Saved again unchanged, the part survives.
    let twice = save(&mut again);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&twice)).unwrap();
    assert!(zip.by_name("ppt/fonts/font1.fntdata").is_ok());
}

#[test]
fn undo_and_unembed_take_the_font_out() {
    let mut s = session();
    s.execute("font.embed", &json!({"family": FAMILY, "regular": base64_encode(&with_fs_type(0))})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.execute("font.embedded", &json!({})).unwrap(), json!([]));
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.execute("font.embedded", &json!({})).unwrap().as_array().unwrap().len(), 1);
    let media = s.doc().unwrap().doc.media.len();
    assert_eq!(s.execute("font.unembed", &json!({"family": FAMILY.to_lowercase()})).unwrap()["removed"], true);
    assert_eq!(s.execute("font.embedded", &json!({})).unwrap(), json!([]));
    assert_eq!(s.doc().unwrap().doc.media.len(), media - 1, "the part goes with it");
    assert_eq!(s.execute("font.unembed", &json!({"family": FAMILY})).unwrap()["removed"], false);
    let bytes = save(&mut s);
    let zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    assert!(!zip.file_names().any(|n| n.ends_with(".fntdata")));
}

#[test]
fn faces_go_in_their_slots_and_a_second_embed_replaces_one() {
    let mut s = session();
    let b64 = base64_encode(&with_fs_type(0));
    assert!(s.execute("font.embed", &json!({"family": FAMILY, "regular": base64_encode(RAW)})).is_err(), "the fixture is print-only");
    s.execute("font.embed", &json!({"family": FAMILY, "regular": b64, "bold": base64_encode(&with_fs_type(8))})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let e = &doc.embedded_fonts[0];
    assert!(e.regular.is_some() && e.bold.is_some() && e.italic.is_none() && e.bold_italic.is_none());
    let media = doc.media.len();
    s.execute("font.embed", &json!({"family": FAMILY, "bold": base64_encode(&with_fs_type(0x0100))})).unwrap();
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.embedded_fonts.len(), 1);
    assert_eq!(doc.media.len(), media, "the old bold part is dropped");
}

#[test]
fn restricted_and_print_only_faces_are_refused() {
    let mut s = session();
    let err = s.execute("font.embed", &json!({"family": FAMILY, "regular": base64_encode(&with_fs_type(2))})).unwrap_err();
    assert!(err.to_string().contains("restricted"), "{err}");
    let err = s.execute("font.embed", &json!({"family": FAMILY, "regular": base64_encode(&with_fs_type(4))})).unwrap_err();
    assert!(err.to_string().contains("previewPrint"), "{err}");
    let r = s
        .execute("font.embed", &json!({"family": FAMILY, "regular": base64_encode(&with_fs_type(0)), "italic": base64_encode(&with_fs_type(2))}))
        .unwrap();
    assert_eq!(r["faces"], json!(["regular"]));
    assert_eq!(r["skipped"], json!([{"face": "italic", "reason": "restricted"}]));
    assert!(s.execute("font.embed", &json!({"family": " ", "regular": base64_encode(&with_fs_type(0))})).is_err());
    assert!(s.execute("font.embed", &json!({"family": FAMILY, "regular": "%%%"})).is_err());
    assert!(s.execute("font.embed", &json!({"family": FAMILY, "regular": base64_encode(b"not a font at all")})).is_err());
}
