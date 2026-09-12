//! The Tantivy schema, ported field-for-field from umbrella §5.1.
//!
//! `body_state`/`attachment_state` are fast `u64` fields holding
//! [`amx_core::BodyState::discriminant`]/[`amx_core::AttachmentState::discriminant`] — tags only,
//! so the coverage envelope (umbrella §4.2.4) is computed from the same query as the search hit
//! set instead of a side ledger that can drift, per §5.1's own note.

use tantivy::schema::{
    DateOptions, FAST, IndexRecordOption, STORED, Schema, SchemaBuilder, TextFieldIndexing,
    TextOptions,
};
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};

/// The name `sender`/`recipients` reference for their edge-ngram, substring-matchable tokenizer
/// (umbrella §5.1). Not one of Tantivy's built-in tokenizers — [`register_tokenizers`] must run
/// against an index before it can write or read documents through this schema.
const EDGE_NGRAM_TOKENIZER: &str = "edge_ngram";

/// Registers [`EDGE_NGRAM_TOKENIZER`] on `index` — required once, before the first write or read.
/// Kept beside the schema so both the writer (task 3) and reader pool (task 6) register the same
/// definition rather than each defining their own.
pub fn register_tokenizers(index: &tantivy::Index) {
    let tokenizer = NgramTokenizer::prefix_only(1, 20).expect("1 <= 20 is a valid ngram range");
    let analyzer = TextAnalyzer::builder(tokenizer).filter(LowerCaser).build();
    index.tokenizers().register(EDGE_NGRAM_TOKENIZER, analyzer);
}

/// Handles to every field in [`build_schema`]'s schema, so callers never re-derive a field by
/// string name (a typo there would fail silently at query time, not at compile time).
#[derive(Debug, Clone, Copy)]
pub struct Fields {
    pub rowid: tantivy::schema::Field,
    pub account_id: tantivy::schema::Field,
    pub mailbox_key: tantivy::schema::Field,
    pub subject: tantivy::schema::Field,
    pub sender: tantivy::schema::Field,
    pub recipients: tantivy::schema::Field,
    pub body: tantivy::schema::Field,
    pub attachment_text: tantivy::schema::Field,
    pub date_sent: tantivy::schema::Field,
    pub date_received: tantivy::schema::Field,
    pub flags: tantivy::schema::Field,
    pub thread_id: tantivy::schema::Field,
    pub body_state: tantivy::schema::Field,
    pub attachment_state: tantivy::schema::Field,
}

/// Builds umbrella §5.1's schema and returns it alongside typed handles to each field.
pub fn build_schema() -> (Schema, Fields) {
    let mut builder = SchemaBuilder::new();

    let raw_indexed_stored = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("raw")
                .set_index_option(IndexRecordOption::Basic),
        )
        .set_stored();

    let rowid = builder.add_i64_field("rowid", FAST | STORED);
    let account_id = builder.add_text_field("account_id", raw_indexed_stored.clone());
    let mailbox_key = builder.add_text_field("mailbox_key", raw_indexed_stored);

    let subject = builder.add_text_field(
        "subject",
        TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("en_stem")
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            )
            .set_stored(),
    );

    let edge_ngram = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("edge_ngram")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();
    let sender = builder.add_text_field("sender", edge_ngram.clone());
    let recipients = builder.add_text_field("recipients", edge_ngram);

    let en_stem_unstored = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("en_stem")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let body = builder.add_text_field("body", en_stem_unstored.clone());
    let attachment_text = builder.add_text_field("attachment_text", en_stem_unstored);

    let date_opts = DateOptions::from(FAST).set_stored();
    let date_sent = builder.add_date_field("date_sent", date_opts.clone());
    let date_received = builder.add_date_field("date_received", date_opts);

    let flags = builder.add_u64_field("flags", FAST | STORED);
    let thread_id = builder.add_i64_field("thread_id", FAST | STORED);
    let body_state = builder.add_u64_field("body_state", FAST | STORED);
    let attachment_state = builder.add_u64_field("attachment_state", FAST | STORED);

    let schema = builder.build();
    let fields = Fields {
        rowid,
        account_id,
        mailbox_key,
        subject,
        sender,
        recipients,
        body,
        attachment_text,
        date_sent,
        date_received,
        flags,
        thread_id,
        body_state,
        attachment_state,
    };
    (schema, fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_umbrella_5_1_field_is_present_with_the_documented_options() {
        let (schema, _fields) = build_schema();

        let entry = |name: &str| schema.get_field(name).unwrap();

        // fast + stored join/path-stem key
        assert!(schema.get_field_entry(entry("rowid")).is_fast());
        assert!(schema.get_field_entry(entry("rowid")).is_stored());

        // raw indexed + stored identity fields
        for name in ["account_id", "mailbox_key"] {
            let field = entry(name);
            assert!(schema.get_field_entry(field).is_indexed());
            assert!(schema.get_field_entry(field).is_stored());
        }

        // body/attachment_text: indexed, never stored — re-read from .emlx on demand
        for name in ["body", "attachment_text"] {
            let field = entry(name);
            assert!(schema.get_field_entry(field).is_indexed());
            assert!(!schema.get_field_entry(field).is_stored());
        }

        // subject/sender/recipients: indexed + stored
        for name in ["subject", "sender", "recipients"] {
            let field = entry(name);
            assert!(schema.get_field_entry(field).is_indexed());
            assert!(schema.get_field_entry(field).is_stored());
        }

        // fast + stored scalar/range fields
        for name in [
            "date_sent",
            "date_received",
            "flags",
            "thread_id",
            "body_state",
            "attachment_state",
        ] {
            let field = entry(name);
            assert!(schema.get_field_entry(field).is_fast());
            assert!(schema.get_field_entry(field).is_stored());
        }
    }

    #[test]
    fn schema_round_trips_through_a_real_index() {
        use tantivy::doc;

        let (schema, fields) = build_schema();
        let dir = tempfile::tempdir().unwrap();
        let index = tantivy::Index::create_in_dir(dir.path(), schema.clone()).unwrap();
        register_tokenizers(&index);
        let mut writer = index.writer(15_000_000).unwrap();

        let mut doc = doc!(
            fields.rowid => 42_260i64,
            fields.account_id => "AB4FC904-21FE-4AC0-A089-716246CE5C46",
            fields.mailbox_key => "[gmail]/tất cả thư",
            fields.subject => "Test subject",
            fields.sender => "alice@example.com",
            fields.recipients => "bob@example.com",
            fields.body => "hello world",
            fields.attachment_text => "extracted pdf text",
            fields.flags => 0u64,
            fields.thread_id => 1i64,
            fields.body_state => 0u64,
            fields.attachment_state => 1u64,
        );
        doc.add_date(
            fields.date_sent,
            tantivy::DateTime::from_timestamp_secs(1_700_000_000),
        );
        doc.add_date(
            fields.date_received,
            tantivy::DateTime::from_timestamp_secs(1_700_000_100),
        );
        writer.add_document(doc).unwrap();
        writer.commit().unwrap();

        let reader = index.reader().unwrap();
        let searcher = reader.searcher();
        assert_eq!(searcher.num_docs(), 1);
    }
}
