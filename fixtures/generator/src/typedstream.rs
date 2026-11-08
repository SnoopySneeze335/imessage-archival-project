//! A minimal `attributedBody` writer.
//!
//! iOS stores message text as an `NSMutableAttributedString` serialized with
//! NeXTSTEP `typedstream`, in `message.attributedBody`. Newer iOS versions
//! often leave `message.text` empty and keep the text only there, so the
//! fixture needs real `attributedBody` blobs to exercise that path.
//!
//! This writes the simplest shape iOS produces: one run of plain text with a
//! single `__kIMMessagePartAttributeName = 0` attribute. Layout:
//!
//! ```text
//! HEADER           "streamtyped" header and the class chain
//!                  NSMutableAttributedString > NSAttributedString > NSObject,
//!                  then NSMutableString > NSString, then '+' (C string)
//! <len> <utf-8>    the text: byte length, then bytes
//! RANGE            end of string object, then "iI" with the range start (1)
//! <len16>          the run length in UTF-16 code units
//! ATTRIBUTES       NSDictionary { __kIMMessagePartAttributeName: NSNumber(0) }
//! ```
//!
//! Integers use typedstream's encoding: one byte below 0x80, otherwise 0x81
//! and an i16, or 0x82 and an i32, little-endian.

const HEADER: &[u8] = &[
    0x04, 0x0b, b's', b't', b'r', b'e', b'a', b'm', b't', b'y', b'p', b'e', b'd', 0x81, 0xe8, 0x03,
    0x84, 0x01, 0x40, 0x84, 0x84, 0x84, 0x19, b'N', b'S', b'M', b'u', b't', b'a', b'b', b'l', b'e',
    b'A', b't', b't', b'r', b'i', b'b', b'u', b't', b'e', b'd', b'S', b't', b'r', b'i', b'n', b'g',
    0x00, 0x84, 0x84, 0x12, b'N', b'S', b'A', b't', b't', b'r', b'i', b'b', b'u', b't', b'e', b'd',
    b'S', b't', b'r', b'i', b'n', b'g', 0x00, 0x84, 0x84, 0x08, b'N', b'S', b'O', b'b', b'j', b'e',
    b'c', b't', 0x00, 0x85, 0x92, 0x84, 0x84, 0x84, 0x0f, b'N', b'S', b'M', b'u', b't', b'a', b'b',
    b'l', b'e', b'S', b't', b'r', b'i', b'n', b'g', 0x01, 0x84, 0x84, 0x08, b'N', b'S', b'S', b't',
    b'r', b'i', b'n', b'g', 0x01, 0x95, 0x84, 0x01, b'+',
];

const RANGE: &[u8] = &[0x86, 0x84, 0x02, b'i', b'I', 0x01];

const ATTRIBUTES: &[u8] = &[
    0x92, 0x84, 0x84, 0x84, 0x0c, b'N', b'S', b'D', b'i', b'c', b't', b'i', b'o', b'n', b'a', b'r',
    b'y', 0x00, 0x95, 0x84, 0x01, b'i', 0x01, 0x92, 0x84, 0x98, 0x98, 0x1d, b'_', b'_', b'k', b'I',
    b'M', b'M', b'e', b's', b's', b'a', b'g', b'e', b'P', b'a', b'r', b't', b'A', b't', b't', b'r',
    b'i', b'b', b'u', b't', b'e', b'N', b'a', b'm', b'e', 0x86, 0x92, 0x84, 0x84, 0x84, 0x08, b'N',
    b'S', b'N', b'u', b'm', b'b', b'e', b'r', 0x00, 0x84, 0x84, 0x07, b'N', b'S', b'V', b'a', b'l',
    b'u', b'e', 0x00, 0x95, 0x84, 0x01, b'*', 0x84, 0x9b, 0x9b, 0x00, 0x86, 0x86, 0x86,
];

fn int(out: &mut Vec<u8>, n: usize) {
    if n < 0x80 {
        out.push(n as u8);
    } else if let Ok(n) = i16::try_from(n) {
        out.push(0x81);
        out.extend_from_slice(&n.to_le_bytes());
    } else {
        out.push(0x82);
        out.extend_from_slice(&(n as i32).to_le_bytes());
    }
}

/// `attributedBody` bytes for a plain-text message.
pub fn attributed_body(text: &str) -> Vec<u8> {
    let mut out = HEADER.to_vec();
    int(&mut out, text.len());
    out.extend_from_slice(text.as_bytes());
    out.extend_from_slice(RANGE);
    int(&mut out, text.encode_utf16().count());
    out.extend_from_slice(ATTRIBUTES);
    out
}
