//! Getting a stored file ready for ImageMagick or ffmpeg.
//!
//! Every tool call on a blob goes through [`Input::prepare`]: thumbnails now,
//! and the background queue's previews and conversions (Milestone 3b). A
//! workaround for files the tools misread therefore applies to all of them at
//! once. Adding one means bumping [`DECODER`], so derivatives that failed
//! under the old handling are tried again by the next run.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// Version of the input handling in this module. A derivative records the
/// version that last tried it, and failed ones from an older version are
/// retried.
///
/// 1. Files passed to the tools as they are (Milestone 3a).
/// 2. HEIF files are named as HEIC to ImageMagick, which does not recognise
///    image sequences (`msf1`) by their bytes. MP4 and QuickTime files with a
///    `chnl` box that ffmpeg 6.1 rejects are read from a patched copy.
pub const DECODER: i64 = 2;

/// Largest `moov` box read when looking for boxes to patch. Real ones are a
/// few MB even for long videos; anything bigger is passed through as is.
const MAX_MOOV: u64 = 64 << 20;

/// ISO base media file brands that libheif reads as HEIF/HEIC. AVIF is not
/// listed: the image has no AV1 decoder.
const HEIF_BRANDS: &[&[u8; 4]] = &[
    b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"hevm", b"hevs", b"mif1", b"mif2",
    b"msf1", b"miaf",
];

/// What a file is, from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// HEIF image or image sequence (HEIC, HEICS).
    Heif,
    /// Any other ISO base media file: MP4, QuickTime, M4A.
    IsoMedia,
    Other,
}

/// A file ready to be handed to a tool. When it is a patched copy, the copy
/// is deleted when this is dropped.
#[derive(Debug)]
pub struct Input {
    path: PathBuf,
    container: Container,
    patched: bool,
}

impl Input {
    /// Looks at `src` and, if a tool needs it, writes a patched copy into
    /// `work_dir` (a temporary folder the run cleans up). `src` itself is
    /// never written.
    pub fn prepare(src: &Path, work_dir: &Path) -> Result<Self, String> {
        let mut file = File::open(src).map_err(|e| format!("opening the file: {e}"))?;
        let container = sniff(&mut file).map_err(|e| format!("reading the file: {e}"))?;
        let mut input = Self {
            path: src.to_owned(),
            container,
            patched: false,
        };
        if container == Container::IsoMedia {
            let offsets =
                unsupported_chnl(&mut file).map_err(|e| format!("reading the file: {e}"))?;
            if !offsets.is_empty() {
                let copy = crate::blobs::tmp_path(work_dir, ".mp4");
                patch_copy(src, &copy, &offsets).map_err(|e| {
                    let _ = fs::remove_file(&copy);
                    format!("patching a copy: {e}")
                })?;
                input.path = copy;
                input.patched = true;
            }
        }
        Ok(input)
    }

    /// The file to read: the original, or its patched copy.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn container(&self) -> Container {
        self.container
    }

    /// Whether the tools read a patched copy rather than the original.
    pub fn is_patched(&self) -> bool {
        self.patched
    }

    /// ImageMagick's name for the first frame, with the format given
    /// explicitly when ImageMagick would not recognise it.
    pub fn magick_first_frame(&self) -> String {
        let prefix = match self.container {
            Container::Heif => "heic:",
            Container::IsoMedia | Container::Other => "",
        };
        format!("{prefix}{}[0]", self.path.display())
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        if self.patched {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Reads the `ftyp` box at the start of a file, if there is one.
fn sniff(file: &mut File) -> io::Result<Container> {
    let mut head = Vec::with_capacity(256);
    Read::by_ref(file).take(256).read_to_end(&mut head)?;
    file.seek(SeekFrom::Start(0))?;
    if head.len() < 12 || &head[4..8] != b"ftyp" {
        return Ok(Container::Other);
    }
    let major = &head[8..12];
    if HEIF_BRANDS.iter().any(|b| &b[..] == major) {
        Ok(Container::Heif)
    } else {
        Ok(Container::IsoMedia)
    }
}

/// One box inside a buffer: where it starts, its header length and size.
struct BoxAt {
    start: usize,
    header: usize,
    size: usize,
    kind: [u8; 4],
}

impl BoxAt {
    fn body<'a>(&self, buf: &'a [u8]) -> &'a [u8] {
        &buf[self.start + self.header..self.start + self.size]
    }
}

/// Splits `buf` into boxes, or `None` unless they fill it exactly. The exact
/// fit is what tells real child boxes apart from other data.
fn boxes(buf: &[u8]) -> Option<Vec<BoxAt>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        let head = buf.get(i..i + 8)?;
        let kind: [u8; 4] = head[4..8].try_into().ok()?;
        let (size, header) = match u32::from_be_bytes(head[..4].try_into().ok()?) {
            0 => (buf.len() - i, 8),
            1 => {
                let large = buf.get(i + 8..i + 16)?;
                let size = usize::try_from(u64::from_be_bytes(large.try_into().ok()?)).ok()?;
                (size, 16)
            }
            n => (n as usize, 8),
        };
        if size < header || i.checked_add(size)? > buf.len() {
            return None;
        }
        out.push(BoxAt {
            start: i,
            header,
            size,
            kind,
        });
        i += size;
    }
    Some(out)
}

