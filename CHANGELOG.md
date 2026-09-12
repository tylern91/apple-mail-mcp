# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-09-12

### Added

- `amx-core`: error taxonomy (`AmxError`), the two-dimensional coverage model
  (`Availability`/`BodyState`/`AttachmentState`/`UnavailableReason`), core traits
  (`MailStore`/`Automation`/`Transport`), and layered config loading.
- `amx-store`: read-only Envelope Index access (`RoConnection`), Full Disk Access
  probing (`TccProbe`), account resolution against `Accounts4.sqlite`
  (`AccountResolver`), mailbox lookup with percent-decode/NFC/casefold
  normalization (`MailboxRegistry`), the `.emlx` on-disk shard path algorithm
  (`EmlxPathResolver`), and filtered message queries (`MessageQuery`). The crate
  is now portable (Linux-buildable) apart from its macOS-only `tcc`/`watch`
  modules.
- `amx-parse`: `.emlx` container parsing via `mail-parser`, the
  attachment-completeness oracle (declared vs. on-disk size), and attachment
  content extraction for HTML, PDF, and Office Open XML (DOCX/PPTX/XLSX).
- `amxcli doctor`: a read-only readiness screen reporting Full Disk Access
  state, store version, account/mailbox counts, and raw `.emlx` coverage
  (clean-parse / quarantine / partial).

### Fixed

- `.emlx` test fixtures are now marked binary in `.gitattributes` so
  `core.autocrlf` can no longer silently corrupt them on checkout.

## [0.1.0] - 2026-09-12

### Added

- Initial workspace scaffold: nine-crate layout, CI/CD workflows, release
  scripting, and repo metadata. No application logic yet.
