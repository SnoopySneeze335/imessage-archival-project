//! Tiny, valid media files built from integers only, so output is identical
//! on every platform. No image or audio libraries are needed.

/// A `size`×`size` RGB PNG: a two-colour checkerboard.
pub fn png(size: u32, a: [u8; 3], b: [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..size {
        raw.push(0); // filter: none
        for x in 0..size {
            let c = if (x / 4 + y / 4) % 2 == 0 { a } else { b };
            raw.extend_from_slice(&c);
        }
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A short mono 16-bit PCM Core Audio File (.caf), like a voice message.
pub fn caf_voice(sample_rate: u32, samples: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"caff");
    out.extend_from_slice(&1u16.to_be_bytes()); // version
    out.extend_from_slice(&0u16.to_be_bytes()); // flags

    let mut desc = Vec::new();
    desc.extend_from_slice(&f64::from(sample_rate).to_bits().to_be_bytes());
    desc.extend_from_slice(b"lpcm");
    desc.extend_from_slice(&2u32.to_be_bytes()); // flags: little-endian
    desc.extend_from_slice(&2u32.to_be_bytes()); // bytes per packet
    desc.extend_from_slice(&1u32.to_be_bytes()); // frames per packet
    desc.extend_from_slice(&1u32.to_be_bytes()); // channels
    desc.extend_from_slice(&16u32.to_be_bytes()); // bits per channel
    caf_chunk(&mut out, b"desc", &desc);

    let mut data = 0u32.to_be_bytes().to_vec(); // edit count
    for i in 0..samples {
        // Triangle wave around 440 Hz at 8 kHz, integer-only.
        let period = 18;
        let phase = (i % period) as i32;
        let tri = if phase < period as i32 / 2 {
            phase
        } else {
            period as i32 - phase
        };
        let s = ((tri - 4) * 3000) as i16;
        data.extend_from_slice(&s.to_le_bytes());
    }
    caf_chunk(&mut out, b"data", &data);
    out
}

/// The first bytes of a JPEG (start of image and a JFIF header) with the
/// image data cut off, as a damaged or partly copied file looks.
pub fn truncated_jpeg() -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    out.extend_from_slice(b"JFIF\0");
    out.extend_from_slice(&[1, 1, 0, 0, 1, 0, 1, 0, 0]);
    out.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43, 0x00]);
    out.extend(std::iter::repeat_n(0x10, 24));
    out
}

fn caf_chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(body.len() as i64).to_be_bytes());
    out.extend_from_slice(body);
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let mut crc_input = kind.to_vec();
    crc_input.extend_from_slice(body);
    out.extend_from_slice(&crc_input);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// zlib stream with uncompressed ("stored") deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = data.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        out.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// A HEIF image turned into what ImageMagick 6 sees in an iOS image sequence
/// (`.heics`): the same bytes with the major brand `msf1`. libheif still
/// reads it; ImageMagick only does when told the format.
pub fn heif_sequence_brand(heic: &[u8]) -> Vec<u8> {
    assert_eq!(&heic[4..8], b"ftyp", "not a HEIF file");
    let mut out = heic.to_vec();
    out[8..12].copy_from_slice(b"msf1");
    out
}

/// An MP4 with a version-1 `chnl` box (mono layout) appended to its first
/// audio sample entry, which ffmpeg 6.1 refuses to open. The `moov` box must
/// come after `mdat`, so growing it moves no sample data.
pub fn with_chnl_v1(mp4: &[u8]) -> Vec<u8> {
    let chnl: &[u8] = &[0, 0, 0, 15, b'c', b'h', b'n', b'l', 1, 0, 0, 0, 0x10, 1, 0];
    // Offsets of the boxes from moov down to the sample entry, then its end.
    let mut ancestors = Vec::new();
    let moov = child(mp4, 0, mp4.len(), b"moov").expect("moov");
    ancestors.push(moov);
    let (mut entry, mut found) = (0, false);
    for trak in children(mp4, moov + 8, end(mp4, moov), b"trak") {
        let mdia = child(mp4, trak + 8, end(mp4, trak), b"mdia").expect("mdia");
        let hdlr = child(mp4, mdia + 8, end(mp4, mdia), b"hdlr").expect("hdlr");
        if &mp4[hdlr + 16..hdlr + 20] != b"soun" {
            continue;
        }
        let minf = child(mp4, mdia + 8, end(mp4, mdia), b"minf").expect("minf");
        let stbl = child(mp4, minf + 8, end(mp4, minf), b"stbl").expect("stbl");
        let stsd = child(mp4, stbl + 8, end(mp4, stbl), b"stsd").expect("stsd");
        entry = stsd + 16;
        ancestors.extend([trak, mdia, minf, stbl, stsd, entry]);
        found = true;
        break;
    }
    assert!(found, "no audio track");
    let mdat = child(mp4, 0, mp4.len(), b"mdat").expect("mdat");
    assert!(mdat < moov, "moov must follow mdat");
    let at = end(mp4, entry);
    let mut out = mp4[..at].to_vec();
    out.extend_from_slice(chnl);
    out.extend_from_slice(&mp4[at..]);
    for a in ancestors {
        let size = u32::from_be_bytes(out[a..a + 4].try_into().unwrap()) + chnl.len() as u32;
        out[a..a + 4].copy_from_slice(&size.to_be_bytes());
    }
    out
}

fn end(buf: &[u8], at: usize) -> usize {
    at + u32::from_be_bytes(buf[at..at + 4].try_into().unwrap()) as usize
}

fn children(buf: &[u8], mut at: usize, stop: usize, kind: &[u8; 4]) -> Vec<usize> {
    let mut out = Vec::new();
    while at + 8 <= stop {
        if &buf[at + 4..at + 8] == kind {
            out.push(at);
        }
        at = end(buf, at);
    }
    out
}

fn child(buf: &[u8], at: usize, stop: usize, kind: &[u8; 4]) -> Option<usize> {
    children(buf, at, stop, kind).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_reference() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn adler32_matches_reference() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
