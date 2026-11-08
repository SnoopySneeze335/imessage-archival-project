//! The fixture must be readable by the parser the ingester uses.

use std::collections::BTreeMap;

use fixture_gen::{SMS_DB, Variant, file_id, generate_variant, scenario};
use imessage_database::{
    message_types::{url::URLMessage, variants::BalloonProvider},
    tables::{capabilities::Capabilities, messages::Message, table::get_connection},
    util::{dates::TIMESTAMP_FACTOR, plist::parse_ns_keyed_archiver, query_context::QueryContext},
};

fn scenario_of(variant: Variant) -> scenario::Scenario {
    match variant {
        Variant::V1 => scenario::build(),
        Variant::V2 => scenario::build_v2(),
    }
}

#[test]
fn imessage_database_decodes_every_message() {
    for variant in [Variant::V1, Variant::V2] {
        let dir = tempfile::tempdir().unwrap();
        let expected = generate_variant(dir.path(), variant).unwrap();
        let id = file_id(SMS_DB.0, SMS_DB.1);
        let db = get_connection(&dir.path().join(&id[..2]).join(&id)).unwrap();

        let capabilities = Capabilities::determine(&db).unwrap();
        let context = QueryContext::default();
        let mut stmt = Message::stream_rows(&db, &capabilities, &context).unwrap();
        let decoded: BTreeMap<String, Message> = Message::rows(&mut stmt, [])
            .unwrap()
            .map(|m| {
                let m = m.unwrap();
                (m.guid.clone(), m)
            })
            .collect();
        assert_eq!(decoded.len(), expected.messages, "{variant:?}");

        let (mut body_only, mut histories, mut previews) = (0, 0, 0);
        for m in scenario_of(variant).messages {
            let d = &decoded[&m.guid];
            assert_eq!(d.text, m.text, "text of {} in {variant:?}", m.guid);
            body_only += usize::from(m.text_only_in_body);

            if !m.edit_history.is_empty() {
                histories += 1;
                let parts = d.edited_parts.as_ref().expect("edit history parsed");
                let events: Vec<(i64, &str)> = parts.parts[0]
                    .edit_history
                    .iter()
                    .map(|e| (e.date / TIMESTAMP_FACTOR, e.text.as_str()))
                    .collect();
                let want: Vec<(i64, &str)> = m
                    .edit_history
                    .iter()
                    .map(|(at, t)| (scenario::apple_s(*at), t.as_str()))
                    .collect();
                assert_eq!(events, want, "edit history of {}", m.guid);
            }

            if let Some(l) = &m.link_preview {
                previews += 1;
                assert!(d.is_url(), "{} is a link preview", m.guid);
                let payload = d.payload_data(&db).expect("payload_data");
                let parsed = parse_ns_keyed_archiver(&payload).unwrap();
                let url = URLMessage::from_map(&parsed).unwrap();
                assert_eq!(url.url, Some(l.url));
                assert_eq!(url.title, Some(l.title));
                assert_eq!(url.summary, Some(l.summary));
                assert_eq!(url.site_name, Some(l.site_name));
            }
        }
        assert!(body_only > 0, "some text lives only in attributedBody");
        assert!(
            histories > 0 && previews > 0,
            "{variant:?} has edits and previews"
        );
        assert_eq!(
            histories,
            scenario_of(variant)
                .messages
                .iter()
                .filter(|m| !m.edit_history.is_empty())
                .count()
        );
    }
}
