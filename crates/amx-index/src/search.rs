//! Turns a free-text query plus already-resolved filters into a ranked tantivy query, and
//! returns hits alongside the coverage envelope computed over the identical query (umbrella
//! §4.2.4) — a single [`MultiCollector`] pass, never a side ledger that can drift.
//!
//! `amx-mcp`'s `search_messages` tool (Phase 3 task 4) wraps this. Filter resolution against
//! `MailboxRegistry`/`AccountResolver` happens in that caller, before a [`SearchRequest`] is
//! built here: umbrella §5.2 requires an unmatched mailbox/account to be `MailboxFilterUnmatched`
//! with fuzzy suggestions, never `hits: []` — a decision this module cannot make, since it only
//! ever sees filter values the caller has already confirmed exist. A filter that legitimately
//! matches zero *documents* (as opposed to zero *mailboxes*) still yields `hits: []` here.

use std::ops::Bound;

use amx_core::AmxError;
use tantivy::collector::{MultiCollector, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, Occur, Query, QueryParser, RangeQuery, TermQuery};
use tantivy::schema::{IndexRecordOption, Value};
use tantivy::{DateTime, Index, Searcher, TantivyDocument, Term};

use crate::coverage::{CoverageCollector, CoverageEnvelope};
use crate::schema::Fields;

/// `subject` outranks the other free-text fields by this factor (umbrella §5.1: "boosted ×3").
const SUBJECT_BOOST: f32 = 3.0;

/// Search parameters. `mailbox_key`/`account_id` must already be values that passed through
/// `MailboxRegistry::resolve`/`AccountResolver::resolve` — see the module docs.
#[derive(Debug, Clone, Default)]
pub struct SearchRequest {
    /// Free-text query across `subject`/`body`/`sender`/`recipients`/`attachment_text`. Empty
    /// matches every document (e.g. a filter-only browse).
    pub query: String,
    pub mailbox_key: Option<String>,
    pub account_id: Option<String>,
    pub sender: Option<String>,
    /// Inclusive Unix-second bounds on `date_sent`.
    pub date_sent_from: Option<i64>,
    pub date_sent_to: Option<i64>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SearchHit {
    pub rowid: i64,
    pub score: f32,
    pub subject: String,
    pub sender: String,
    pub mailbox_key: String,
    pub account_id: String,
    pub date_sent: Option<i64>,
    pub thread_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub coverage: CoverageEnvelope,
}

pub struct Search;

impl Search {
    /// Runs `request` against `searcher`, reading hits and the coverage envelope off the same
    /// query in one `MultiCollector` pass (umbrella §4.2.4).
    pub fn run(
        searcher: &Searcher,
        fields: &Fields,
        request: &SearchRequest,
    ) -> Result<SearchResponse, AmxError> {
        let query = Self::build_query(searcher.index(), fields, request)?;

        let mut collectors = MultiCollector::new();
        let top_docs_handle = collectors
            .add_collector(TopDocs::with_limit(request.limit + request.offset).order_by_score());
        let coverage_handle = collectors.add_collector(CoverageCollector::new(*fields));

        let mut fruits = searcher.search(&query, &collectors)?;
        let top_docs = top_docs_handle.extract(&mut fruits);
        let coverage = coverage_handle.extract(&mut fruits);

        let hits = top_docs
            .into_iter()
            .skip(request.offset)
            .map(|(score, address)| {
                let doc: TantivyDocument = searcher.doc(address)?;
                Ok(Self::to_hit(fields, score, &doc))
            })
            .collect::<Result<Vec<_>, AmxError>>()?;

        Ok(SearchResponse { hits, coverage })
    }

    fn build_query(
        index: &Index,
        fields: &Fields,
        request: &SearchRequest,
    ) -> Result<Box<dyn Query>, AmxError> {
        let mut clauses: Vec<(Occur, Box<dyn Query>)> =
            vec![(Occur::Must, Self::text_query(index, fields, request)?)];

        if let Some(mailbox_key) = &request.mailbox_key {
            clauses.push((
                Occur::Must,
                Self::term_query(fields.mailbox_key, mailbox_key),
            ));
        }
        if let Some(account_id) = &request.account_id {
            clauses.push((Occur::Must, Self::term_query(fields.account_id, account_id)));
        }
        if let Some(sender) = &request.sender {
            clauses.push((Occur::Must, Self::term_query(fields.sender, sender)));
        }
        if request.date_sent_from.is_some() || request.date_sent_to.is_some() {
            clauses.push((
                Occur::Must,
                Self::date_range_query(fields, request.date_sent_from, request.date_sent_to),
            ));
        }

        Ok(Box::new(BooleanQuery::new(clauses)))
    }

