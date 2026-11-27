//! Font parts for PresentationML `p:embeddedFontLst`: PowerPoint stores embedded faces as
//! Embedded OpenType (W3C EOT submission, version 0x00020001, uncompressed, not XOR-encrypted)
//! in `application/x-fontdata` parts ([MS-OI29500] 2.1.32), and LibreOffice writes the same.

/// What a font's OS/2 `fsType` lets a document do with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Embedding {
    /// No restriction (bit 1-3 clear).
    Installable,
    /// Editable embedding (bit 3).
    Editable,
    /// Preview and print only (bit 2): the document may show it, never edit with it.
    PreviewPrint,
    /// Restricted license (bit 1 alone): no embedding.
    Restricted,
}

impl Embedding {
    /// Embedding in a presentation someone edits: installable or editable faces only.
    pub fn allows_editing(self) -> bool {
        matches!(self, Embedding::Installable | Embedding::Editable)
    }
}

/// Why a face can't go into a presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbedError {
    /// Not one TrueType or OpenType font (collections, WOFF and the like included).
    NotAFont,
    /// The font's license forbids it (restricted, preview-and-print only, or bitmap only).
    NotAllowed(Embedding),
    /// A bitmap-only font (fsType bit 9): no outlines to embed.
    BitmapOnly,
}

struct Tables<'a> {
    data: &'a [u8],
    dir: Vec<([u8; 4], usize, usize)>,
}

impl<'a> Tables<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let tag = data.get(0..4)?;
        if tag != [0, 1, 0, 0] && tag != b"OTTO" && tag != b"true" {
            return None;
        }
        let n = u16::from_be_bytes(data.get(4..6)?.try_into().ok()?) as usize;
        let mut dir = Vec::with_capacity(n);
        for i in 0..n {
            let r = data.get(12 + 16 * i..28 + 16 * i)?;
            let off = u32::from_be_bytes(r[8..12].try_into().ok()?) as usize;
            let len = u32::from_be_bytes(r[12..16].try_into().ok()?) as usize;
            data.get(off..off.checked_add(len)?)?;
            dir.push((r[0..4].try_into().ok()?, off, len));
        }
        Some(Tables { data, dir })
    }

    fn get(&self, tag: &[u8; 4]) -> Option<&'a [u8]> {
        let (_, off, len) = self.dir.iter().find(|(t, ..)| t == tag)?;
        self.data.get(*off..*off + *len)
    }
}

fn be16(b: &[u8], at: usize) -> u16 {
    b.get(at..at + 2).map_or(0, |x| u16::from_be_bytes([x[0], x[1]]))
}

fn be32(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).map_or(0, |x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]))
}

/// The embedding permission `fsType` gives; the least restrictive bit wins, as the OpenType spec says.
pub fn embedding_of(fs_type: u16) -> Embedding {
    if fs_type & 0x0008 != 0 {
        Embedding::Editable
    } else if fs_type & 0x0004 != 0 {
        Embedding::PreviewPrint
    } else if fs_type & 0x0002 != 0 {
        Embedding::Restricted
    } else {
        Embedding::Installable
    }
}

/// The `fsType` of a TrueType or OpenType font (0 without an OS/2 table); none when it isn't one.
pub fn font_fs_type(sfnt: &[u8]) -> Option<u16> {
    let t = Tables::parse(sfnt)?;
    Some(t.get(b"OS/2").map_or(0, |os2| be16(os2, 8)))
}

