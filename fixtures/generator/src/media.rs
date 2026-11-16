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

/// AAC-LC audio rewrapped from an ADTS stream into a Core Audio File, as
/// iOS stores some voice messages: a `desc` chunk, the codec configuration
/// as a `kuki` (magic cookie) ES descriptor, a `pakt` packet table, and the
/// raw packets. Browsers cannot read CAF, but the AAC inside plays as is.
pub fn caf_aac(adts: &[u8]) -> Vec<u8> {
    let mut packets: Vec<&[u8]> = Vec::new();
    let (mut at, mut config) = (0, None);
    while at + 7 <= adts.len() {
        let h = &adts[at..at + 7];
        assert!(h[0] == 0xFF && h[1] & 0xF0 == 0xF0, "not an ADTS frame");
        let header = if h[1] & 1 == 1 { 7 } else { 9 };
        let len = (usize::from(h[3] & 3) << 11) | (usize::from(h[4]) << 3) | usize::from(h[5] >> 5);
        let profile = (h[2] >> 6) + 1; // audio object type
        let rate_index = (h[2] >> 2) & 0xF;
        let channels = ((h[2] & 1) << 2) | (h[3] >> 6);
        config.get_or_insert((profile, rate_index, channels));
        packets.push(&adts[at + header..at + len]);
        at += len;
    }
    let (profile, rate_index, channels) = config.expect("no ADTS frames");
    const RATES: [u32; 13] = [
        96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
        8_000, 7_350,
    ];
    let rate = RATES[usize::from(rate_index)];

    let mut out = Vec::new();
    out.extend_from_slice(b"caff");
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());

    let mut desc = Vec::new();
    desc.extend_from_slice(&f64::from(rate).to_bits().to_be_bytes());
    desc.extend_from_slice(b"aac ");
    desc.extend_from_slice(&u32::from(profile).to_be_bytes()); // MPEG-4 object type
    desc.extend_from_slice(&0u32.to_be_bytes()); // bytes per packet: variable
    desc.extend_from_slice(&1024u32.to_be_bytes()); // frames per packet
    desc.extend_from_slice(&u32::from(channels).to_be_bytes());
    desc.extend_from_slice(&0u32.to_be_bytes()); // bits per channel
    caf_chunk(&mut out, b"desc", &desc);

    // ES_Descriptor > DecoderConfigDescriptor > DecoderSpecificInfo, the
    // AudioSpecificConfig being 5 bits object type, 4 rate index, 4 channels.
    let asc =
        (u16::from(profile) << 11) | (u16::from(rate_index) << 7) | (u16::from(channels) << 3);
    let dsi = [&[0x05, 2][..], &asc.to_be_bytes()].concat();
    let dcd = [
        &[0x04, 13 + dsi.len() as u8, 0x40, 0x15, 0, 0, 0][..],
        &[0, 0, 0, 0, 0, 0, 0, 0],
        &dsi,
    ]
    .concat();
    let es = [&[0x03, 3 + dcd.len() as u8, 0, 1, 0][..], &dcd].concat();
    // The cookie is the body of an `esds` box: version and flags, then the
    // descriptor.
    caf_chunk(&mut out, b"kuki", &[&[0, 0, 0, 0][..], &es].concat());

    let mut pakt = Vec::new();
    pakt.extend_from_slice(&(packets.len() as i64).to_be_bytes());
    pakt.extend_from_slice(&(packets.len() as i64 * 1024).to_be_bytes()); // valid frames
    pakt.extend_from_slice(&0i32.to_be_bytes()); // priming frames
    pakt.extend_from_slice(&0i32.to_be_bytes()); // remainder frames
    for p in &packets {
        // Packet sizes as big-endian base-128 varints.
        let mut n = p.len();
        let mut bytes = vec![(n & 0x7F) as u8];
        n >>= 7;
        while n > 0 {
            bytes.push(0x80 | (n & 0x7F) as u8);
            n >>= 7;
        }
        bytes.reverse();
        pakt.extend_from_slice(&bytes);
    }
    caf_chunk(&mut out, b"pakt", &pakt);

    let mut data = 0u32.to_be_bytes().to_vec(); // edit count
    for p in &packets {
        data.extend_from_slice(p);
    }
    caf_chunk(&mut out, b"data", &data);
    out
}

/// The first `keep` bytes of `file`, as a damaged or partly copied file
/// looks: the header is intact but the media data is cut off.
pub fn truncated(file: &[u8], keep: usize) -> Vec<u8> {
    file[..keep.min(file.len())].to_vec()
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
