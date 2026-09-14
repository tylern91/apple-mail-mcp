//! Regression fixture for UseJunior #198: a hand-rolled header encoder in that codebase
//! triple-encoded a non-ASCII `Subject:` into mojibake. `compose` never hand-rolls header
//! encoding — `mail-builder` owns it as a single responsibility (see `amx-compose/src/mime.rs`)
//! — so this asserts the round trip is byte-identical and the raw header carries exactly one
//! `=?utf-8?` encoded-word run, never a nested/double encoding.

use amx_compose::{Draft, EmailAddress, compose};
use mail_parser::MessageParser;

#[test]
fn non_ascii_subject_round_trips_without_double_encoding() {
    let subject = "\u{dc}berweisung \u{2014} \u{652f}\u{6255}\u{3044}";

    let draft = Draft {
        from: Some(EmailAddress::with_name("Alice", "alice@example.com")),
        to: vec![EmailAddress::new("bob@example.com")],
        subject: subject.to_string(),
        text_body: Some("Hi Bob".to_string()),
        ..Draft::default()
    };

    let composed = compose(&draft).expect("compose should succeed");

    let raw_str = String::from_utf8(composed.raw.clone()).expect("raw bytes should be valid utf-8");
    let subject_header_line = raw_str
        .lines()
        .find(|line| line.starts_with("Subject:"))
        .expect("a Subject header must be present");
    let encoded_word_count = subject_header_line.matches("=?utf-8?").count();
    assert_eq!(
        encoded_word_count, 1,
        "expected exactly one utf-8 encoded-word run in the Subject header, found {encoded_word_count}: {subject_header_line:?}"
    );

    let parsed = MessageParser::default()
        .parse(&composed.raw)
        .expect("re-parsing the composed message should succeed");
    let decoded_subject = parsed.subject().expect("subject should be present");
    assert_eq!(
        decoded_subject, subject,
        "decoded subject must be byte-identical to the original, non-mojibake"
    );
}