/// Face `index` of a TrueType collection as a font of its own (tables copied, offsets rebased);
/// a single font comes back unchanged.
pub fn single_font(data: &[u8], index: u32) -> Option<Vec<u8>> {
    if !data.starts_with(b"ttcf") {
        return Tables::parse(data).map(|_| data.to_vec());
    }
    let n = be32(data, 8);
    if index >= n || n > 4096 {
        return None;
    }
    let at = be32(data, 12 + 4 * index as usize) as usize;
    let font = data.get(at..)?;
    if !matches!(font.get(0..4)?, [0, 1, 0, 0] | b"OTTO" | b"true") {
        return None;
    }
    let count = be16(font, 4) as usize;
    let mut out = font.get(..12 + 16 * count)?.to_vec();
    for i in 0..count {
        let r = 12 + 16 * i;
        let off = be32(font, r + 8) as usize;
        let len = be32(font, r + 12) as usize;
        let table = data.get(off..off.checked_add(len)?)?;
        let new = u32::try_from(out.len()).ok()?;
        out[r + 8..r + 12].copy_from_slice(&new.to_be_bytes());
        out.extend_from_slice(table);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    Some(out)
}

/// A Windows-platform name (UTF-16LE), US English first.
fn name(t: &Tables, id: u16) -> Vec<u8> {
    let Some(nm) = t.get(b"name") else { return Vec::new() };
    let count = be16(nm, 2) as usize;
    let strings = be16(nm, 4) as usize;
    let mut best: Option<(bool, usize, usize)> = None;
    for i in 0..count {
        let r = 6 + 12 * i;
        let (pid, eid, lid, nid) = (be16(nm, r), be16(nm, r + 2), be16(nm, r + 4), be16(nm, r + 6));
        if pid != 3 || !matches!(eid, 0 | 1 | 10) || nid != id {
            continue;
        }
        let us = lid == 0x0409;
        if best.is_none_or(|b| us && !b.0) {
            best = Some((us, be16(nm, r + 8) as usize, be16(nm, r + 10) as usize));
        }
    }
    let Some((_, len, off)) = best else { return Vec::new() };
    let Some(raw) = nm.get(strings + off..strings + off + len) else { return Vec::new() };
    raw.chunks_exact(2).flat_map(|c| [c[1], c[0]]).take(0xFFFE).collect()
}

/// Wrap a TrueType or OpenType font as an EOT part, refusing faces whose license forbids
/// editable embedding.
pub fn eot_wrap(sfnt: &[u8]) -> Result<Vec<u8>, EmbedError> {
    let t = Tables::parse(sfnt).ok_or(EmbedError::NotAFont)?;
    let head = t.get(b"head").ok_or(EmbedError::NotAFont)?;
    let os2 = t.get(b"OS/2").unwrap_or(&[]);
    let fs_type = be16(os2, 8);
    if fs_type & 0x0200 != 0 {
        return Err(EmbedError::BitmapOnly);
    }
    let e = embedding_of(fs_type);
    if !e.allows_editing() {
        return Err(EmbedError::NotAllowed(e));
    }
    let mut h = Vec::with_capacity(256);
    h.extend_from_slice(&0x0002_0001u32.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes()); // flags: not subset, not compressed, not encrypted
    let mut panose = [0u8; 10];
    if let Some(p) = os2.get(32..42) {
        panose.copy_from_slice(p);
    }
    h.extend_from_slice(&panose);
    h.push(1); // DEFAULT_CHARSET
    let italic = be16(os2, 62) & 1 != 0 || be16(head, 44) & 2 != 0;
    h.push(u8::from(italic));
    let weight = if os2.len() >= 6 {
        be16(os2, 4)
    } else if be16(head, 44) & 1 != 0 {
        700
    } else {
        400
    };
    h.extend_from_slice(&u32::from(weight).to_le_bytes());
    h.extend_from_slice(&fs_type.to_le_bytes());
    h.extend_from_slice(&0x504Cu16.to_le_bytes());
    for i in 0..4 {
        h.extend_from_slice(&be32(os2, 42 + 4 * i).to_le_bytes());
    }
    let has_cp = be16(os2, 0) >= 1 && os2.len() >= 86;
    for i in 0..2 {
        h.extend_from_slice(&(if has_cp { be32(os2, 78 + 4 * i) } else { 0 }).to_le_bytes());
    }
    h.extend_from_slice(&be32(head, 8).to_le_bytes());
    h.extend_from_slice(&[0u8; 16]);
    for id in [1u16, 2, 5, 4] {
        let n = name(&t, id);
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(&(n.len() as u16).to_le_bytes());
        h.extend_from_slice(&n);
    }
    h.extend_from_slice(&0u16.to_le_bytes()); // Padding5
    h.extend_from_slice(&0u16.to_le_bytes()); // no RootString
    let total = 8 + h.len() + sfnt.len();
    let total = u32::try_from(total).map_err(|_| EmbedError::NotAFont)?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&(sfnt.len() as u32).to_le_bytes());
    out.extend_from_slice(&h);
    out.extend_from_slice(sfnt);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font with only a head, an OS/2 and a name table: enough for the header.
    fn font(fs_type: u16, family: &str) -> Vec<u8> {
        let mut head = vec![0u8; 54];
        head[8..12].copy_from_slice(&0x1234_5678u32.to_be_bytes());
        let mut os2 = vec![0u8; 96];
        os2[0..2].copy_from_slice(&4u16.to_be_bytes());
        os2[4..6].copy_from_slice(&700u16.to_be_bytes());
        os2[8..10].copy_from_slice(&fs_type.to_be_bytes());
        os2[62..64].copy_from_slice(&1u16.to_be_bytes());
        let fam: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut name = Vec::new();
        name.extend_from_slice(&0u16.to_be_bytes());
        name.extend_from_slice(&1u16.to_be_bytes());
        name.extend_from_slice(&18u16.to_be_bytes());
        for v in [3u16, 1, 0x0409, 1, fam.len() as u16, 0] {
            name.extend_from_slice(&v.to_be_bytes());
        }
        name.extend_from_slice(&fam);
        let tables: [(&[u8; 4], Vec<u8>); 3] = [(b"OS/2", os2), (b"head", head), (b"name", name)];
        let mut out = vec![0, 1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0];
        let mut off = 12 + 16 * tables.len();
        let mut body = Vec::new();
        for (tag, data) in &tables {
            out.extend_from_slice(*tag);
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&(off as u32).to_be_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            body.extend_from_slice(data);
            while body.len() % 4 != 0 {
                body.push(0);
            }
            off = 12 + 16 * tables.len() + body.len();
        }
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn wraps_a_font_the_reader_unwraps_again() {
        let f = font(0, "Fam");
        let eot = eot_wrap(&f).unwrap();
        assert_eq!(u32::from_le_bytes(eot[0..4].try_into().unwrap()) as usize, eot.len());
        assert_eq!(u32::from_le_bytes(eot[8..12].try_into().unwrap()), 0x0002_0001);
        assert_eq!(u16::from_le_bytes([eot[34], eot[35]]), 0x504C);
        assert_eq!(eot[27], 1, "italic from fsSelection");
        assert_eq!(u32::from_le_bytes(eot[28..32].try_into().unwrap()), 700);
        // FamilyName follows the fixed part.
        assert_eq!(u16::from_le_bytes([eot[82], eot[83]]), 6);
        assert_eq!(&eot[84..90], &[b'F', 0, b'a', 0, b'm', 0]);
        assert_eq!(crate::embedded_sfnt(&eot), Some(&f[..]));
    }

    #[test]
    fn refuses_what_the_license_forbids() {
        assert_eq!(eot_wrap(&font(0x0002, "R")), Err(EmbedError::NotAllowed(Embedding::Restricted)));
        assert_eq!(eot_wrap(&font(0x0004, "P")), Err(EmbedError::NotAllowed(Embedding::PreviewPrint)));
        assert_eq!(eot_wrap(&font(0x0200, "B")), Err(EmbedError::BitmapOnly));
        assert!(eot_wrap(&font(0x0008, "E")).is_ok());
        assert!(eot_wrap(&font(0x000A, "E")).is_ok(), "editable wins over restricted");
        assert_eq!(eot_wrap(b"wOFF0000000000000000"), Err(EmbedError::NotAFont));
        assert_eq!(font_fs_type(&font(0x0004, "P")), Some(4));
    }

    #[test]
    fn takes_one_face_out_of_a_collection() {
        let (a, b) = (font(0, "A"), font(8, "B"));
        let mut ttc = b"ttcf".to_vec();
        ttc.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        ttc.extend_from_slice(&2u32.to_be_bytes());
        let base = 20;
        ttc.extend_from_slice(&(base as u32).to_be_bytes());
        ttc.extend_from_slice(&((base + a.len()) as u32).to_be_bytes());
        // Rebase each font's table offsets onto the collection.
        for (f, start) in [(&a, base), (&b, base + a.len())] {
            let mut f = f.to_vec();
            for i in 0..3 {
                let r = 12 + 16 * i + 8;
                let off = u32::from_be_bytes(f[r..r + 4].try_into().unwrap()) + start as u32;
                f[r..r + 4].copy_from_slice(&off.to_be_bytes());
            }
            ttc.extend_from_slice(&f);
        }
        assert_eq!(single_font(&ttc, 0).unwrap(), a);
        assert_eq!(single_font(&ttc, 1).unwrap(), b);
        assert!(single_font(&ttc, 2).is_none());
        assert_eq!(single_font(&a, 0).unwrap(), a);
    }
}