    /// The boosted free-text query over `subject`/`body`/`sender`/`recipients`/
    /// `attachment_text`. An empty query string matches every document, since a request with
    /// only filters set (e.g. "everything in this mailbox") is a legitimate browse, not an
    /// invalid search.
    fn text_query(
        index: &Index,
        fields: &Fields,
        request: &SearchRequest,
    ) -> Result<Box<dyn Query>, AmxError> {
        if request.query.trim().is_empty() {
            return Ok(Box::new(AllQuery));
        }

        let mut parser = QueryParser::for_index(
            index,
            vec![
                fields.subject,
                fields.body,
                fields.sender,
                fields.recipients,
                fields.attachment_text,
            ],
        );
        parser.set_field_boost(fields.subject, SUBJECT_BOOST);
        parser
            .parse_query(&request.query)
            .map_err(|err| AmxError::SearchQueryInvalid {
                query: request.query.clone(),
                reason: err.to_string(),
            })
    }

    /// An exact-match filter against one of the raw/edge-ngram identity fields. These fields are
    /// indexed (not just stored), so a `TermQuery` — not a stored-value comparison — is what
    /// actually filters at the collector level.
    fn term_query(field: tantivy::schema::Field, value: &str) -> Box<dyn Query> {
        Box::new(TermQuery::new(
            Term::from_field_text(field, value),
            IndexRecordOption::Basic,
        ))
    }

    fn date_range_query(fields: &Fields, from: Option<i64>, to: Option<i64>) -> Box<dyn Query> {
        let lower = from.map_or(Bound::Unbounded, |secs| {
            Bound::Included(Term::from_field_date(
                fields.date_sent,
                DateTime::from_timestamp_secs(secs),
            ))
        });
        let upper = to.map_or(Bound::Unbounded, |secs| {
            Bound::Included(Term::from_field_date(
                fields.date_sent,
                DateTime::from_timestamp_secs(secs),
            ))
        });
        Box::new(RangeQuery::new(lower, upper))
    }

