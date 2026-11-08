//! Property-list blobs stored on messages.
//!
//! - `message_summary_info`: a binary plist iOS writes on edited and unsent
//!   messages. `otr` lists the body parts, `ec` maps a part index to its edit
//!   history (`d`: Apple-epoch seconds as a real, `t`: that version's
//!   `attributedBody` typedstream; item 0 is the original), `ep` lists edited
//!   parts and `rp` unsent parts.
//! - `payload_data` of a link preview: an `NSKeyedArchiver` archive whose root
//!   holds `richLinkMetadata` (an `LPLinkMetadata`).

use plist::{Dictionary, Uid, Value};

use crate::{scenario::apple_s, typedstream::attributed_body};

fn binary(value: Value) -> Vec<u8> {
    let mut buf = Vec::new();
    value
        .to_writer_binary(&mut buf)
        .expect("plist serializes to memory");
    buf
}

fn uid(n: u64) -> Value {
    Value::Uid(Uid::new(n))
}

/// `message_summary_info` for a one-part message. `history` is every version
/// of part 0 in order, as (Unix seconds, text), the original first; `unsent`
/// marks part 0 as unsent.
pub fn summary_info(history: &[(i64, &str)], unsent: bool) -> Vec<u8> {
    let len = history
        .last()
        .map_or(0, |(_, t)| t.encode_utf16().count() as u64);
    let mut part = Dictionary::new();
    part.insert("le".into(), len.into());
    part.insert("lo".into(), 0u64.into());
    let mut otr = Dictionary::new();
    otr.insert("0".into(), part.into());

    let mut root = Dictionary::new();
    root.insert("otr".into(), otr.into());
    if !history.is_empty() {
        let events: Vec<Value> = history
            .iter()
            .map(|&(at, text)| {
                let mut e = Dictionary::new();
                e.insert("d".into(), Value::Real(apple_s(at) as f64));
                e.insert("t".into(), Value::Data(attributed_body(text)));
                e.into()
            })
            .collect();
        let mut ec = Dictionary::new();
        ec.insert("0".into(), Value::Array(events));
        root.insert("ec".into(), ec.into());
        root.insert("ep".into(), Value::Array(vec![0u64.into()]));
    }
    if unsent {
        root.insert("rp".into(), Value::Array(vec![0u64.into()]));
    }
    binary(root.into())
}

/// Link-preview `payload_data` as the Messages app archives it.
pub fn link_preview(url: &str, title: &str, summary: &str, site_name: &str) -> Vec<u8> {
    let class = |name: &str, chain: &[&str]| {
        let mut c = Dictionary::new();
        c.insert(
            "$classes".into(),
            Value::Array(chain.iter().map(|&s| s.into()).collect()),
        );
        c.insert("$classname".into(), name.into());
        Value::from(c)
    };
    let mut payload = Dictionary::new();
    payload.insert("richLinkMetadata".into(), uid(2));
    payload.insert("$class".into(), uid(9));
    let mut meta = Dictionary::new();
    meta.insert("title".into(), uid(3));
    meta.insert("summary".into(), uid(4));
    meta.insert("URL".into(), uid(5));
    meta.insert("originalURL".into(), uid(5));
    meta.insert("siteName".into(), uid(7));
    meta.insert("$class".into(), uid(8));
    let mut nsurl = Dictionary::new();
    nsurl.insert("NS.base".into(), uid(0));
    nsurl.insert("NS.relative".into(), uid(6));
    nsurl.insert("$class".into(), uid(10));

    let objects = vec![
        "$null".into(),
        payload.into(),
        meta.into(),
        title.into(),
        summary.into(),
        nsurl.into(),
        url.into(),
        site_name.into(),
        class("LPLinkMetadata", &["LPLinkMetadata", "NSObject"]),
        class(
            "RichLinkPluginPayload",
            &["RichLinkPluginPayload", "NSObject"],
        ),
        class("NSURL", &["NSURL", "NSObject"]),
    ];
    let mut top = Dictionary::new();
    top.insert("root".into(), uid(1));
    let mut root = Dictionary::new();
    root.insert("$archiver".into(), "NSKeyedArchiver".into());
    root.insert("$objects".into(), Value::Array(objects));
    root.insert("$top".into(), top.into());
    root.insert("$version".into(), 100_000u64.into());
    binary(root.into())
}
