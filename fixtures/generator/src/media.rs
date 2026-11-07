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