/// File offsets of the type field of every `chnl` box in an audio sample
/// entry that ffmpeg 6.1 refuses: any version or flags other than 0. Such a
/// box (channel layout, ISO/IEC 23001-8) stops ffmpeg from opening the file
/// at all, even to read the video.
fn unsupported_chnl(file: &mut File) -> io::Result<Vec<u64>> {
    let len = file.metadata()?.len();
    let mut pos = 0u64;
    while pos + 8 <= len {
        file.seek(SeekFrom::Start(pos))?;
        let mut head = [0u8; 16];
        file.read_exact(&mut head[..8])?;
        let (size, header) = match u32::from_be_bytes(head[..4].try_into().expect("4 bytes")) {
            0 => (len - pos, 8),
            1 => {
                file.read_exact(&mut head[8..])?;
                (
                    u64::from_be_bytes(head[8..].try_into().expect("8 bytes")),
                    16,
                )
            }
            n => (u64::from(n), 8),
        };
        if size < header || pos + size > len {
            break;
        }
        if &head[4..8] == b"moov" {
            if size > MAX_MOOV {
                break;
            }
            let mut moov = vec![0u8; (size - header) as usize];
            file.read_exact(&mut moov)?;
            let mut found = Vec::new();
            walk(&moov, pos + header, false, &mut found);
            file.seek(SeekFrom::Start(0))?;
            return Ok(found);
        }
        pos += size;
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(Vec::new())
}

/// Walks the boxes of `buf`, which starts at file offset `base`, down to the
/// sample descriptions of sound tracks.
fn walk(buf: &[u8], base: u64, sound: bool, found: &mut Vec<u64>) {
    let Some(kids) = boxes(buf) else { return };
    for b in &kids {
        let body = b.body(buf);
        let at = base + (b.start + b.header) as u64;
        match &b.kind {
            b"trak" | b"minf" | b"stbl" => walk(body, at, sound, found),
            b"mdia" => {
                let is_sound = boxes(body).is_some_and(|kids| {
                    kids.iter().any(|k| {
                        &k.kind == b"hdlr" && k.body(body).get(8..12) == Some(&b"soun"[..])
                    })
                });
                walk(body, at, is_sound, found);
            }
            // Full box header (4 bytes) and entry count (4), then entries.
            b"stsd" if sound && body.len() >= 8 => {
                let Some(entries) = boxes(&body[8..]) else {
                    continue;
                };
                for e in &entries {
                    sound_entry(
                        e.body(&body[8..]),
                        at + 8 + (e.start + e.header) as u64,
                        found,
                    );
                }
            }
            _ => {}
        }
    }
}

/// Finds the child boxes of an audio sample entry. Their offset depends on
/// the entry's version: 28 bytes of fields for version 0 (and ISO version 1),
/// 44 for QuickTime version 1, 64 for version 2.
fn sound_entry(entry: &[u8], base: u64, found: &mut Vec<u64>) {
    let version = entry
        .get(8..10)
        .map_or(0, |v| u16::from_be_bytes([v[0], v[1]]));
    let candidates: &[usize] = match version {
        0 => &[28],
        1 => &[44, 28],
        2 => &[64],
        _ => &[],
    };
    for &skip in candidates {
        if let Some(rest) = entry.get(skip..)
            && boxes(rest).is_some()
        {
            audio_children(rest, base + skip as u64, found);
            return;
        }
    }
}

fn audio_children(buf: &[u8], base: u64, found: &mut Vec<u64>) {
    let Some(kids) = boxes(buf) else { return };
    for b in &kids {
        let body = b.body(buf);
        match &b.kind {
            b"chnl" if body.len() >= 4 && body[..4] != [0, 0, 0, 0] => {
                found.push(base + b.start as u64 + 4);
            }
            // QuickTime keeps codec boxes inside `wave`.
            b"wave" => audio_children(body, base + (b.start + b.header) as u64, found),
            _ => {}
        }
    }
}

/// Copies `src` to `dest` and renames the boxes at `offsets` to `free`, which
/// every reader skips. The box sizes stay the same, so nothing else moves.
fn patch_copy(src: &Path, dest: &Path, offsets: &[u64]) -> io::Result<()> {
    fs::copy(src, dest)?;
    let mut f = OpenOptions::new().write(true).open(dest)?;
    for &at in offsets {
        f.seek(SeekFrom::Start(at))?;
        f.write_all(b"free")?;
    }
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }

    /// An audio sample entry of `version` with the given child boxes.
    fn mp4a(version: u16, kids: &[Vec<u8>]) -> Vec<u8> {
        let mut body = vec![0u8; 6];
        body.extend_from_slice(&1u16.to_be_bytes()); // data reference index
        body.extend_from_slice(&version.to_be_bytes());
        body.extend_from_slice(&[0u8; 6]);
        body.extend_from_slice(&1u16.to_be_bytes()); // channels
        body.extend_from_slice(&16u16.to_be_bytes()); // sample size
        body.extend_from_slice(&[0u8; 4]);
        body.extend_from_slice(&(8000u32 << 16).to_be_bytes());
        if version == 1 {
            body.extend_from_slice(&[0, 0, 4, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 2]);
        }
        bx(b"mp4a", &cat(&[body, kids.concat()]))
    }

    fn track(handler: &[u8; 4], entry: Vec<u8>) -> Vec<u8> {
        let hdlr = bx(b"hdlr", &cat(&[vec![0; 8], handler.to_vec(), vec![0; 13]]));
        let stsd = bx(b"stsd", &cat(&[vec![0, 0, 0, 0, 0, 0, 0, 1], entry]));
        let minf = bx(b"minf", &bx(b"stbl", &stsd));
        bx(b"trak", &bx(b"mdia", &cat(&[hdlr, minf])))
    }

    fn file(tracks: &[Vec<u8>]) -> Vec<u8> {
        cat(&[
            bx(b"ftyp", b"mp42\0\0\0\0isommp42"),
            bx(b"mdat", &[7u8; 64]),
            bx(b"moov", &tracks.concat()),
        ])
    }

    fn chnl(version: u8) -> Vec<u8> {
        bx(b"chnl", &[version, 0, 0, 0, 0x10, 1, 0])
    }

    fn scan(bytes: &[u8]) -> (tempfile::TempDir, PathBuf, Vec<u64>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob");
        fs::write(&path, bytes).unwrap();
        let found = unsupported_chnl(&mut File::open(&path).unwrap()).unwrap();
        (dir, path, found)
    }

    #[test]
    fn finds_a_version_1_chnl_in_a_sound_track_only() {
        let esds = bx(b"esds", &[0u8; 12]);
        let bytes = file(&[
            track(b"vide", bx(b"avc1", &[0u8; 78])),
            track(b"soun", mp4a(0, &[esds.clone(), chnl(1)])),
        ]);
        let (_dir, _, found) = scan(&bytes);
        assert_eq!(found.len(), 1);
        let at = found[0] as usize;
        assert_eq!(&bytes[at..at + 4], b"chnl");

        // Version 0 is what ffmpeg 6.1 reads; leave it alone.
        let (_dir, _, found) = scan(&file(&[track(b"soun", mp4a(0, &[esds, chnl(0)]))]));
        assert!(found.is_empty());

        // QuickTime version 1 entries have 16 more bytes of fields, and may
        // keep the box inside `wave`.
        let bytes = file(&[track(b"soun", mp4a(1, &[bx(b"wave", &chnl(1))]))]);
        let (_dir, _, found) = scan(&bytes);
        assert_eq!(found.len(), 1);
        assert_eq!(&bytes[found[0] as usize..found[0] as usize + 4], b"chnl");

        // The same bytes outside a sound track are not touched.
        let (_dir, _, found) = scan(&file(&[track(b"vide", mp4a(0, &[chnl(1)]))]));
        assert!(found.is_empty());
    }

    #[test]
    fn patches_a_copy_and_never_the_original() {
        let bytes = file(&[track(b"soun", mp4a(0, &[chnl(1)]))]);
        let (dir, path, found) = scan(&bytes);
        let work = dir.path().join("work");
        fs::create_dir(&work).unwrap();
        let input = Input::prepare(&path, &work).unwrap();
        assert!(input.is_patched());
        assert_eq!(input.container(), Container::IsoMedia);
        let copy = fs::read(input.path()).unwrap();
        assert_eq!(copy.len(), bytes.len());
        let at = found[0] as usize;
        assert_eq!(&copy[at..at + 4], b"free");
        assert_eq!(copy[..at], bytes[..at]);
        assert_eq!(copy[at + 4..], bytes[at + 4..]);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let copy_path = input.path().to_owned();
        drop(input);
        assert!(!copy_path.exists(), "the copy is removed when done");
    }

    #[test]
    fn heif_brands_are_named_for_imagemagick() {
        let dir = tempfile::tempdir().unwrap();
        let check = |name: &str, bytes: Vec<u8>| {
            let path = dir.path().join(name);
            fs::write(&path, bytes).unwrap();
            let input = Input::prepare(&path, dir.path()).unwrap();
            assert!(!input.is_patched());
            (input.container(), input.magick_first_frame())
        };
        let (c, arg) = check("seq", bx(b"ftyp", b"msf1\0\0\0\0msf1heichevc"));
        assert_eq!(c, Container::Heif);
        assert!(arg.starts_with("heic:") && arg.ends_with("seq[0]"), "{arg}");
        let (c, arg) = check("photo", bx(b"ftyp", b"heic\0\0\0\0mif1heic"));
        assert_eq!(c, Container::Heif);
        assert!(arg.starts_with("heic:"));
        let (c, arg) = check("movie", file(&[]));
        assert_eq!(c, Container::IsoMedia);
        assert!(!arg.contains(':'));
        let (c, _) = check("jpeg", vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(c, Container::Other);
    }
}
