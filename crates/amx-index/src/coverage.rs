//! The coverage envelope (umbrella §4.2.4). Computed by a `Collector` reading `body_state`/
//! `attachment_state` off **the current query's hit set** -- the same query a search ran, never
//! a side ledger -- so it cannot drift the way parasxos' separate `docs` table did. An agent
//! reading this envelope cannot mistake "no such mail exists" for "that mail was never indexed".

use serde::Serialize;
use tantivy::collector::{Collector, SegmentCollector};
use tantivy::columnar::Column;
use tantivy::{DocId, Score, SegmentOrdinal, SegmentReader};

use crate::schema::Fields;

const BODY_INDEXED: u64 = 0;
const BODY_PENDING: u64 = 1;
const BODY_UNAVAILABLE: u64 = 2;
const BODY_QUARANTINED: u64 = 3;

const ATTACHMENT_NONE: u64 = 0;
const ATTACHMENT_EXTRACTED: u64 = 1;
const ATTACHMENT_NOT_DOWNLOADED: u64 = 2;
const ATTACHMENT_UNEXTRACTABLE: u64 = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct BodyCoverage {
    pub indexed: usize,
    pub pending: usize,
    pub unavailable: usize,
    pub quarantined: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct AttachmentCoverage {
    pub messages_with_attachments: usize,
    pub extracted: usize,
    pub not_downloaded: usize,
    pub unextractable: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoverageEnvelope {
    pub corpus_messages: usize,
    pub body: BodyCoverage,
    pub attachments: AttachmentCoverage,
    pub body_searched_fraction: f64,
    pub attachment_searched_fraction: f64,
    pub warnings: Vec<String>,
}

impl CoverageEnvelope {
    fn from_counts(body: BodyCoverage, attachments: AttachmentCoverage) -> Self {
        let corpus_messages = body.indexed + body.pending + body.unavailable + body.quarantined;

        let body_searched_fraction = if corpus_messages == 0 {
            1.0
        } else {
            body.indexed as f64 / corpus_messages as f64
        };
        let attachment_searched_fraction = if attachments.messages_with_attachments == 0 {
            1.0
        } else {
            attachments.extracted as f64 / attachments.messages_with_attachments as f64
        };

        let mut warnings = Vec::new();
        if attachments.not_downloaded > 0 {
            let pct = (attachments.not_downloaded as f64
                / attachments.messages_with_attachments as f64
                * 100.0)
                .round() as u64;
            warnings.push(format!(
                "{pct}% of attachment-bearing messages have no attachment bytes on disk \
                 (.partial.emlx). Attachment-scoped hits are incomplete. Remedy: \
                 `amxcli fetch-full --scope attachments`."
            ));
        }
        if body.unavailable > 0 {
            warnings.push(format!(
                "{} message(s) have no searchable body (encrypted, permission denied, or \
                 pruned by the mail provider) and are permanently excluded from search.",
                body.unavailable
            ));
        }

        Self {
            corpus_messages,
            body,
            attachments,
            body_searched_fraction,
            attachment_searched_fraction,
            warnings,
        }
    }
}

/// A `tantivy::Collector` that tallies `body_state`/`attachment_state` over a query's hit set
/// without materializing the hits themselves.
pub struct CoverageCollector {
    fields: Fields,
}

impl CoverageCollector {
    pub fn new(fields: Fields) -> Self {
        Self { fields }
    }
}

impl Collector for CoverageCollector {
    type Fruit = CoverageEnvelope;
    type Child = SegmentCoverageCollector;

    fn for_segment(
        &self,
        _segment_local_id: SegmentOrdinal,
        segment: &SegmentReader,
    ) -> tantivy::Result<Self::Child> {
        let fast_fields = segment.fast_fields();
        Ok(SegmentCoverageCollector {
            body_state: fast_fields.u64(self.fields_name(self.fields.body_state))?,
            attachment_state: fast_fields.u64(self.fields_name(self.fields.attachment_state))?,
            body: BodyCoverage::default(),
            attachments: AttachmentCoverage::default(),
        })
    }

    fn requires_scoring(&self) -> bool {
        false
    }

    fn merge_fruits(
        &self,
        segment_fruits: Vec<(BodyCoverage, AttachmentCoverage)>,
    ) -> tantivy::Result<CoverageEnvelope> {
        let mut body = BodyCoverage::default();
        let mut attachments = AttachmentCoverage::default();
        for (segment_body, segment_attachments) in segment_fruits {
            body.indexed += segment_body.indexed;
            body.pending += segment_body.pending;
            body.unavailable += segment_body.unavailable;
            body.quarantined += segment_body.quarantined;

            attachments.messages_with_attachments += segment_attachments.messages_with_attachments;
            attachments.extracted += segment_attachments.extracted;
            attachments.not_downloaded += segment_attachments.not_downloaded;
            attachments.unextractable += segment_attachments.unextractable;
        }
        Ok(CoverageEnvelope::from_counts(body, attachments))
    }
}

impl CoverageCollector {
    /// `FastFieldReaders` is string-keyed (there is no field-name lookup from a `Field` handle),
    /// so this looks the schema-fixed names up the same way `reconcile.rs`'s
    /// `collect_indexed_rowids` does for `rowid`.
    fn fields_name(&self, field: tantivy::schema::Field) -> &'static str {
        if field == self.fields.body_state {
            "body_state"
        } else {
            "attachment_state"
        }
    }
}

pub struct SegmentCoverageCollector {
    body_state: Column<u64>,
    attachment_state: Column<u64>,
    body: BodyCoverage,
    attachments: AttachmentCoverage,
}

impl SegmentCollector for SegmentCoverageCollector {
    type Fruit = (BodyCoverage, AttachmentCoverage);

    fn collect(&mut self, doc: DocId, _score: Score) {
        match self.body_state.first(doc) {
            Some(BODY_INDEXED) => self.body.indexed += 1,
            Some(BODY_PENDING) => self.body.pending += 1,
            Some(BODY_UNAVAILABLE) => self.body.unavailable += 1,
            Some(BODY_QUARANTINED) => self.body.quarantined += 1,
            _ => {}
        }

        match self.attachment_state.first(doc) {
            Some(ATTACHMENT_NONE) | None => {}
            Some(ATTACHMENT_EXTRACTED) => {
                self.attachments.messages_with_attachments += 1;
                self.attachments.extracted += 1;
            }
            Some(ATTACHMENT_NOT_DOWNLOADED) => {
                self.attachments.messages_with_attachments += 1;
                self.attachments.not_downloaded += 1;
            }
            Some(ATTACHMENT_UNEXTRACTABLE) => {
                self.attachments.messages_with_attachments += 1;
                self.attachments.unextractable += 1;
            }
            Some(_) => {}
        }
    }

    fn harvest(self) -> Self::Fruit {
        (self.body, self.attachments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{build_schema, register_tokenizers};
    use tantivy::doc;
    use tantivy::query::AllQuery;

    fn add(
        writer: &mut tantivy::IndexWriter,
        fields: &Fields,
        rowid: i64,
        body_state: u64,
        attachment_state: u64,
    ) {
        writer
            .add_document(doc!(
                fields.rowid => rowid,
                fields.body_state => body_state,
                fields.attachment_state => attachment_state,
            ))
            .unwrap();
    }

    #[test]
    fn matches_the_umbrella_4_2_4_worked_example_shape() {
        let (schema, fields) = build_schema();
        let index = tantivy::Index::create_in_ram(schema);
        register_tokenizers(&index);
        let mut writer = index.writer(15_000_000).unwrap();

        // 3 indexed bodies, 1 unavailable; of the indexed, 2 have extracted attachments and
        // 1 has none-downloaded attachments; the unavailable message has no attachments.
        add(&mut writer, &fields, 1, BODY_INDEXED, ATTACHMENT_EXTRACTED);
        add(&mut writer, &fields, 2, BODY_INDEXED, ATTACHMENT_EXTRACTED);
        add(
            &mut writer,
            &fields,
            3,
            BODY_INDEXED,
            ATTACHMENT_NOT_DOWNLOADED,
        );
        add(&mut writer, &fields, 4, BODY_UNAVAILABLE, ATTACHMENT_NONE);
        writer.commit().unwrap();

        let reader = index.reader().unwrap();
        let searcher = reader.searcher();
        let envelope = searcher
            .search(&AllQuery, &CoverageCollector::new(fields))
            .unwrap();

        assert_eq!(envelope.corpus_messages, 4);
        assert_eq!(
            envelope.body,
            BodyCoverage {
                indexed: 3,
                pending: 0,
                unavailable: 1,
                quarantined: 0,
            }
        );
        assert_eq!(
            envelope.attachments,
            AttachmentCoverage {
                messages_with_attachments: 3,
                extracted: 2,
                not_downloaded: 1,
                unextractable: 0,
            }
        );
        assert!((envelope.body_searched_fraction - 0.75).abs() < f64::EPSILON);
        assert!((envelope.attachment_searched_fraction - (2.0 / 3.0)).abs() < f64::EPSILON);
        assert_eq!(envelope.warnings.len(), 2);
    }

    #[test]
    fn an_empty_hit_set_reports_full_coverage_with_no_warnings() {
        let (schema, fields) = build_schema();
        let index = tantivy::Index::create_in_ram(schema);
        register_tokenizers(&index);
        let mut writer: tantivy::IndexWriter = index.writer(15_000_000).unwrap();
        writer.commit().unwrap();

        let reader = index.reader().unwrap();
        let searcher = reader.searcher();
        let envelope = searcher
            .search(&AllQuery, &CoverageCollector::new(fields))
            .unwrap();

        assert_eq!(envelope.corpus_messages, 0);
        assert_eq!(envelope.body_searched_fraction, 1.0);
        assert_eq!(envelope.attachment_searched_fraction, 1.0);
        assert!(envelope.warnings.is_empty());
    }

    #[test]
    fn serializes_to_the_documented_json_shape() {
        let (schema, fields) = build_schema();
        let index = tantivy::Index::create_in_ram(schema);
        register_tokenizers(&index);
        let mut writer = index.writer(15_000_000).unwrap();
        add(&mut writer, &fields, 1, BODY_INDEXED, ATTACHMENT_NONE);
        writer.commit().unwrap();

        let reader = index.reader().unwrap();
        let searcher = reader.searcher();
        let envelope = searcher
            .search(&AllQuery, &CoverageCollector::new(fields))
            .unwrap();

        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["corpus_messages"], 1);
        assert_eq!(json["body"]["indexed"], 1);
        assert_eq!(json["attachments"]["messages_with_attachments"], 0);
        assert!(json.get("body_searched_fraction").is_some());
        assert!(json.get("attachment_searched_fraction").is_some());
        assert!(json["warnings"].is_array());
    }
}