    /// `subject`/`sender`/`date_sent`/`thread_id` are absent when the source message lacked them
    /// (`sync.rs`'s `build_document` only sets a field when `parsed` carries it) — a search hit
    /// reflects that with an empty string / `None` rather than a stand-in value.
    fn to_hit(fields: &Fields, score: f32, doc: &TantivyDocument) -> SearchHit {
        let text = |field: tantivy::schema::Field| -> String {
            doc.get_first(field)
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string()
        };

        SearchHit {
            rowid: doc
                .get_first(fields.rowid)
                .and_then(|value| value.as_i64())
                .unwrap_or_default(),
            score,
            subject: text(fields.subject),
            sender: text(fields.sender),
            mailbox_key: text(fields.mailbox_key),
            account_id: text(fields.account_id),
            date_sent: doc
                .get_first(fields.date_sent)
                .and_then(|value| value.as_datetime())
                .map(|date| date.into_utc().unix_timestamp()),
            thread_id: doc
                .get_first(fields.thread_id)
                .and_then(|value| value.as_i64()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{build_schema, register_tokenizers};
    use tantivy::directory::RamDirectory;
    use tantivy::doc;

    /// Builds an in-memory index with a handful of documents, commits, and returns a
    /// `Searcher` over them — enough to exercise ranking/filters/coverage without touching disk
    /// (kept Linux-safe per the umbrella §3 portable/darwin split).
    fn seeded_index() -> (tantivy::Index, Fields) {
        let (schema, fields) = build_schema();
        let index = tantivy::Index::create(RamDirectory::create(), schema, Default::default())
            .expect("in-memory index");
        register_tokenizers(&index);

        let mut writer = index.writer(15_000_000).expect("writer");

        // rowid 1: subject match only.
        writer
            .add_document(doc!(
                fields.rowid => 1i64,
                fields.account_id => "acct-a",
                fields.mailbox_key => "inbox",
                fields.subject => "quarterly roadmap",
                fields.sender => "Alice <alice@example.com>",
                fields.body => "nothing relevant here",
                fields.date_sent => DateTime::from_timestamp_secs(1_700_000_000),
                fields.body_state => 0u64,
                fields.attachment_state => 0u64,
            ))
            .unwrap();

        // rowid 2: body match only, different mailbox/account.
        writer
            .add_document(doc!(
                fields.rowid => 2i64,
                fields.account_id => "acct-b",
                fields.mailbox_key => "archive",
                fields.subject => "unrelated subject",
                fields.sender => "Bob <bob@example.com>",
                fields.body => "the roadmap ships next quarter",
                fields.date_sent => DateTime::from_timestamp_secs(1_600_000_000),
                fields.body_state => 0u64,
                fields.attachment_state => 0u64,
            ))
            .unwrap();

        writer.commit().unwrap();
        (index, fields)
    }

    #[test]
    fn subject_match_outranks_body_only_match() {
        let (index, fields) = seeded_index();
        let searcher = index.reader().unwrap().searcher();

        let request = SearchRequest {
            query: "roadmap".to_string(),
            limit: 10,
            ..Default::default()
        };
        let response = Search::run(&searcher, &fields, &request).unwrap();

        assert_eq!(response.hits.len(), 2);
        assert_eq!(
            response.hits[0].rowid, 1,
            "subject boost must rank it first"
        );
        assert_eq!(response.coverage.corpus_messages, 2);
    }

    #[test]
    fn mailbox_filter_narrows_hits_without_touching_coverage_semantics() {
        let (index, fields) = seeded_index();
        let searcher = index.reader().unwrap().searcher();

        let request = SearchRequest {
            query: "roadmap".to_string(),
            mailbox_key: Some("archive".to_string()),
            limit: 10,
            ..Default::default()
        };
        let response = Search::run(&searcher, &fields, &request).unwrap();

        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].rowid, 2);
    }

    #[test]
    fn a_resolved_filter_matching_no_documents_is_a_legitimate_empty_result() {
        let (index, fields) = seeded_index();
        let searcher = index.reader().unwrap().searcher();

        let request = SearchRequest {
            query: "roadmap".to_string(),
            mailbox_key: Some("does-not-exist-in-this-index".to_string()),
            limit: 10,
            ..Default::default()
        };
        let response = Search::run(&searcher, &fields, &request).unwrap();

        // Umbrella §5.2: `hits: []` is only wrong when the *filter itself* is unmatched against
        // the registry — that check happens upstream of this module. Here the filter value is
        // syntactically fine and simply matches nothing, which is a true statement about the
        // corpus. The coverage envelope reflects the same filtered query (coverage.rs's own
        // contract), so it is empty too, not the full 2-document corpus.
        assert!(response.hits.is_empty());
        assert_eq!(response.coverage.corpus_messages, 0);
    }

    #[test]
    fn date_range_excludes_documents_outside_the_bound() {
        let (index, fields) = seeded_index();
        let searcher = index.reader().unwrap().searcher();

        let request = SearchRequest {
            query: String::new(),
            date_sent_from: Some(1_650_000_000),
            limit: 10,
            ..Default::default()
        };
        let response = Search::run(&searcher, &fields, &request).unwrap();

        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].rowid, 1);
    }

    #[test]
    fn empty_query_with_no_filters_matches_everything() {
        let (index, fields) = seeded_index();
        let searcher = index.reader().unwrap().searcher();

        let request = SearchRequest {
            query: String::new(),
            limit: 10,
            ..Default::default()
        };
        let response = Search::run(&searcher, &fields, &request).unwrap();

        assert_eq!(response.hits.len(), 2);
    }

    #[test]
    fn malformed_query_syntax_is_a_typed_error() {
        let (index, fields) = seeded_index();
        let searcher = index.reader().unwrap().searcher();

        let request = SearchRequest {
            query: "subject:(unterminated".to_string(),
            limit: 10,
            ..Default::default()
        };
        let err = Search::run(&searcher, &fields, &request).unwrap_err();
        assert!(matches!(err, AmxError::SearchQueryInvalid { .. }));
    }
}
