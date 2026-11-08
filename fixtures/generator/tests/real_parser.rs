//! The fixture must be readable by the parser the ingester uses.

use std::collections::BTreeMap;

use fixture_gen::{SMS_DB, file_id, generate, scenario};
use imessage_database::{
    tables::{capabilities::Capabilities, messages::Message, table::get_connection},
    util::query_context::QueryContext,
};

#[test]
fn imessage_database_decodes_every_message_text() {
    let dir = tempfile::tempdir().unwrap();
    let expected = generate(dir.path()).unwrap();
    let id = file_id(SMS_DB.0, SMS_DB.1);
    let db = get_connection(&dir.path().join(&id[..2]).join(&id)).unwrap();

    let capabilities = Capabilities::determine(&db).unwrap();
    let context = QueryContext::default();
    let mut stmt = Message::stream_rows(&db, &capabilities, &context).unwrap();
    let decoded: BTreeMap<String, Option<String>> = Message::rows(&mut stmt, [])
        .unwrap()
        .map(|m| {
            let m = m.unwrap();
            (m.guid, m.text)
        })
        .collect();

    assert_eq!(decoded.len(), expected.messages);
    let mut body_only = 0;
    for m in scenario::build().messages {
        assert_eq!(decoded[&m.guid], m.text, "text of {}", m.guid);
        body_only += usize::from(m.text_only_in_body);
    }
    assert!(
        body_only > 0,
        "some messages must keep text only in attributedBody"
    );
}
