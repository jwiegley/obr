//! Bridge module for converting between obr `Issue` records and Org-mode text.
//!
//! Serialization is record-oriented to match the streaming/parallel export
//! pipeline: [`org_file_header`] emits the file preamble once, and
//! [`emit_issue_record`] renders one issue as a self-contained level-1
//! heading block. [`org_text_to_issues`] parses a whole Org document back
//! into issues via the `org2jsonl` crate.
//!
//! Format contract (stable; the export is a wire format — `PLAN.org` when
//! it is the tracked surface, `issues.org` for a workspace still exporting
//! in-dir):
//! - One level-1 heading per issue: `* KEYWORD [#P] title    :tags:`. The gap
//!   before the tags is four spaces unless the surface carries an
//!   `org-tags-column` file-local variable, in which case Org's own alignment
//!   applies — see [`OrgStyle`].
//! - Issue fields in a `:PROPERTIES:` drawer; `:ID:` is the only property
//!   required on read.
//! - Instants are Org-native inactive timestamps in the machine's LOCAL zone
//!   — `:CREATED:  [2026-08-09 Sun 12:31]` — so Org's own tooling can read
//!   them: `org-sort-entries` by property (the format sorts lexicographically
//!   in chronological order), `org-entry-get` and `org-ql`, column view, and
//!   `C-c .` to edit one. Inactive is a deliberate CHOICE, not a limitation:
//!   an ACTIVE `<…>` timestamp in a property drawer *does* reach the day
//!   agenda (verified in Org 9.7.11), and putting 549 issues there is exactly
//!   what nobody wants. Writing `<…>` is the available way to opt in.
//!   Seven properties carry an instant: `CREATED`, `MODIFIED`, `FINISHED`,
//!   `DUE`, `DEFERRED`, `DELETED`, `COMPACTED`. Their pre-Org spellings
//!   (`CREATED_AT`, `UPDATED_AT`, `CLOSED_AT`, `DUE_AT`, `DEFER_UNTIL`,
//!   `DELETED_AT`, `COMPACTED_AT`, all RFC3339) are still read so an older
//!   file imports losslessly, and nothing writes them. The consequences —
//!   minute precision becomes the data model, and the file is not
//!   byte-identical across machine zones — are stated in
//!   `docs/RESIDUALS.md`.
//! - `closed_at` is spelled `:FINISHED:`, not `:CLOSED:`, because `CLOSED` is
//!   in `org-special-properties`: Org shadows the drawer key with the
//!   `CLOSED:` planning-line keyword, so `org-entry-get` returns nil for it
//!   and `org-entry-put` signals an error. A short-lived build spelled it
//!   `:CLOSED:`; that spelling is read as legacy, alongside `:CLOSED_AT:`,
//!   so no file loses its close instant. See `docs/RESIDUALS.md` for the
//!   verified `org-special-properties` list to check a new property against.
//! - The `:LABELS:` property (compact JSON array) is the authoritative label
//!   set; heading tags are emitted for Org ergonomics and only consulted when
//!   `:LABELS:` is absent (hand-edited or legacy files). This keeps labels
//!   containing `:` (e.g. the `provides:<capability>` wire format) lossless.
//! - Dependencies, comments, and `agent_context` ride in JSON `src` blocks
//!   under fixed level-2 child headings. Those three sections must carry a
//!   well-formed block: a hand-edit that breaks one fails the import rather
//!   than importing as empty and deleting the relations on the next flush.
//! - The level-2 sections an issue may carry are [`ORG_CHILD_SECTIONS`] and
//!   only those; the file is rewritten from the database on every flush, so
//!   anything else warns on import and does not survive.
//! - Emission is deterministic and carries no cross-issue state, so the
//!   parallel export path may shard records across threads. It is not a pure
//!   function of the issue alone: rendering an instant reads the machine's
//!   local zone, which is the same for every thread in a run.
//! - The file is a fixpoint from the first write. Values the format cannot
//!   represent (edge blank lines in free text, padded property values and
//!   titles, a `Pinned` status with the flag clear) are normalized once, at
//!   emission, so generation one already equals generation two; see
//!   [`canonical_body_text`] and `docs/RESIDUALS.md`.
//!
//! Statuses that cannot be represented as an Org keyword (`Status::Custom`)
//! are refused loudly at export rather than silently mangled; see
//! [`emit_issue_record`].

use crate::error::{BeadsError, Result};
use crate::model::{Issue, IssueType, Priority, Status};
use chrono::offset::LocalResult;
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone, Utc};
use org2jsonl::model::{Element, EntryContent, Heading, InlineContent, OrgEntry, Property};
use std::path::Path;
use std::str::FromStr;
use unicode_width::UnicodeWidthStr;

/// Flat-file export format, chosen by file extension.
///
/// This is the single format probe for the whole sync layer. Never re-derive
/// the extension inline; dispatch through [`ExportFormat::for_path`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// One JSON object per line (upstream's native format).
    Jsonl,
    /// Org-mode document (this module).
    Org,
}

impl ExportFormat {
    /// Choose the format for `path`: `.org` (case-insensitive) selects Org,
    /// anything else is JSONL. Atomic-write temp names (`issues.org.tmp`,
    /// `issues.org.<pid>.tmp`) resolve to their wire format so staged files
    /// are verified as what they contain.
    #[must_use]
    pub fn for_path(path: &Path) -> Self {
        Self::declared_for_path(path).unwrap_or(Self::Jsonl)
    }

    /// The format `path`'s name actually declares, or `None` when it declares
    /// neither.
    ///
    /// [`Self::for_path`] answers "how do I read this?" and defaults to JSONL,
    /// which is right for reading but wrong for validating a user-supplied
    /// target. Callers that must reject a non-export path ask here instead of
    /// spelling the extension pair themselves — the spellings drifted
    /// otherwise (`obr vcs-status` rejected `PLAN.ORG` while sync accepted it,
    /// and several sites missed the staged `.org.<pid>.tmp` forms entirely).
    #[must_use]
    pub fn declared_for_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?;

        // Plain extensions are matched on the raw `OsStr`, so a leaf whose
        // bytes are not valid UTF-8 is still classified by its extension —
        // `vcs-status` accepts such targets deliberately and must not lose
        // them to a lossy conversion here.
        let extension = Path::new(name).extension();
        if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("org")) {
            return Some(Self::Org);
        }
        if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl")) {
            return Some(Self::Jsonl);
        }

        // Staged temp names (`issues.org.tmp`, `issues.org.<pid>.tmp`) need
        // string handling. Those are obr's own writes, always valid UTF-8.
        let lower = name.to_str()?.to_ascii_lowercase();
        let base = lower
            .strip_suffix(".tmp")
            .map_or(lower.as_str(), |stripped| {
                stripped
                    .trim_end_matches(|c: char| c.is_ascii_digit())
                    .trim_end_matches('.')
            });
        // `lower` is already lowercased, so the comparisons are effectively
        // case-insensitive.
        #[allow(clippy::case_sensitive_file_extension_comparisons)]
        if base.ends_with(".org") {
            Some(Self::Org)
        } else if base.ends_with(".jsonl") {
            Some(Self::Jsonl)
        } else {
            None
        }
    }

    /// The extension used for this format's export temp files.
    #[must_use]
    pub const fn temp_extension(self) -> &'static str {
        match self {
            Self::Jsonl => "jsonl.tmp",
            Self::Org => "org.tmp",
        }
    }

    /// The on-disk wire extension for this format (no leading dot).
    #[must_use]
    pub const fn wire_extension(self) -> &'static str {
        match self {
            Self::Jsonl => "jsonl",
            Self::Org => "org",
        }
    }

    /// True when this is the Org format.
    #[must_use]
    pub const fn is_org(self) -> bool {
        matches!(self, Self::Org)
    }
}

/// Schema version stamped into every emitted drawer (write-only; both the
/// current and the legacy key spelling are accepted and ignored on read).
const ORG_SCHEMA_VERSION: u32 = 17;

/// Active (incomplete) TODO keywords recognized in issue files.
pub const ORG_TODO_KEYWORDS: &[&str] = &["TODO", "DOING", "DRAFT", "WAIT", "DEFER", "NOTE"];

/// Completed TODO keywords recognized in issue files.
pub const ORG_DONE_KEYWORDS: &[&str] = &["DONE", "CANCELED"];

/// The level-2 sections an issue heading may carry, in emission order.
///
/// These are the only child headings that map to an `Issue` field. Anything
/// else under an issue is a hand-edit obr cannot store: it warns on import
/// (see [`UNRECOGNIZED_ORG_SECTION_CODE`]) and the next flush drops it.
pub const ORG_CHILD_SECTIONS: &[&str] = &[
    "Design",
    "Acceptance Criteria",
    "Notes",
    "Close Reason",
    "Delete Reason",
    "Agent Context",
    "Dependencies",
    "Comments",
];

/// Warning code for a level-2 section obr does not model.
pub const UNRECOGNIZED_ORG_SECTION_CODE: &str = "UNRECOGNIZED_ORG_SECTION";

/// The file preamble: title, keyword declaration, one blank line.
///
/// Written even for an empty issue set. The `#+SEQ_TODO:` line exists for
/// Org-mode rendering only — parsing always uses the compiled-in keyword
/// arrays, never the file's declaration.
#[must_use]
pub fn org_file_header() -> &'static [u8] {
    b"#+TITLE: Obr Issues\n#+SEQ_TODO: TODO DOING DRAFT WAIT DEFER NOTE | DONE CANCELED\n\n"
}

/// File keyword carrying the workspace's issue prefix.
pub const ISSUE_PREFIX_KEYWORD: &str = "#+ISSUE_PREFIX:";

/// The Emacs file-local variable that carries the surface's tag column.
pub const ORG_TAGS_COLUMN_VARIABLE: &str = "org-tags-column";

/// Drawer property names obr owns, and which must therefore never be captured
/// as "unrecognized" and re-emitted alongside the authoritative value.
///
/// Three groups, and each is here for a different reason:
///
/// 1. Every key [`parse_properties`] reads. A preserved duplicate is emitted
///    *after* the authoritative one, and the parse loop assigns on every hit,
///    so the duplicate would win on the next read. `ID` is the worst case: it
///    would silently reassign identity.
/// 2. The retired spellings. These are the subtle ones. Eight parse arms are
///    guarded — `"COMPACTED_AT" if !carries_property(properties, "COMPACTED")`
///    — and a failed Rust match guard falls through to the *next* arm and
///    thence to the wildcard. So `:CREATED_AT:` reaches the wildcard whenever
///    `:CREATED:` is also present, which is always on a file any current build
///    wrote. Capturing them would resurrect the retired spellings forever, and
///    clearing `due_at` through the CLI would remove `:DUE:` and unshadow a
///    preserved `:DUE_AT:`, restoring a deleted instant.
/// 3. `PROPERTIES` and `END`, which are drawer delimiters rather than
///    properties — but orgize parses `:END: value` as a property named `END`
///    (`node_property_node` succeeds because `space1` finds the space), and
///    re-emitting that mid-drawer truncates the entry.
///
/// `OBR_SCHEMA_VERSION` is written but never parsed, so it reaches the
/// wildcard on every single entry; `BEADS_SCHEMA_VERSION` is its pre-rename
/// spelling.
const RESERVED_ORG_PROPERTY_KEYS: &[&str] = &[
    "ASSIGNEE",
    "BEADS_SCHEMA_VERSION",
    "CLOSED",
    "CLOSED_AT",
    "CLOSED_BY_SESSION",
    "CLOSE_REASON",
    "COMPACTED",
    "COMPACTED_AT",
    "COMPACTED_AT_COMMIT",
    "COMPACTION_LEVEL",
    "CREATED",
    "CREATED_AT",
    "CREATED_BY",
    "DEFERRED",
    "DEFER_UNTIL",
    "DELETED",
    "DELETED_AT",
    "DELETED_BY",
    "DELETE_REASON",
    "DUE",
    "DUE_AT",
    "END",
    "EPHEMERAL",
    "ESTIMATED_MINUTES",
    "EXTERNAL_REF",
    "FINISHED",
    "ID",
    "ISSUE_TYPE",
    "IS_TEMPLATE",
    "LABELS",
    "MODIFIED",
    "OBR_SCHEMA_VERSION",
    "ORIGINAL_SIZE",
    "ORIGINAL_TYPE",
    "OWNER",
    "PINNED",
    "PROPERTIES",
    "SENDER",
    "SOURCE_REPO",
    "SOURCE_REPO_PATH",
    "SOURCE_SYSTEM",
    "TITLE",
    "UPDATED_AT",
];

/// True when a drawer property belongs to obr and must not be preserved
/// verbatim. Case-insensitive: Org property names are case-insensitive on read.
#[must_use]
pub fn is_reserved_org_property(key: &str) -> bool {
    RESERVED_ORG_PROPERTY_KEYS
        .iter()
        .any(|reserved| key.trim().eq_ignore_ascii_case(reserved))
}

/// Drawer properties obr does not model, keyed by issue id, each in the order
/// it appeared in the file.
pub type PreservedProperties = std::collections::HashMap<String, Vec<(String, String)>>;

/// The rendering choices a surface carries that belong to the person editing
/// it rather than to obr.
///
/// Tag alignment is a per-user Emacs preference (`org-tags-column`; Org's own
/// default is `-77`, this project's author uses `-97`), so obr cannot pick a
/// column. It reads one from the file it is rewriting, via the same file-local
/// variable line Emacs itself honors — which is the point: one declaration,
/// both tools, no way for them to disagree.
///
/// Absent that line the historical fixed gap is emitted unchanged, so
/// alignment is opt-in per surface and no existing file is reformatted by
/// upgrading.
/// Not `Copy`: it carries the preserved-property map. The export chain passes
/// it by reference, which the parallel path already supports — its workers run
/// under `thread::scope`, so borrows cross the worker boundary exactly as
/// `&[Issue]` and `&DateTime<Utc>` already do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OrgStyle {
    /// `org-tags-column` with Org's own semantics: positive is the column the
    /// tags start at, negative flushes them right so the line ends at that
    /// column, and zero means exactly one space. `None` keeps the fixed gap.
    pub tags_column: Option<i32>,
    /// Drawer properties another tool wrote that obr does not model, read back
    /// from the surface so a flush does not delete them. Empty for a surface
    /// obr has not seen, and for every writer-based export.
    pub preserved: PreservedProperties,
}

/// Read `org-tags-column` from a first-line file-local variable block.
///
/// Recognizes the shape Emacs itself parses — `# -*- org-tags-column: -97 -*-`,
/// including a multi-variable line such as
/// `# -*- mode: org; org-tags-column: -97 -*-`. Anything unparseable yields
/// `None`, which is the do-nothing answer rather than an error: a malformed
/// preamble must not fail an export.
#[must_use]
pub fn org_tags_column_from_org(text: &str) -> Option<i32> {
    let first = text.lines().next()?;
    let body = first.split_once("-*-")?.1.rsplit_once("-*-")?.0;
    body.split(';')
        .filter_map(|entry| entry.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case(ORG_TAGS_COLUMN_VARIABLE))
        .and_then(|(_, value)| value.trim().parse::<i32>().ok())
}

/// Read a surface's declared [`OrgStyle`] from the file obr is about to
/// rewrite.
///
/// Only the first line is examined, so this is cheap on a large corpus. A
/// missing, unreadable, or undeclared file yields the default style, which
/// emits exactly what every previous build emitted — alignment is opt-in, and
/// upgrading reformats nobody's surface.
#[must_use]
pub fn style_from_surface(path: &Path) -> OrgStyle {
    let Ok(text) = std::fs::read_to_string(path) else {
        return OrgStyle::default();
    };
    OrgStyle {
        tags_column: org_tags_column_from_org(&text),
        preserved: harvest_preserved_properties(&text),
    }
}

/// Collect the drawer properties obr does not model, keyed by issue id.
///
/// Reads what is on disk so the next write can put it back. A file that does
/// not parse yields nothing rather than an error: a malformed surface must not
/// fail an export, and the worst case of yielding nothing is the behavior
/// every build before this one had.
///
/// Values are trimmed, because the drawer parser trims what it reads — an
/// untrimmed value would come back changed and break the fixpoint on the
/// second write. Empty values are dropped for the same reason: they read back
/// as absent.
#[must_use]
pub fn harvest_preserved_properties(text: &str) -> PreservedProperties {
    let mut harvested = PreservedProperties::new();
    let entries = org2jsonl::org_to_json::org_to_entries_with_keywords(
        text,
        ORG_TODO_KEYWORDS,
        ORG_DONE_KEYWORDS,
    );
    for entry in entries {
        let EntryContent::Heading(heading) = entry.content else {
            continue;
        };
        if heading.level != 1 {
            continue;
        }
        let Some(id) = heading
            .properties
            .iter()
            .find(|prop| prop.key.trim().eq_ignore_ascii_case("ID"))
            .map(|prop| prop.value.trim().to_string())
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let kept: Vec<(String, String)> = heading
            .properties
            .iter()
            .filter(|prop| !is_reserved_org_property(&prop.key))
            .map(|prop| (prop.key.trim().to_string(), prop.value.trim().to_string()))
            .filter(|(key, value)| !key.is_empty() && !value.is_empty())
            .collect();
        if !kept.is_empty() {
            harvested.insert(id, kept);
        }
    }
    harvested
}

/// Render the file-local variable line for a surface that declares a tag
/// column, so the declaration survives obr rewriting the file.
#[must_use]
fn org_file_local_line(style: &OrgStyle) -> String {
    match style.tags_column {
        Some(column) => format!("# -*- {ORG_TAGS_COLUMN_VARIABLE}: {column} -*-\n"),
        None => String::new(),
    }
}

/// The Org file header, carrying `#+ISSUE_PREFIX:` when the prefix is known.
///
/// The keyword makes a tracked surface self-describing: after `git clone`,
/// `obr sync --import-only --rebuild` can recover the prefix from the file
/// alone, with no `.obr/config.yaml` present. Readers accept files without it
/// (every corpus written before this keyword existed).
#[must_use]
pub fn org_file_header_for(prefix: Option<&str>) -> Vec<u8> {
    org_file_header_styled(prefix, &OrgStyle::default())
}

/// [`org_file_header_for`] plus the file-local variable line, when the surface
/// declares one.
///
/// The variable line must come first: Emacs only reads a file-local block on
/// the first line (or the second, after a shebang).
#[must_use]
pub fn org_file_header_styled(prefix: Option<&str>, style: &OrgStyle) -> Vec<u8> {
    let mut header = org_file_local_line(style);
    let Some(prefix) = prefix.map(str::trim).filter(|p| !p.is_empty()) else {
        header.push_str(std::str::from_utf8(org_file_header()).unwrap_or_default());
        return header.into_bytes();
    };
    header.push_str(&format!(
        "#+TITLE: Obr Issues\n#+SEQ_TODO: TODO DOING DRAFT WAIT DEFER NOTE | DONE CANCELED\n{ISSUE_PREFIX_KEYWORD} {prefix}\n\n"
    ));
    header.into_bytes()
}

/// Read `#+ISSUE_PREFIX:` from Org text, if present.
///
/// Only the leading keyword block is scanned: file keywords precede the first
/// heading, so this stops at the first `*` line and never walks a large corpus.
#[must_use]
pub fn issue_prefix_from_org(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('*') {
            break;
        }
        if let Some(value) = trimmed.strip_prefix(ISSUE_PREFIX_KEYWORD) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Render one issue as a self-contained Org heading block, including the
/// trailing blank line that separates it from the next record.
///
/// Deterministic and free of cross-issue state, so records may be prepared on
/// worker threads and concatenated in issue order. The one input beyond
/// `issue` is the process's local zone, which every thread reads the same and
/// which does not change under a run; see [`format_org_timestamp`].
///
/// # Errors
///
/// Refuses to serialize an issue whose status is `Status::Custom` (no Org
/// keyword can represent it round-trip); propagates JSON serialization
/// failures for dependencies, comments, and labels instead of silently
/// writing placeholders.
pub fn emit_issue_record(issue: &Issue) -> Result<Vec<u8>> {
    emit_issue_record_styled(issue, &OrgStyle::default())
}

/// [`emit_issue_record`] honoring the surface's [`OrgStyle`].
///
/// # Errors
///
/// Same as [`emit_issue_record`].
pub fn emit_issue_record_styled(issue: &Issue, style: &OrgStyle) -> Result<Vec<u8>> {
    let mut out = String::new();

    // --- Heading line ---
    out.push_str("* ");
    out.push_str(&status_to_keyword(issue)?);
    out.push(' ');
    out.push_str(priority_to_org(issue));
    out.push(' ');
    // Trimmed for the same reason property values are: any Org parser
    // whitespace-delimits a heading title, so `parse_heading_to_issue` trims
    // what it reads and padding would come back changed on the second write.
    let title = sanitize_property_value(issue.title.trim());
    out.push_str(&title);

    let mut sorted_labels = issue.labels.clone();
    sorted_labels.sort();
    // Org tags for ergonomics: only labels that are valid Org tag tokens.
    // The authoritative set is the :LABELS: drawer property below.
    let tag_safe: Vec<&String> = sorted_labels
        .iter()
        .filter(|l| is_org_tag_safe(l))
        .collect();
    if !tag_safe.is_empty() {
        let mut tags = String::from(":");
        for label in &tag_safe {
            tags.push_str(label);
            tags.push(':');
        }
        push_tag_gap(&mut out, &tags, style);
        out.push_str(&tags);
    }
    out.push('\n');

    emit_properties_drawer(&mut out, issue, &title, &sorted_labels, style)?;

    // --- Description body ---
    // Guarded on the canonical form, not the raw one: a description of
    // `Some("\n")` has no representable content, and emitting it would write
    // blank lines that read back as absent and vanish on the next flush.
    // Same rationale as `push_opt_str`.
    if let Some(canon) = body_to_emit(issue.description.as_deref()) {
        out.push('\n');
        push_stable_body(&mut out, &canon);
        out.push('\n');
    }

    // --- Text child sections ---
    push_text_child(&mut out, "Design", issue.design.as_deref());
    push_text_child(
        &mut out,
        "Acceptance Criteria",
        issue.acceptance_criteria.as_deref(),
    );
    push_text_child(&mut out, "Notes", issue.notes.as_deref());

    // Reasons are drawer properties when single-line; a multi-line value
    // would be flattened by the property sanitizer (found on the real
    // corpus: six multi-line close reasons), so it becomes a text child
    // with the same stability guarantees as a description.
    if reason_needs_text_child(issue.close_reason.as_deref()) {
        push_text_child(&mut out, "Close Reason", issue.close_reason.as_deref());
    }
    if reason_needs_text_child(issue.delete_reason.as_deref()) {
        push_text_child(&mut out, "Delete Reason", issue.delete_reason.as_deref());
    }

    // --- Agent context: verbatim stored JSON text in a guarded src block ---
    if let Some(canon) = body_to_emit(issue.agent_context.as_deref()) {
        push_child_heading(&mut out, "Agent Context");
        out.push_str("#+begin_src json\n");
        out.push_str(&escape_block_lines(&canon));
        out.push('\n');
        out.push_str("#+end_src\n");
    }

    // --- Dependencies / comments as machine-serialized JSON src blocks ---
    if !issue.dependencies.is_empty() {
        let json = serde_json::to_string_pretty(&issue.dependencies).map_err(BeadsError::Json)?;
        push_json_child(&mut out, "Dependencies", &json);
    }
    if !issue.comments.is_empty() {
        let json = serde_json::to_string_pretty(&issue.comments).map_err(BeadsError::Json)?;
        push_json_child(&mut out, "Comments", &json);
    }

    // Record separator.
    out.push('\n');
    Ok(out.into_bytes())
}

/// Convert a collection of issues to a complete Org document.
///
/// Convenience wrapper over [`org_file_header`] + [`emit_issue_record`] for
/// whole-file paths (`--no-db` seeding, tests).
///
/// # Errors
///
/// Fails on the first issue [`emit_issue_record`] refuses.
///
/// # Panics
///
/// Never in practice: emission produces valid UTF-8 by construction.
pub fn issues_to_org_text(issues: &[Issue]) -> Result<String> {
    let mut output = Vec::with_capacity(issues.len() * 256 + 64);
    output.extend_from_slice(org_file_header());
    for issue in issues {
        output.extend_from_slice(&emit_issue_record(issue)?);
    }
    // Emission is pure ASCII/UTF-8 by construction.
    Ok(String::from_utf8(output).expect("org emission produced invalid UTF-8"))
}

/// Emit the `:PROPERTIES:` drawer in its fixed order.
fn emit_properties_drawer(
    out: &mut String,
    issue: &Issue,
    title: &str,
    sorted_labels: &[String],
    style: &OrgStyle,
) -> Result<()> {
    out.push_str(":PROPERTIES:\n");
    push_aligned(out, "OBR_SCHEMA_VERSION", &ORG_SCHEMA_VERSION.to_string());
    push_aligned(out, "ID", &sanitize_property_value(&issue.id));
    if title_needs_property_override(title) {
        // A title Org would re-parse with trailing tags is preserved via an
        // authoritative property so the heading-line rendering stays lossless.
        push_aligned(out, "TITLE", title);
    }
    // Authoritative labels. Always emitted (even when empty) so that heading
    // tags are never mistaken for labels on files this build wrote.
    let labels_json = serde_json::to_string(sorted_labels).map_err(BeadsError::Json)?;
    push_aligned(out, "LABELS", &sanitize_property_value(&labels_json));
    push_aligned(
        out,
        "ISSUE_TYPE",
        &sanitize_property_value(issue.issue_type.as_str()),
    );
    push_aligned(out, "CREATED", &format_org_timestamp(issue.created_at));
    push_aligned(out, "MODIFIED", &format_org_timestamp(issue.updated_at));

    push_opt_str(out, "ASSIGNEE", issue.assignee.as_deref());
    push_opt_str(out, "OWNER", issue.owner.as_deref());
    if let Some(minutes) = issue.estimated_minutes {
        push_aligned(out, "ESTIMATED_MINUTES", &minutes.to_string());
    }
    push_opt_str(out, "CREATED_BY", issue.created_by.as_deref());
    // `FINISHED`, not `CLOSED`: `CLOSED` is an org-special-property, shadowed
    // by the planning-line keyword, so Emacs cannot read or write it as a
    // drawer property. See the module header.
    push_opt_time(out, "FINISHED", issue.closed_at);
    // Multi-line reasons cannot survive a drawer property (the sanitizer
    // flattens newlines to spaces); they travel as text children instead —
    // see `emit_issue_record`. Same for DELETE_REASON below.
    if !reason_needs_text_child(issue.close_reason.as_deref()) {
        push_opt_str(out, "CLOSE_REASON", issue.close_reason.as_deref());
    }
    push_opt_str(out, "CLOSED_BY_SESSION", issue.closed_by_session.as_deref());
    push_opt_time(out, "DUE", issue.due_at);
    push_opt_time(out, "DEFERRED", issue.defer_until);
    push_opt_str(out, "EXTERNAL_REF", issue.external_ref.as_deref());
    push_opt_str(out, "SOURCE_SYSTEM", issue.source_system.as_deref());
    push_opt_str(out, "SOURCE_REPO", issue.source_repo.as_deref());
    push_opt_str(out, "SOURCE_REPO_PATH", issue.source_repo_path.as_deref());
    push_opt_time(out, "DELETED", issue.deleted_at);
    push_opt_str(out, "DELETED_BY", issue.deleted_by.as_deref());
    if !reason_needs_text_child(issue.delete_reason.as_deref()) {
        push_opt_str(out, "DELETE_REASON", issue.delete_reason.as_deref());
    }
    push_opt_str(out, "ORIGINAL_TYPE", issue.original_type.as_deref());
    if let Some(level) = issue.compaction_level
        && level > 0
    {
        push_aligned(out, "COMPACTION_LEVEL", &level.to_string());
    }
    push_opt_time(out, "COMPACTED", issue.compacted_at);
    push_opt_str(
        out,
        "COMPACTED_AT_COMMIT",
        issue.compacted_at_commit.as_deref(),
    );
    if let Some(size) = issue.original_size {
        push_aligned(out, "ORIGINAL_SIZE", &size.to_string());
    }
    push_opt_str(out, "SENDER", issue.sender.as_deref());
    if issue.ephemeral {
        push_aligned(out, "EPHEMERAL", "true");
    }
    // `Status::Pinned` forces the flag on read (`parse_heading_to_issue`), so
    // the write side forces it too. Gating this on the flag alone let
    // `--status pinned` with `pinned: false` — what `obr create` stores —
    // write a heading with no `:PINNED:`, gain the flag on import, and grow
    // the line on the second flush. The model state converges on the first
    // flush instead; `stats` already reads `pinned || status == Pinned`, so
    // no consumer sees a change.
    if issue.pinned || issue.status == Status::Pinned {
        push_aligned(out, "PINNED", "true");
    }
    if issue.is_template {
        push_aligned(out, "IS_TEMPLATE", "true");
    }
    // Drawer properties another tool owns, put back exactly as they were read
    // so a flush is not a deletion. Emitted last because obr's own properties
    // are the authoritative ones and a duplicate would win on the next parse;
    // the reserved-key filter in `harvest_preserved_properties` is what makes
    // "duplicate" impossible, and this ordering is the second line of defence.
    if let Some(preserved) = style.preserved.get(&issue.id) {
        for (key, value) in preserved {
            push_aligned(out, key, &sanitize_property_value(value));
        }
    }
    out.push_str(":END:\n");
    Ok(())
}

/// The canonical on-disk form of a stored free-text value: CRLF collapsed to
/// LF, trailing newlines removed, and leading blank (whitespace-only) lines
/// removed.
///
/// This is the single canonicalization in the emitter, and it is idempotent
/// by construction (`canon(canon(x)) == canon(x)`), which is the property the
/// fixpoint depends on. Every line-oriented transform downstream
/// ([`sanitize_org_text`], [`escape_block_lines`]) is then the identity on
/// line structure; the previous `lines().join("\n")` canonicalization was not
/// idempotent — it removed one trailing newline per pass — so a body ending
/// in two or more newlines took the example-block fallback in generation one
/// and unwrapped in generation two.
///
/// What it removes is what the Org surface cannot carry, which is why these
/// normalize once, on the first flush, in the same accepted class as line
/// endings:
///
/// - Trailing newlines: every emission path is `lines()`-based, so they
///   never reach the file at all.
/// - Leading blank lines: the blank line after the drawer is the body's own
///   delimiter, and the Org reader consumes leading blanks inside an example
///   block too, so a body that starts with one comes back without it. (A
///   *trailing* blank line is representable — the example-block fallback
///   carries it — and is deliberately kept.)
/// - A wholly blank body: it parses to no elements and reads back as absent.
fn canonical_body_text(text: &str) -> String {
    let lf = text.replace("\r\n", "\n");
    let body = lf.trim_end_matches('\n');
    let lines: Vec<&str> = body.split('\n').collect();
    let first_content = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    lines[first_content..].join("\n")
}

/// The canonical text to emit for a free-text field, or `None` when the field
/// holds nothing the surface can carry.
///
/// A value that canonicalizes to nothing (`Some("\n")`, `Some(" ")`,
/// `Some("")`) reads back as absent, so emitting it would write blank lines
/// that vanish on the next flush. Skipping it makes the first write the
/// fixpoint, exactly as [`push_opt_str`] does for drawer properties.
fn body_to_emit(value: Option<&str>) -> Option<String> {
    let canon = canonical_body_text(value?);
    (!canon.is_empty()).then_some(canon)
}

/// True for values a single-line drawer property cannot carry.
fn is_multiline(value: &str) -> bool {
    value.contains('\n') || value.contains('\r')
}

/// True when a close/delete reason has to travel as a text child.
///
/// Decided on the canonical form, which is what will actually be stored: a
/// reason of `"done\n"` is multi-line raw but single-line canonical, and
/// classifying it raw put it in a `** Close Reason` child on the first write
/// and in the drawer on the second.
fn reason_needs_text_child(value: Option<&str>) -> bool {
    value.is_some_and(|v| is_multiline(&canonical_body_text(v)))
}

fn push_opt_str(out: &mut String, key: &str, value: Option<&str>) {
    // A property whose value trims to nothing reads back as absent (the
    // drawer parser trims values and the reader maps empty to None), so
    // emitting it would churn away on the next write. Skipping it makes the
    // first write the fixpoint; the round-trip result is identical.
    //
    // The emitted value is trimmed for the same reason: the drawer parser
    // trims what it reads, so `" alice "` would come back as `"alice"` and
    // the second write would differ from the first. A drawer property cannot
    // carry leading or trailing whitespace; padding normalizes once, on the
    // first flush.
    if let Some(v) = value
        && !v.trim().is_empty()
    {
        push_aligned(out, key, &sanitize_property_value(v.trim()));
    }
}

fn push_opt_time(out: &mut String, key: &str, value: Option<DateTime<Utc>>) {
    if let Some(t) = value {
        push_aligned(out, key, &format_org_timestamp(t));
    }
}

/// The column Org itself puts a property value in: `org-property-format` is
/// `"%-10s %s"`, so the `:KEY:` token is padded to ten characters and one
/// space separates it from the value.
///
/// EVERY drawer property is emitted through [`push_aligned`], so each one
/// lands where `org-set-property` would put it and editing it in Emacs
/// rewrites the identical bytes. That invariant is grep-checkable: no
/// `:KEY: value` string is formatted anywhere else in this module.
///
/// A drawer mixing `:LABELS: []` (column 9) with `:ID:       x` (column 11)
/// is exactly what Org's own `org--align-node-property` rewrites the moment
/// the entry is re-indented, so single-spaced keys are a standing churn loop
/// against Emacs. Widening is invisible to the reader, which trims property
/// values before matching, so files written either way import identically.
const ORG_PROPERTY_KEY_WIDTH: usize = 10;

fn push_aligned(out: &mut String, key: &str, value: &str) {
    out.push_str(&format!(
        "{:<width$} {value}\n",
        format!(":{key}:"),
        width = ORG_PROPERTY_KEY_WIDTH
    ));
}

/// Render an instant as an Org inactive timestamp in the machine's local
/// zone: `[2026-08-09 Sun 12:31]`.
///
/// The weekday is computed by chrono from the date (`%a`, which is
/// locale-independent English), never carried, so a hand-edit that names the
/// wrong day is corrected by the next flush rather than preserved.
///
/// Local, not UTC, by design: every Org reader — `org-sort-entries`,
/// `org-entry-get`, column view, a human — takes the text as wall-clock time
/// in its own zone, and Org has no place to put an offset. The costs —
/// minute precision, and a surface whose bytes differ between machines in
/// different zones — are recorded in `docs/RESIDUALS.md`.
fn format_org_timestamp(t: DateTime<Utc>) -> String {
    format_org_timestamp_in(&Local, t)
}

/// [`format_org_timestamp`] against an explicit zone, so the rendering rule
/// can be tested against a zone with known DST transitions instead of
/// whatever the test host happens to be set to.
fn format_org_timestamp_in<Tz: TimeZone>(zone: &Tz, t: DateTime<Utc>) -> String {
    format!(
        "[{}]",
        t.with_timezone(zone)
            .naive_local()
            .format("%Y-%m-%d %a %H:%M")
    )
}

/// Upper bound on the DST gap [`local_to_instant`] will walk across. One day
/// is far beyond any transition ever legislated (the largest on record is two
/// hours), and bounding it keeps a pathological zone from spinning.
const MAX_DST_GAP_MINUTES: i64 = 24 * 60;

/// The instant a local wall-clock reading denotes.
///
/// An Org timestamp carries no zone, so this is not a total function and both
/// failures resolve deterministically:
///
/// - **Fall-back.** One local hour occurs twice a year. The EARLIER instant
///   is chosen, always. The choice is stable under re-rendering — both
///   candidates render to the same local text — so the surface remains a
///   fixpoint even in the case where the stored instant shifts by the offset
///   delta on the way in.
/// - **Spring-forward.** One local hour does not occur at all. obr renders
///   from real instants and so cannot write such a reading; only a hand-edit
///   can. It resolves to the first representable instant at or after it, i.e.
///   the transition itself, and the next flush rewrites the property to that
///   instant's own reading.
fn local_to_instant<Tz: TimeZone>(zone: &Tz, naive: NaiveDateTime) -> Option<DateTime<Utc>> {
    match zone.from_local_datetime(&naive) {
        LocalResult::Single(t) => Some(t.with_timezone(&Utc)),
        // `LocalResult::Ambiguous` is ordered by UTC OFFSET, not by instant:
        // chrono yields the larger offset first, so on a fall-back its `.0` /
        // `.earliest()` is the LATER instant. Naming either variant "earliest"
        // and taking it would silently store the wrong hour — it did, until a
        // verification pass measured chrono instead of trusting the name.
        // Compare the instants themselves.
        LocalResult::Ambiguous(a, b) => {
            let (a, b) = (a.with_timezone(&Utc), b.with_timezone(&Utc));
            Some(a.min(b))
        }
        LocalResult::None => {
            let mut probe = naive;
            for _ in 0..MAX_DST_GAP_MINUTES {
                probe = probe.checked_add_signed(TimeDelta::minutes(1))?;
                if let Some(t) = zone.from_local_datetime(&probe).earliest() {
                    return Some(t.with_timezone(&Utc));
                }
            }
            None
        }
    }
}

/// Parse a timestamp property value, whichever spelling of the key carried
/// it.
///
/// Two syntaxes are accepted, in this order:
///
/// 1. An Org timestamp, inactive `[…]` or active `<…>`, read in the local
///    zone. This is what obr writes (always inactive).
/// 2. RFC3339. Every `*_AT` property carried this before the Org spellings
///    existed, so a `PLAN.org` written by an older build imports unchanged,
///    and it is what a machine-generated hand-edit is likeliest to produce.
///
/// One function serves both key spellings deliberately: a legacy file, a
/// current file, and a file somebody hand-merged from the two all import.
///
/// # Errors
///
/// Returns a validation error naming `field` when the value is neither.
fn parse_timestamp_value(value: &str, field: &str) -> Result<DateTime<Utc>> {
    parse_org_timestamp_in(&Local, value)
        .or_else(|| {
            DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|t| t.with_timezone(&Utc))
        })
        .ok_or_else(|| BeadsError::Validation {
            field: field.to_string(),
            reason: format!(
                "expected an Org timestamp like [2026-08-09 Sun 12:31] or an RFC3339 \
                 instant, got {value:?}"
            ),
        })
}

/// Parse an Org timestamp in `zone`, or `None` when the text is not one, so
/// the caller can fall back to RFC3339.
///
/// The accepted shape is exactly what obr writes plus the slack a hand-edit
/// needs: a date, an optional weekday word, and an optional `HH:MM` or
/// `HH:MM:SS` clock. Anything further inside the brackets — a repeater or
/// warning cookie, a timestamp range — is refused rather than ignored,
/// because obr has nowhere to store it and the next flush would drop it
/// silently. Failing the import is the same choice the module already makes
/// for a broken JSON section.
fn parse_org_timestamp_in<Tz: TimeZone>(zone: &Tz, value: &str) -> Option<DateTime<Utc>> {
    let body = value
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .or_else(|| {
            value
                .strip_prefix('<')
                .and_then(|inner| inner.strip_suffix('>'))
        })?;

    let mut fields = body.split_whitespace();
    let date = NaiveDate::parse_from_str(fields.next()?, "%Y-%m-%d").ok()?;

    let mut next = fields.next();
    // The weekday is decoration; it is recomputed on every write, so a
    // hand-edit naming the wrong day still imports.
    if next.is_some_and(|field| field.chars().all(char::is_alphabetic)) {
        next = fields.next();
    }
    let time = match next {
        None => NaiveTime::MIN,
        Some(field) => NaiveTime::parse_from_str(field, "%H:%M")
            .or_else(|_| NaiveTime::parse_from_str(field, "%H:%M:%S"))
            .ok()?,
    };
    if fields.next().is_some() {
        return None;
    }

    local_to_instant(zone, date.and_time(time))
}

/// True when the drawer already carries `canonical`, the current spelling of
/// a timestamp property.
///
/// Every legacy arm is guarded on this, so a file that somehow holds more
/// than one spelling of the same field resolves the same way whichever order
/// they appear in: the current spelling wins, exactly as `:LABELS:` wins over
/// heading tags. `closed_at` has two legacy spellings rather than one
/// (`CLOSED`, then `CLOSED_AT`), and they are ranked newest-first by the same
/// rule.
fn carries_property(properties: &[Property], canonical: &str) -> bool {
    properties.iter().any(|prop| prop.key == canonical)
}

/// Push the whitespace between a heading's text and its tag string.
///
/// With no declared column this is the historical fixed four spaces. With one,
/// it is Org's own rule, transcribed from `org--align-tags-here` (org.el):
///
/// ```elisp
/// (new (max (if (>= to-col 0) to-col
///             (- (abs to-col) (string-width (match-string 1))))
///           ;; Introduce at least one space after the heading or the stars.
///           (save-excursion (goto-char blank-start) (1+ (current-column)))))
/// ```
///
/// Three details are load-bearing. Org measures from the last non-blank
/// character, so trailing space left by an empty title is trimmed first.
/// Org uses `string-width`, so the measure is display width, not bytes —
/// tags are filtered to ASCII by `is_org_tag_safe`, but a title is not. And
/// the `max` clamps to one space rather than letting a long heading pull the
/// tags left, which is why a heading already past the column simply gets a
/// single space instead of being reflowed.
fn push_tag_gap(out: &mut String, tags: &str, style: &OrgStyle) {
    let Some(column) = style.tags_column else {
        out.push_str("    ");
        return;
    };
    let head = out.trim_end_matches([' ', '\t']);
    let head_width = UnicodeWidthStr::width(head);
    out.truncate(head.len());
    let target = if column >= 0 {
        column.unsigned_abs() as usize
    } else {
        (column.unsigned_abs() as usize).saturating_sub(UnicodeWidthStr::width(tags))
    };
    let pad = target.saturating_sub(head_width).max(1);
    out.extend(std::iter::repeat_n(' ', pad));
}

/// Emit a level-2 section heading and the blank line that separates it from
/// its content.
///
/// The blank line is not cosmetic. Without it, a body whose first line is a
/// planning keyword (`SCHEDULED:`, `DEADLINE:`) or a drawer opener
/// (`:PROPERTIES:`) is absorbed by the *heading* during parsing: org2jsonl
/// attaches it to the child's `planning` / `properties`, neither of which the
/// reader consumes (only `body` and `body_spacing`, see `parse_children`), so
/// the line is silently dropped on import and the surface stops being a
/// fixpoint. The stability probe cannot catch it, because the probe renders
/// the candidate body in a context that already has the blank line
/// (`body_reconstructs_exactly`) — so the probe says "safe" and the emitter
/// then writes the one shape where it is not.
///
/// Separating a heading from its body also matches what this module already
/// does after `:END:` for the description.
fn push_child_heading(out: &mut String, heading: &str) {
    out.push('\n');
    out.push_str(&format!("** {heading}\n\n"));
}

fn push_text_child(out: &mut String, heading: &str, value: Option<&str>) {
    if let Some(canon) = body_to_emit(value) {
        push_child_heading(out, heading);
        push_stable_body(out, &canon);
        out.push('\n');
    }
}

/// Emit a free-text body in whichever representation round-trips exactly.
///
/// Preferred: sanitized Org text, so intentional structure (lists, tables,
/// src blocks) stays native and human-editable (docs/DECISIONS.md U2). But some
/// texts — typically pasted code under a real list bullet — reconstruct with
/// drifting indentation in the org2jsonl writer (+2 spaces per import/flush
/// cycle, observed unbounded on the real tracker corpus). For any body whose
/// parse-and-reconstruct is not byte-exact, fall back to a verbatim
/// `#+begin_example` block: lossless, a fixpoint from the first write, and
/// idiomatic Org for code-like content. The reader unwraps a body that is
/// exactly one example block.
///
/// `canon` must already be [`canonical_body_text`]: this function does no
/// canonicalization of its own, so that the stability probe judges exactly
/// the bytes that will be written and re-read.
fn push_stable_body(out: &mut String, canon: &str) {
    debug_assert_eq!(
        canon,
        canonical_body_text(canon),
        "push_stable_body requires a canonical body"
    );
    let sanitized = sanitize_org_text(canon);
    if body_reconstructs_exactly(&sanitized, canon) {
        out.push_str(&sanitized);
    } else {
        out.push_str("#+begin_example\n");
        out.push_str(&escape_block_lines(canon));
        out.push_str("\n#+end_example");
    }
}

/// True when `sanitized`, emitted as a heading body, parses and reconstructs
/// back to exactly `original`. Deterministic per input, so emission stays
/// pure and thread-shardable.
fn body_reconstructs_exactly(sanitized: &str, original: &str) -> bool {
    let probe = format!("* TODO probe\n:PROPERTIES:\n:ID:       probe\n:END:\n\n{sanitized}\n");
    let entries = org2jsonl::org_to_json::org_to_entries_with_keywords(
        &probe,
        ORG_TODO_KEYWORDS,
        ORG_DONE_KEYWORDS,
    );
    for entry in entries {
        if let EntryContent::Heading(heading) = entry.content
            && heading.level == 1
        {
            let reconstructed = reconstruct_body(&heading.body, &heading.body_spacing);
            return unsanitize_org_text(&reconstructed) == original;
        }
    }
    false
}

fn push_json_child(out: &mut String, heading: &str, json: &str) {
    push_child_heading(out, heading);
    out.push_str("#+begin_src json\n");
    out.push_str(json);
    out.push('\n');
    out.push_str("#+end_src\n");
}

/// True when `label` consists solely of legal Org tag characters and may be
/// rendered as a heading tag.
fn is_org_tag_safe(label: &str) -> bool {
    !label.is_empty()
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '#' | '%'))
}

/// True when a (sanitized) title could be re-parsed by Org with a trailing
/// tag group, truncating or merging the title into the tags. Deliberately
/// broad — any colon-terminated title is at risk when tags follow it on the
/// heading line — since a false positive only costs one redundant `:TITLE:`
/// property.
fn title_needs_property_override(title: &str) -> bool {
    title.ends_with(':')
}

/// Sanitize a property value for safe inclusion in an Org property drawer:
/// flatten newlines to spaces so the value cannot open a second line.
///
/// Line flattening is the whole defense. A drawer ends at a line that *is*
/// `:END:`, and every property this module writes is `:KEY: value`, so a
/// value that stays on one line can never produce a terminator no matter what
/// it contains. `:END:` was previously also rewritten to `:END `, which was
/// not a defense (mid-line `:END:` parses fine — see
/// `end_token_in_values_is_preserved_and_roundtrips`) but was
/// destructive: it applied to the authoritative `:LABELS:` JSON, and
/// `x:END:y` is a valid label under `LabelValidator`, so the label silently
/// became `x:END y` with no inverse anywhere on the read path.
fn sanitize_property_value(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}

/// Escape free text for Org-mode bodies: any line starting with `*` (a
/// heading), `,` (an escape), or a git conflict marker gains one leading
/// comma.
///
/// The conflict-marker escapes exist because the sync safety layer's
/// never-bypassable marker scan is line-oriented: JSONL keeps quoted markers
/// inside escaped strings, but Org writes descriptions as real multi-line
/// text, so an issue that merely *quotes* `<<<<<<<`/`=======`/`>>>>>>>`
/// (the fork's own tracker has several) would otherwise make every future
/// import refuse the file as conflicted. Escaped on write, markers from a
/// real git conflict still land unescaped and the scan stays authoritative.
///
/// Exact inverse of [`unsanitize_org_text`] (which strips one leading comma
/// from any line that has one).
///
/// Line structure is preserved, not normalized: the input is already
/// [`canonical_body_text`], so the `lines()`/`join` pass is the identity on
/// it. Canonicalizing here as well is what made the emitter strip one newline
/// per generation.
fn sanitize_org_text(text: &str) -> String {
    debug_assert_eq!(
        text,
        canonical_body_text(text),
        "sanitize_org_text requires a canonical body"
    );
    text.lines()
        .map(|line| {
            if line.starts_with('*')
                || line.starts_with(',')
                || line.starts_with("<<<<<<<")
                || line.starts_with("=======")
                || line.starts_with(">>>>>>>")
            {
                format!(",{line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Exact inverse of [`sanitize_org_text`]: strip one leading comma from any
/// line that has one.
fn unsanitize_org_text(text: &str) -> String {
    text.lines()
        .map(|line| line.strip_prefix(',').unwrap_or(line).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Escape arbitrary stored text for inclusion inside a `#+begin_src` or
/// `#+begin_example` block: one leading comma for any line whose first
/// non-blank characters are `*` or `#+`, or that starts with `,` or a
/// column-zero git conflict marker. Inverse: [`unescape_block_lines`].
///
/// Valid JSON cannot produce such lines, so for well-formed `agent_context`
/// documents this is the identity. For the verbatim description fallback the
/// marker escapes are load-bearing: the sync safety layer's never-bypassable
/// conflict scan reads raw lines before any Org parsing, so a quoted
/// `<<<<<<<`/`=======`/`>>>>>>>` written verbatim inside a block would make
/// every future import refuse the file (found on the real 549-issue corpus,
/// same rationale as [`sanitize_org_text`]).
///
/// Like [`sanitize_org_text`], this preserves line structure rather than
/// normalizing it: its input is already [`canonical_body_text`].
fn escape_block_lines(text: &str) -> String {
    debug_assert_eq!(
        text,
        canonical_body_text(text),
        "escape_block_lines requires a canonical body"
    );
    text.lines()
        .map(|line| {
            let t = line.trim_start();
            if t.starts_with('*')
                || t.starts_with("#+")
                || line.starts_with(',')
                || line.starts_with("<<<<<<<")
                || line.starts_with("=======")
                || line.starts_with(">>>>>>>")
            {
                format!(",{line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Exact inverse of [`escape_block_lines`].
fn unescape_block_lines(text: &str) -> String {
    text.lines()
        .map(|line| line.strip_prefix(',').unwrap_or(line).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Convert an issue's status to its Org keyword.
///
/// # Errors
///
/// `Status::Custom` has no Org keyword: emitting one would be absorbed into
/// the title on re-parse with the status silently reset to `Open`, so it is
/// refused with the issue id named.
fn status_to_keyword(issue: &Issue) -> Result<String> {
    Ok(match &issue.status {
        Status::Open => "TODO".to_string(),
        Status::InProgress => "DOING".to_string(),
        Status::Blocked => "WAIT".to_string(),
        Status::Deferred => "DEFER".to_string(),
        Status::Draft => "DRAFT".to_string(),
        Status::Closed => "DONE".to_string(),
        Status::Tombstone => "CANCELED".to_string(),
        Status::Pinned => "NOTE".to_string(),
        Status::Custom(s) => {
            return Err(BeadsError::Validation {
                field: "status".to_string(),
                reason: format!(
                    "issue {} has custom status {s:?}, which cannot be represented as an \
                     Org TODO keyword; set a standard status or export to a .jsonl path",
                    issue.id
                ),
            });
        }
    })
}

/// Convert an Org keyword to an issue status: the exact inverse of
/// [`status_to_keyword`] over [`ORG_TODO_KEYWORDS`] + [`ORG_DONE_KEYWORDS`].
///
/// Those two tables are what every parse entry point passes to the tokenizer,
/// so a heading can only ever carry a keyword from them; the unknown arm is
/// defensive. It used to fall through to `Status::from_str`, which mints
/// `Status::Custom` — a status [`status_to_keyword`] then refuses — and to
/// accept raw internal status names (`OPEN`, `IN_PROGRESS`, …) that the
/// tokenizer never yields, so a file written with them parsed as `Open` with
/// the keyword absorbed into the title. Failing loudly is the only behavior
/// that cannot lose a status silently.
///
/// # Errors
///
/// Names the keyword when it is outside the recognized set.
fn keyword_to_status(keyword: &str) -> Result<Status> {
    match keyword.to_uppercase().as_str() {
        "TODO" => Ok(Status::Open),
        "DOING" => Ok(Status::InProgress),
        "WAIT" => Ok(Status::Blocked),
        "DEFER" => Ok(Status::Deferred),
        "DRAFT" => Ok(Status::Draft),
        "DONE" => Ok(Status::Closed),
        "CANCELED" => Ok(Status::Tombstone),
        "NOTE" => Ok(Status::Pinned),
        other => Err(BeadsError::Validation {
            field: "status".to_string(),
            reason: format!(
                "unrecognized Org TODO keyword {other:?}; expected one of {} or {}",
                ORG_TODO_KEYWORDS.join(", "),
                ORG_DONE_KEYWORDS.join(", ")
            ),
        }),
    }
}

/// Convert an issue's priority to an Org priority cookie. Out-of-range
/// priorities collapse to `[#C]` (MEDIUM), with a warning naming the issue.
fn priority_to_org(issue: &Issue) -> &'static str {
    match issue.priority.0 {
        0 => "[#A]",
        1 => "[#B]",
        2 => "[#C]",
        3 => "[#D]",
        4 => "[#E]",
        other => {
            tracing::warn!(
                issue_id = %issue.id,
                priority = other,
                "out-of-range priority collapsed to [#C] (MEDIUM) in Org export"
            );
            "[#C]"
        }
    }
}

/// Convert an Org priority cookie letter to an issue priority.
fn org_to_priority(org_priority: Option<&str>) -> Priority {
    match org_priority {
        Some("A") => Priority::CRITICAL,
        Some("B") => Priority::HIGH,
        Some("D") => Priority::LOW,
        Some("E") => Priority::BACKLOG,
        _ => Priority::MEDIUM,
    }
}

/// Parse Org-mode text into issues.
///
/// Only level-1 headings become issues; the zeroth section (file header) and
/// deeper structure hang off their headings.
///
/// # Errors
///
/// One malformed heading (missing `:ID:`, bad timestamp, bad embedded JSON,
/// a reserved JSON section without its `json` src block, an unrecognized
/// TODO keyword) aborts the whole parse; the error names the heading's
/// ordinal position and title where possible. A level-2 section obr does not
/// model is not an error — it warns (see [`UNRECOGNIZED_ORG_SECTION_CODE`])
/// and is dropped.
pub fn org_text_to_issues(org_text: &str) -> Result<Vec<Issue>> {
    let entries = org2jsonl::org_to_json::org_to_entries_with_keywords(
        org_text,
        ORG_TODO_KEYWORDS,
        ORG_DONE_KEYWORDS,
    );
    let mut issues = Vec::new();
    let mut ordinal = 0usize;

    for entry in entries {
        if let EntryContent::Heading(heading) = entry.content
            && heading.level == 1
        {
            ordinal += 1;
            let issue = parse_heading_to_issue(&heading).map_err(|err| {
                annotate_heading_error(err, ordinal, &extract_title_text(&heading.title))
            })?;
            issues.push(issue);
        }
    }

    Ok(issues)
}

/// Scan an Org document for its level-1 heading `:ID:` properties without
/// constructing full issues. Returns `(ordinal, id)` pairs in file order.
///
/// This is the light-weight counterpart of [`org_text_to_issues`] used by
/// export verification and analysis; it applies the same "only `:ID:` is
/// required" contract and the same ordinal-based diagnostics.
///
/// # Errors
///
/// Fails when a level-1 heading has no (or an empty) `:ID:` property.
pub fn org_heading_ids(org_text: &str) -> Result<Vec<(usize, String)>> {
    let entries = org2jsonl::org_to_json::org_to_entries_with_keywords(
        org_text,
        ORG_TODO_KEYWORDS,
        ORG_DONE_KEYWORDS,
    );
    let mut ids = Vec::new();
    let mut ordinal = 0usize;

    for entry in entries {
        if let EntryContent::Heading(heading) = entry.content
            && heading.level == 1
        {
            ordinal += 1;
            let id = heading
                .properties
                .iter()
                .find(|prop| prop.key == "ID")
                .map(|prop| prop.value.trim().to_string())
                .filter(|id| !id.is_empty())
                .ok_or_else(|| BeadsError::Validation {
                    field: "id".to_string(),
                    reason: format!(
                        "heading #{ordinal} ({:?}): missing required :ID: property",
                        extract_title_text(&heading.title)
                    ),
                })?;
            ids.push((ordinal, id));
        }
    }

    Ok(ids)
}

/// Issues parsed from level-1 headings, each with its heading ordinal.
pub type ParsedOrgIssues = Vec<(usize, Issue)>;
/// Per-heading parse failures: `(ordinal, message)`.
pub type OrgParseFailures = Vec<(usize, String)>;

/// Parse every level-1 heading, collecting per-heading failures instead of
/// aborting on the first. Returns parsed issues as `(ordinal, issue)` and
/// failures as `(ordinal, message)` — the shape validation summaries need.
#[must_use]
pub fn parse_issues_collecting_failures(org_text: &str) -> (ParsedOrgIssues, OrgParseFailures) {
    let entries = org2jsonl::org_to_json::org_to_entries_with_keywords(
        org_text,
        ORG_TODO_KEYWORDS,
        ORG_DONE_KEYWORDS,
    );
    let mut issues = Vec::new();
    let mut failures = Vec::new();
    let mut ordinal = 0usize;

    for entry in entries {
        if let EntryContent::Heading(heading) = entry.content
            && heading.level == 1
        {
            ordinal += 1;
            match parse_heading_to_issue(&heading) {
                Ok(issue) => issues.push((ordinal, issue)),
                Err(err) => failures.push((ordinal, err.to_string())),
            }
        }
    }

    (issues, failures)
}

/// Wrap a per-heading parse error with the heading's position and title so a
/// malformed record in a large file is findable (the parser itself carries no
/// line numbers).
fn annotate_heading_error(err: BeadsError, ordinal: usize, title: &str) -> BeadsError {
    match err {
        BeadsError::Validation { field, reason } => BeadsError::Validation {
            field,
            reason: format!("heading #{ordinal} ({title:?}): {reason}"),
        },
        other => other,
    }
}

/// Convert an Org heading to an Issue.
fn parse_heading_to_issue(heading: &Heading) -> Result<Issue> {
    let mut issue = Issue::default();

    if let Some(ref keyword) = heading.keyword {
        issue.status = keyword_to_status(keyword)?;
        if issue.status == Status::Pinned {
            issue.pinned = true;
        }
    }

    issue.priority = org_to_priority(heading.priority.as_deref());
    // Org heading titles are whitespace-delimited from cookie and tags; the
    // parser hands back the raw span (including the tag separator spaces), so
    // trim. Titles with leading/trailing whitespace are not representable.
    issue.title = extract_title_text(&heading.title).trim().to_string();

    // Provisional labels from heading tags; an authoritative :LABELS:
    // property replaces them below.
    issue.labels.clone_from(&heading.tags);
    issue.labels.sort();

    parse_properties(&heading.properties, &mut issue)?;

    if let Some(text) = extract_body_as_text(&heading.body, &heading.body_spacing) {
        issue.description = Some(text);
    }

    for child in &heading.children {
        if child.level != 2 {
            continue;
        }
        let child_title = extract_title_text(&child.title);
        match child_title.as_str() {
            "Design" => {
                if let Some(text) = extract_body_as_text(&child.body, &child.body_spacing) {
                    issue.design = Some(text);
                }
            }
            "Acceptance Criteria" => {
                if let Some(text) = extract_body_as_text(&child.body, &child.body_spacing) {
                    issue.acceptance_criteria = Some(text);
                }
            }
            "Notes" => {
                if let Some(text) = extract_body_as_text(&child.body, &child.body_spacing) {
                    issue.notes = Some(text);
                }
            }
            // Multi-line reasons; the child overrides any (flattened)
            // drawer property parsed earlier.
            "Close Reason" => {
                if let Some(text) = extract_body_as_text(&child.body, &child.body_spacing) {
                    issue.close_reason = Some(text);
                }
            }
            "Delete Reason" => {
                if let Some(text) = extract_body_as_text(&child.body, &child.body_spacing) {
                    issue.delete_reason = Some(text);
                }
            }
            "Agent Context" => {
                let json = require_json_block(&child.body, &issue.id, "Agent Context")?;
                issue.agent_context = Some(unescape_block_lines(&json));
            }
            "Dependencies" => {
                let json = require_json_block(&child.body, &issue.id, "Dependencies")?;
                issue.dependencies = serde_json::from_str(&json).map_err(BeadsError::Json)?;
            }
            "Comments" => {
                let json = require_json_block(&child.body, &issue.id, "Comments")?;
                issue.comments = serde_json::from_str(&json).map_err(BeadsError::Json)?;
            }
            other => {
                debug_assert!(
                    !ORG_CHILD_SECTIONS.contains(&other),
                    "recognized section {other:?} reached the unknown-section arm"
                );
                warn_unrecognized_section(&issue.id, other);
            }
        }
    }

    Ok(issue)
}

/// Extract plain text from inline title content, preserving markup
/// delimiters for round-trip fidelity.
fn extract_title_text(contents: &[InlineContent]) -> String {
    let mut out = String::new();
    for item in contents {
        flatten_inline_to_text(item, &mut out);
    }
    out
}

/// Recursively flatten inline content to its source-text form. Covers every
/// `InlineContent` variant; unknown future variants cannot exist (the enum is
/// exhaustively matched so a new upstream variant is a compile error here,
/// not a silent drop).
#[allow(clippy::too_many_lines)]
fn flatten_inline_to_text(item: &InlineContent, out: &mut String) {
    match item {
        InlineContent::Text { value }
        | InlineContent::LatexFragment { value }
        | InlineContent::Timestamp { value }
        | InlineContent::InlineBabel { value }
        | InlineContent::StatisticsCookie { value } => out.push_str(value),
        InlineContent::Subscript {
            contents,
            use_braces,
        } => {
            out.push('_');
            if *use_braces {
                out.push('{');
            }
            for child in contents {
                flatten_inline_to_text(child, out);
            }
            if *use_braces {
                out.push('}');
            }
        }
        InlineContent::Superscript {
            contents,
            use_braces,
        } => {
            out.push('^');
            if *use_braces {
                out.push('{');
            }
            for child in contents {
                flatten_inline_to_text(child, out);
            }
            if *use_braces {
                out.push('}');
            }
        }
        InlineContent::Bold { contents } => {
            out.push('*');
            for child in contents {
                flatten_inline_to_text(child, out);
            }
            out.push('*');
        }
        InlineContent::Italic { contents } => {
            out.push('/');
            for child in contents {
                flatten_inline_to_text(child, out);
            }
            out.push('/');
        }
        InlineContent::Underline { contents } => {
            out.push('_');
            for child in contents {
                flatten_inline_to_text(child, out);
            }
            out.push('_');
        }
        InlineContent::StrikeThrough { contents } => {
            out.push('+');
            for child in contents {
                flatten_inline_to_text(child, out);
            }
            out.push('+');
        }
        InlineContent::Code { value } => {
            out.push('~');
            out.push_str(value);
            out.push('~');
        }
        InlineContent::Verbatim { value } => {
            out.push('=');
            out.push_str(value);
            out.push('=');
        }
        InlineContent::LineBreak => out.push('\n'),
        InlineContent::Entity { name } => {
            out.push('\\');
            out.push_str(name);
            out.push_str("{}");
        }
        InlineContent::Link { description, path } => {
            out.push_str("[[");
            out.push_str(path);
            if let Some(desc_contents) = description {
                out.push_str("][");
                for child in desc_contents {
                    flatten_inline_to_text(child, out);
                }
            }
            out.push_str("]]");
        }
        InlineContent::FootnoteReference { label, definition } => {
            out.push_str("[fn:");
            if let Some(l) = label {
                out.push_str(l);
            }
            if let Some(def) = definition {
                out.push(':');
                for child in def {
                    flatten_inline_to_text(child, out);
                }
            }
            out.push(']');
        }
        InlineContent::ExportSnippet { backend, value } => {
            out.push_str("@@");
            out.push_str(backend);
            out.push(':');
            out.push_str(value);
            out.push_str("@@");
        }
        InlineContent::InlineSrc { language, value } => {
            out.push_str("src_");
            out.push_str(language);
            out.push('{');
            out.push_str(value);
            out.push('}');
        }
        InlineContent::Macro { value } => {
            out.push_str("{{{");
            out.push_str(value);
            out.push_str("}}}");
        }
        InlineContent::Target { value } => {
            out.push_str("<<");
            out.push_str(value);
            out.push_str(">>");
        }
        InlineContent::RadioTarget { value } => {
            out.push_str("<<<");
            out.push_str(value);
            out.push_str(">>>");
        }
    }
}

/// Extract a heading/child body as the issue's stored free text.
///
/// A body that is exactly one `#+begin_example` block is the emitter's
/// stable-fallback representation (see [`push_stable_body`]): unwrap it
/// verbatim. Anything else is reconstructed as Org source text.
fn extract_body_as_text(elements: &[Element], body_spacing: &[bool]) -> Option<String> {
    if let [Element::ExampleBlock { value }] = elements {
        let text = unescape_block_lines(value.trim_end_matches('\n'));
        return if text.is_empty() { None } else { Some(text) };
    }
    let body_text = reconstruct_body(elements, body_spacing);
    if body_text.is_empty() {
        None
    } else {
        Some(unsanitize_org_text(&body_text))
    }
}

/// Reconstruct a heading body back to Org source text, faithfully covering
/// every `Element` variant by delegating to org2jsonl's own writer over a
/// synthetic zeroth section. This is what makes descriptions containing
/// lists, tables, blocks, and drawers survive a round-trip instead of being
/// silently dropped.
fn reconstruct_body(elements: &[Element], body_spacing: &[bool]) -> String {
    if elements.is_empty() {
        return String::new();
    }
    let fixed: Vec<Element> = elements.iter().map(fix_clock_prefix).collect();
    let entry = OrgEntry {
        schema_version: org2jsonl::SCHEMA_VERSION,
        file: None,
        char_begin: None,
        char_end: None,
        line_begin: None,
        line_end: None,
        content: EntryContent::Section {
            elements: fixed,
            body_spacing: body_spacing.to_vec(),
        },
        post_blank: None,
    };
    let text = org2jsonl::json_to_org::entry_to_org(&entry);
    text.trim_end_matches('\n').to_string()
}

/// Work around a known org2jsonl writer defect: the parser stores `CLOCK:`
/// lines with their prefix included, and the writer prepends the prefix
/// again, corrupting the line once per round-trip. Strip the stored prefix
/// before handing elements to the writer. Applied recursively through
/// container elements.
fn fix_clock_prefix(element: &Element) -> Element {
    match element {
        Element::Clock { value } => Element::Clock {
            value: value
                .strip_prefix("CLOCK:")
                .map_or_else(|| value.clone(), |rest| rest.trim_start().to_string()),
        },
        Element::QuoteBlock { elements } => Element::QuoteBlock {
            elements: elements.iter().map(fix_clock_prefix).collect(),
        },
        Element::CenterBlock { elements } => Element::CenterBlock {
            elements: elements.iter().map(fix_clock_prefix).collect(),
        },
        Element::FootnoteDefinition { label, elements } => Element::FootnoteDefinition {
            label: label.clone(),
            elements: elements.iter().map(fix_clock_prefix).collect(),
        },
        Element::DynamicBlock {
            name,
            parameters,
            elements,
        } => Element::DynamicBlock {
            name: name.clone(),
            parameters: parameters.clone(),
            elements: elements.iter().map(fix_clock_prefix).collect(),
        },
        Element::PlainList { kind, items } => Element::PlainList {
            kind: kind.clone(),
            items: items
                .iter()
                .map(|item| {
                    let mut fixed = item.clone();
                    fixed.contents = item.contents.iter().map(fix_clock_prefix).collect();
                    fixed
                })
                .collect(),
        },
        other => other.clone(),
    }
}

/// The JSON payload of a reserved section, or an error naming the issue.
///
/// `Dependencies`, `Comments`, and `Agent Context` carry machine-serialized
/// JSON in a lowercase `json` src block. A hand-edit that turns the block
/// into prose, upper-cases the fence, or leaves it half-typed used to import
/// as the empty default with no diagnostic — and the next flush wrote the
/// issue back without the section, permanently dropping every edge or
/// comment. Refusing the file is the only outcome that cannot lose data: the
/// user fixes the block or deletes the section.
///
/// # Errors
///
/// Fails when the section carries no lowercase `json` src block.
fn require_json_block(body: &[Element], issue_id: &str, section: &str) -> Result<String> {
    extract_json_from_body(body).ok_or_else(|| BeadsError::Validation {
        field: "org".to_string(),
        reason: format!(
            "issue {issue_id}: the \"** {section}\" section must contain a \
             `#+begin_src json` block (lowercase language, exactly as obr \
             writes it). Restore the block or delete the section — importing \
             it as empty would delete the stored {} on the next flush.",
            section.to_lowercase()
        ),
    })
}

/// Warn that a level-2 section under an issue is not one obr can store.
///
/// `PLAN.org` is the file users are told to edit, so a section obr does not
/// model has to be announced rather than dropped in silence: the next flush
/// rewrites the file from the database and the text is gone. Delivery follows
/// the house warning rules (stderr only, silent under `--quiet`, a JSON
/// envelope under `--json`), and repeats for the same issue and section are
/// suppressed so a document parsed twice in one import warns once.
fn warn_unrecognized_section(issue_id: &str, section: &str) {
    crate::legacy_compat::warn_once_with_code(
        UNRECOGNIZED_ORG_SECTION_CODE,
        &format!("{UNRECOGNIZED_ORG_SECTION_CODE}:{issue_id}:{section}"),
        &format!(
            "issue {issue_id}: the Org section \"** {section}\" is not one obr \
             stores ({}), so it will not survive the next flush. Move the text \
             into Notes or keep it outside the surface file.",
            ORG_CHILD_SECTIONS.join(", ")
        ),
    );
}

/// Extract the value of the first JSON `src` block in a child body.
fn extract_json_from_body(body: &[Element]) -> Option<String> {
    for element in body {
        if let Element::SrcBlock {
            language, value, ..
        } = element
            && language == "json"
        {
            return Some(value.clone());
        }
    }
    None
}

/// Parse the property drawer into the issue.
#[allow(clippy::too_many_lines)]
fn parse_properties(properties: &[Property], issue: &mut Issue) -> Result<()> {
    let mut saw_created_at = false;
    let mut saw_updated_at = false;

    for prop in properties {
        let key = prop.key.as_str();
        let value = prop.value.trim();

        match key {
            "ID" => issue.id = value.to_string(),
            "TITLE" => issue.title = value.to_string(),
            "LABELS" => {
                issue.labels = serde_json::from_str(value).map_err(|e| BeadsError::Validation {
                    field: "labels".to_string(),
                    reason: format!("invalid :LABELS: JSON array: {e}"),
                })?;
                issue.labels.sort();
            }
            "ISSUE_TYPE" => issue.issue_type = IssueType::from_str(value)?,
            "CREATED" => {
                issue.created_at = parse_timestamp_value(value, "created_at")?;
                saw_created_at = true;
            }
            "MODIFIED" => {
                issue.updated_at = parse_timestamp_value(value, "updated_at")?;
                saw_updated_at = true;
            }
            // legacy_compat: surfaces written before the Org timestamps
            // spelled these seven `*_AT` and carried RFC3339. Read, never
            // written, and shadowed by the current spelling.
            "CREATED_AT" if !carries_property(properties, "CREATED") => {
                issue.created_at = parse_timestamp_value(value, "created_at")?;
                saw_created_at = true;
            }
            "UPDATED_AT" if !carries_property(properties, "MODIFIED") => {
                issue.updated_at = parse_timestamp_value(value, "updated_at")?;
                saw_updated_at = true;
            }
            "ASSIGNEE" => issue.assignee = Some(value.to_string()),
            "OWNER" => issue.owner = Some(value.to_string()),
            "ESTIMATED_MINUTES" => {
                issue.estimated_minutes = Some(parse_int(value, "estimated_minutes")?);
            }
            "CREATED_BY" => issue.created_by = Some(value.to_string()),
            "FINISHED" => issue.closed_at = Some(parse_timestamp_value(value, "closed_at")?),
            // legacy_compat: `closed_at` has had three spellings. `CLOSED_AT`
            // (RFC3339) predates the Org timestamps; `CLOSED` was the
            // Org-timestamp spelling of one unreleased build, retired because
            // `CLOSED` is an org-special-property. Both are read, neither is
            // written, and the current spelling shadows them both regardless
            // of drawer order — same rule as the other six.
            "CLOSED" if !carries_property(properties, "FINISHED") => {
                issue.closed_at = Some(parse_timestamp_value(value, "closed_at")?);
            }
            "CLOSED_AT"
                if !carries_property(properties, "FINISHED")
                    && !carries_property(properties, "CLOSED") =>
            {
                issue.closed_at = Some(parse_timestamp_value(value, "closed_at")?);
            }
            "CLOSE_REASON" => issue.close_reason = Some(value.to_string()),
            "CLOSED_BY_SESSION" => issue.closed_by_session = Some(value.to_string()),
            "DUE" => issue.due_at = Some(parse_timestamp_value(value, "due_at")?),
            "DUE_AT" if !carries_property(properties, "DUE") => {
                // legacy_compat: pre-Org-timestamp spelling.
                issue.due_at = Some(parse_timestamp_value(value, "due_at")?);
            }
            "DEFERRED" => issue.defer_until = Some(parse_timestamp_value(value, "defer_until")?),
            "DEFER_UNTIL" if !carries_property(properties, "DEFERRED") => {
                // legacy_compat: pre-Org-timestamp spelling.
                issue.defer_until = Some(parse_timestamp_value(value, "defer_until")?);
            }
            "EXTERNAL_REF" => issue.external_ref = Some(value.to_string()),
            "SOURCE_SYSTEM" => issue.source_system = Some(value.to_string()),
            "SOURCE_REPO" => issue.source_repo = Some(value.to_string()),
            "SOURCE_REPO_PATH" => issue.source_repo_path = Some(value.to_string()),
            "DELETED" => issue.deleted_at = Some(parse_timestamp_value(value, "deleted_at")?),
            "DELETED_AT" if !carries_property(properties, "DELETED") => {
                // legacy_compat: pre-Org-timestamp spelling.
                issue.deleted_at = Some(parse_timestamp_value(value, "deleted_at")?);
            }
            "DELETED_BY" => issue.deleted_by = Some(value.to_string()),
            "DELETE_REASON" => issue.delete_reason = Some(value.to_string()),
            "ORIGINAL_TYPE" => issue.original_type = Some(value.to_string()),
            "COMPACTION_LEVEL" => {
                issue.compaction_level = Some(parse_int(value, "compaction_level")?);
            }
            "COMPACTED" => issue.compacted_at = Some(parse_timestamp_value(value, "compacted_at")?),
            "COMPACTED_AT" if !carries_property(properties, "COMPACTED") => {
                // legacy_compat: pre-Org-timestamp spelling.
                issue.compacted_at = Some(parse_timestamp_value(value, "compacted_at")?);
            }
            "COMPACTED_AT_COMMIT" => issue.compacted_at_commit = Some(value.to_string()),
            "ORIGINAL_SIZE" => issue.original_size = Some(parse_int(value, "original_size")?),
            "SENDER" => issue.sender = Some(value.to_string()),
            "EPHEMERAL" => issue.ephemeral = value.eq_ignore_ascii_case("true"),
            "PINNED" => issue.pinned = value.eq_ignore_ascii_case("true"),
            "IS_TEMPLATE" => issue.is_template = value.eq_ignore_ascii_case("true"),
            _ => {
                // Both schema-version spellings (OBR_SCHEMA_VERSION and the
                // legacy BEADS_SCHEMA_VERSION) are write-only markers, and
                // unknown properties are ignored so newer files degrade
                // gracefully in older builds.
            }
        }
    }

    if issue.id.is_empty() {
        return Err(BeadsError::Validation {
            field: "id".to_string(),
            reason: "missing required :ID: property".to_string(),
        });
    }
    if !saw_created_at || !saw_updated_at {
        // A heading without timestamps inherits Utc::now() from
        // Issue::default() and would win every last-write-wins comparison.
        tracing::warn!(
            issue_id = %issue.id,
            "Org heading is missing :CREATED:/:MODIFIED:; current time \
             substituted, which affects sync conflict resolution"
        );
    }

    Ok(())
}

fn parse_int(value: &str, field: &str) -> Result<i32> {
    value.parse().map_err(|e| BeadsError::Validation {
        field: field.to_string(),
        reason: format!("invalid integer: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Comment, Dependency, DependencyType};
    use chrono::TimeZone;
    use proptest::prelude::*;

    /// A test instant, floored to the minute the Org surface can store.
    ///
    /// An Org timestamp carries `HH:MM`, so a fixture with seconds in it
    /// would make every round-trip assertion below restate the truncation
    /// instead of the property it is actually about.
    /// `org_timestamps_truncate_to_the_minute` is where the truncation itself
    /// is asserted.
    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs - secs.rem_euclid(60), 0).unwrap()
    }

    fn base_issue() -> Issue {
        Issue {
            id: "bd-test".to_string(),
            title: "Test Issue".to_string(),
            created_at: ts(1_700_000_000),
            updated_at: ts(1_700_000_001),
            ..Default::default()
        }
    }

    fn roundtrip(issues: &[Issue]) -> Vec<Issue> {
        let text = issues_to_org_text(issues).expect("emit");
        org_text_to_issues(&text).expect("parse")
    }

    /// Every normalization the Org surface is allowed to apply, and only on
    /// the first write. Written as one function so the round-trip property
    /// can assert against it rather than pre-applying it to the generator.
    ///
    /// Each entry is a value the surface cannot represent: a drawer property
    /// cannot carry edge whitespace (the parser trims), a heading title
    /// cannot either, a body cannot begin or end with blank lines (they are
    /// the body's own delimiters) or consist solely of whitespace (it parses
    /// to no elements), and `Status::Pinned` forces the `pinned` flag.
    fn normalized_for_org(issue: &Issue) -> Issue {
        let normalized_property = |value: Option<&str>| -> Option<String> {
            let trimmed = value?.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        };
        let normalized_reason = |value: Option<&str>| -> Option<String> {
            let canon = canonical_body_text(value?);
            if canon.is_empty() {
                None
            } else if is_multiline(&canon) {
                Some(canon)
            } else {
                Some(canon.trim().to_string())
            }
        };

        let mut expected = issue.clone();
        expected.title = issue.title.trim().to_string();
        expected.pinned = issue.pinned || issue.status == Status::Pinned;
        expected.description = body_to_emit(issue.description.as_deref());
        expected.design = body_to_emit(issue.design.as_deref());
        expected.acceptance_criteria = body_to_emit(issue.acceptance_criteria.as_deref());
        expected.notes = body_to_emit(issue.notes.as_deref());
        expected.agent_context = body_to_emit(issue.agent_context.as_deref());
        expected.close_reason = normalized_reason(issue.close_reason.as_deref());
        expected.delete_reason = normalized_reason(issue.delete_reason.as_deref());
        expected.assignee = normalized_property(issue.assignee.as_deref());
        expected.owner = normalized_property(issue.owner.as_deref());
        expected.external_ref = normalized_property(issue.external_ref.as_deref());
        expected
    }

    /// The product invariant at record scope: emit → parse → emit must be
    /// byte-identical, from the FIRST write. Returns the (single) generation
    /// so callers can assert on its shape too.
    fn assert_first_write_fixpoint(issue: &Issue) -> (String, Issue) {
        let gen1 = issues_to_org_text(std::slice::from_ref(issue)).expect("emit gen1");
        let mut parsed = org_text_to_issues(&gen1).expect("parse gen1");
        let gen2 = issues_to_org_text(&parsed).expect("emit gen2");
        assert_eq!(
            gen1, gen2,
            "the first write must already be a fixpoint\n--- gen1 ---\n{gen1}\n--- gen2 ---\n{gen2}"
        );
        (gen1, parsed.remove(0))
    }

    /// Exhaustively destructure `Issue` so that a new upstream field breaks
    /// this build instead of silently vanishing from the Org representation. When this fails,
    /// decide the new field's Org representation and extend the emitter,
    /// parser, and round-trip tests together.
    #[test]
    #[allow(clippy::no_effect_underscore_binding)]
    fn org_emission_covers_every_issue_field() {
        let Issue {
            id: _id,
            content_hash: _content_hash, // #[serde(skip)] — never on disk in any format
            title: _title,
            description: _description,
            design: _design,
            acceptance_criteria: _acceptance_criteria,
            notes: _notes,
            status: _status,
            priority: _priority,
            issue_type: _issue_type,
            assignee: _assignee,
            owner: _owner,
            estimated_minutes: _estimated_minutes,
            created_at: _created_at,
            created_by: _created_by,
            updated_at: _updated_at,
            closed_at: _closed_at,
            close_reason: _close_reason,
            closed_by_session: _closed_by_session,
            due_at: _due_at,
            defer_until: _defer_until,
            external_ref: _external_ref,
            source_system: _source_system,
            source_repo: _source_repo,
            source_repo_path: _source_repo_path,
            agent_context: _agent_context,
            deleted_at: _deleted_at,
            deleted_by: _deleted_by,
            delete_reason: _delete_reason,
            original_type: _original_type,
            compaction_level: _compaction_level,
            compacted_at: _compacted_at,
            compacted_at_commit: _compacted_at_commit,
            original_size: _original_size,
            sender: _sender,
            ephemeral: _ephemeral,
            pinned: _pinned,
            is_template: _is_template,
            labels: _labels,
            dependencies: _dependencies,
            comments: _comments,
        } = Issue::default();
    }

    #[test]
    fn full_field_roundtrip() {
        let mut issue = base_issue();
        issue.description = Some("A description.\n\nSecond paragraph.".to_string());
        issue.design = Some("Design text".to_string());
        issue.acceptance_criteria = Some("AC text".to_string());
        issue.notes = Some("Notes text".to_string());
        issue.status = Status::InProgress;
        issue.priority = Priority::HIGH;
        issue.issue_type = IssueType::Bug;
        issue.assignee = Some("alice".to_string());
        issue.owner = Some("bob".to_string());
        issue.estimated_minutes = Some(90);
        issue.created_by = Some("carol".to_string());
        issue.closed_at = Some(ts(1_700_000_100));
        issue.close_reason = Some("done".to_string());
        issue.closed_by_session = Some("sess-1".to_string());
        issue.due_at = Some(ts(1_700_000_200));
        issue.defer_until = Some(ts(1_700_000_300));
        issue.external_ref = Some("JIRA-123".to_string());
        issue.source_system = Some("jira".to_string());
        issue.source_repo = Some("myrepo".to_string());
        issue.source_repo_path = Some("/home/user/src/myrepo".to_string());
        issue.agent_context = Some("{\n  \"skills\": [\"rust\"]\n}".to_string());
        issue.deleted_at = None;
        issue.original_type = Some("bug".to_string());
        issue.compaction_level = Some(2);
        issue.compacted_at = Some(ts(1_700_000_400));
        issue.compacted_at_commit = Some("abc123".to_string());
        issue.original_size = Some(1024);
        issue.sender = Some("daemon".to_string());
        issue.ephemeral = true;
        issue.is_template = true;
        issue.labels = vec!["zeta".to_string(), "alpha".to_string()];
        issue.dependencies = vec![Dependency {
            issue_id: "bd-test".to_string(),
            depends_on_id: "bd-other".to_string(),
            dep_type: DependencyType::Blocks,
            created_at: ts(1_700_000_000),
            created_by: None,
            metadata: None,
            thread_id: None,
        }];
        issue.comments = vec![Comment {
            id: 1,
            issue_id: "bd-test".to_string(),
            author: "alice".to_string(),
            body: "hello".to_string(),
            created_at: ts(1_700_000_050),
        }];

        let parsed = roundtrip(std::slice::from_ref(&issue));
        assert_eq!(parsed.len(), 1);
        let got = &parsed[0];

        assert_eq!(got.id, issue.id);
        assert_eq!(got.title, issue.title);
        assert_eq!(got.description, issue.description);
        assert_eq!(got.design, issue.design);
        assert_eq!(got.acceptance_criteria, issue.acceptance_criteria);
        assert_eq!(got.notes, issue.notes);
        assert_eq!(got.status, issue.status);
        assert_eq!(got.priority, issue.priority);
        assert_eq!(got.issue_type, issue.issue_type);
        assert_eq!(got.assignee, issue.assignee);
        assert_eq!(got.owner, issue.owner);
        assert_eq!(got.estimated_minutes, issue.estimated_minutes);
        assert_eq!(got.created_at, issue.created_at);
        assert_eq!(got.created_by, issue.created_by);
        assert_eq!(got.updated_at, issue.updated_at);
        assert_eq!(got.closed_at, issue.closed_at);
        assert_eq!(got.close_reason, issue.close_reason);
        assert_eq!(got.closed_by_session, issue.closed_by_session);
        assert_eq!(got.due_at, issue.due_at);
        assert_eq!(got.defer_until, issue.defer_until);
        assert_eq!(got.external_ref, issue.external_ref);
        assert_eq!(got.source_system, issue.source_system);
        assert_eq!(got.source_repo, issue.source_repo);
        assert_eq!(got.source_repo_path, issue.source_repo_path);
        assert_eq!(got.agent_context, issue.agent_context);
        assert_eq!(got.original_type, issue.original_type);
        assert_eq!(got.compaction_level, issue.compaction_level);
        assert_eq!(got.compacted_at, issue.compacted_at);
        assert_eq!(got.compacted_at_commit, issue.compacted_at_commit);
        assert_eq!(got.original_size, issue.original_size);
        assert_eq!(got.sender, issue.sender);
        assert_eq!(got.ephemeral, issue.ephemeral);
        assert_eq!(got.is_template, issue.is_template);
        assert_eq!(got.labels, vec!["alpha".to_string(), "zeta".to_string()]);
        assert_eq!(got.dependencies, issue.dependencies);
        assert_eq!(got.comments, issue.comments);
    }

    #[test]
    fn draft_status_roundtrips() {
        let mut issue = base_issue();
        issue.status = Status::Draft;
        let text = issues_to_org_text(std::slice::from_ref(&issue)).unwrap();
        assert!(text.contains("* DRAFT [#C] Test Issue"), "got: {text}");
        let parsed = roundtrip(&[issue]);
        assert_eq!(parsed[0].status, Status::Draft);
    }

    #[test]
    fn all_standard_statuses_roundtrip() {
        for status in [
            Status::Open,
            Status::InProgress,
            Status::Blocked,
            Status::Deferred,
            Status::Draft,
            Status::Closed,
            Status::Tombstone,
            Status::Pinned,
        ] {
            let mut issue = base_issue();
            issue.status = status.clone();
            let parsed = roundtrip(&[issue]);
            assert_eq!(parsed[0].status, status);
        }
    }

    #[test]
    fn custom_status_is_refused_with_issue_id() {
        let mut issue = base_issue();
        issue.status = Status::Custom("weird".to_string());
        let err = issues_to_org_text(&[issue]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bd-test"), "error must name the issue: {msg}");
        assert!(msg.contains("weird"), "error must name the status: {msg}");
    }

    #[test]
    fn colon_labels_roundtrip_losslessly() {
        let mut issue = base_issue();
        issue.labels = vec!["provides:parser".to_string(), "plain".to_string()];
        let text = issues_to_org_text(std::slice::from_ref(&issue)).unwrap();
        // Only the tag-safe label is rendered as an Org tag.
        assert!(text.contains(":plain:"), "got: {text}");
        assert!(!text.contains(":provides:parser:"), "got: {text}");
        let parsed = roundtrip(&[issue]);
        assert_eq!(
            parsed[0].labels,
            vec!["plain".to_string(), "provides:parser".to_string()]
        );
    }

    // -----------------------------------------------------------------
    // Org timestamps
    // -----------------------------------------------------------------

    /// A zone shaped like US Pacific for 2026 only: standard UTC-8, daylight
    /// UTC-7, springing forward at 2026-03-08 02:00 local and falling back at
    /// 2026-11-01 02:00 local.
    ///
    /// The DST rules cannot be tested against `chrono::Local`: it reads the
    /// host's zone, which is UTC on CI and something else on a laptop, and
    /// `TZ` cannot be set from a test without `unsafe` and a race against
    /// every other test in the binary. A hand-written zone makes the two
    /// interesting readings — the hour that happens twice and the hour that
    /// never happens — reachable deterministically anywhere.
    #[derive(Clone, Copy, Debug)]
    struct PacificLike;

    impl PacificLike {
        const STANDARD: i32 = -8 * 3600;
        const DAYLIGHT: i32 = -7 * 3600;

        fn standard() -> chrono::FixedOffset {
            chrono::FixedOffset::east_opt(Self::STANDARD).unwrap()
        }

        fn daylight() -> chrono::FixedOffset {
            chrono::FixedOffset::east_opt(Self::DAYLIGHT).unwrap()
        }

        fn naive(text: &str) -> NaiveDateTime {
            NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap()
        }
    }

    impl TimeZone for PacificLike {
        type Offset = chrono::FixedOffset;

        fn from_offset(_offset: &Self::Offset) -> Self {
            Self
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<Self::Offset> {
            self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
        }

        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<Self::Offset> {
            // 02:00–03:00 on the spring date never happens; 01:00–02:00 on
            // the fall date happens twice, daylight first.
            let gap_start = Self::naive("2026-03-08 02:00:00");
            let gap_end = Self::naive("2026-03-08 03:00:00");
            let fold_start = Self::naive("2026-11-01 01:00:00");
            let fold_end = Self::naive("2026-11-01 02:00:00");
            if *local >= gap_start && *local < gap_end {
                LocalResult::None
            } else if *local >= fold_start && *local < fold_end {
                // Mirror chrono's real ordering: `Ambiguous` is ordered by UTC
                // OFFSET (larger first), NOT by instant, so standard (-08:00)
                // precedes daylight (-07:00) and `.0` is the LATER instant.
                // A mock that encodes an assumption about a dependency instead
                // of the dependency's behavior is worse than no mock: reversing
                // these two would make the fall-back test pass against
                // production code that picks the wrong hour.
                LocalResult::Ambiguous(Self::standard(), Self::daylight())
            } else if *local >= gap_end && *local < fold_start {
                LocalResult::Single(Self::daylight())
            } else {
                LocalResult::Single(Self::standard())
            }
        }

        fn offset_from_utc_date(&self, utc: &NaiveDate) -> Self::Offset {
            self.offset_from_utc_datetime(&utc.and_time(NaiveTime::MIN))
        }

        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> Self::Offset {
            let spring = Self::naive("2026-03-08 10:00:00");
            let fall = Self::naive("2026-11-01 09:00:00");
            if *utc >= spring && *utc < fall {
                Self::daylight()
            } else {
                Self::standard()
            }
        }
    }

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// The weekday is derived from the date, never assumed — including
    /// across a year boundary and a leap day, the two places an assumed
    /// weekday goes wrong.
    #[test]
    fn org_timestamp_weekday_is_computed() {
        for (instant, expected) in [
            ("2026-08-09T12:31:00Z", "[2026-08-09 Sun 12:31]"),
            ("2025-12-31T23:59:00Z", "[2025-12-31 Wed 23:59]"),
            ("2026-01-01T00:00:00Z", "[2026-01-01 Thu 00:00]"),
            ("2024-02-29T06:05:00Z", "[2024-02-29 Thu 06:05]"),
            ("2024-03-01T06:05:00Z", "[2024-03-01 Fri 06:05]"),
        ] {
            assert_eq!(format_org_timestamp_in(&Utc, utc(instant)), expected);
        }
    }

    /// The reserved set must cover every key the parser reads and every key
    /// the writer emits. Derived from the source rather than transcribed, so
    /// adding a property to either side without adding it here fails here
    /// instead of silently producing a duplicated drawer entry.
    #[test]
    fn the_reserved_set_covers_every_key_obr_owns() {
        let source = include_str!("org_bridge.rs");

        // Keys the parser matches on: the string literals of the match arms in
        // `parse_properties`, which is the only place drawer keys are read.
        let parse_body = source
            .split_once("fn parse_properties(")
            .expect("parse_properties exists")
            .1;
        let mut owned: Vec<String> = Vec::new();
        for line in parse_body.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("}\n") {
                break;
            }
            // `"KEY" =>` and `"KEY" if guard =>`
            if let Some(rest) = trimmed.strip_prefix('"')
                && let Some((key, tail)) = rest.split_once('"')
                && (tail.trim_start().starts_with("=>") || tail.trim_start().starts_with("if "))
                && key.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                && !key.is_empty()
            {
                owned.push(key.to_string());
            }
        }
        assert!(
            owned.len() > 30,
            "extraction found only {} parse arms, so it is broken: {owned:?}",
            owned.len()
        );

        // Keys the writer emits through the single aligning helper.
        for line in source.lines() {
            if let Some(rest) = line.trim_start().strip_prefix("push_aligned(out, \"")
                && let Some((key, _)) = rest.split_once('"')
            {
                owned.push(key.to_string());
            }
        }

        owned.sort_unstable();
        owned.dedup();
        for key in &owned {
            assert!(
                is_reserved_org_property(key),
                "{key} is read or written by obr but is not in RESERVED_ORG_PROPERTY_KEYS, \
                 so a preserved copy would duplicate it in the drawer"
            );
        }
        // The delimiters are not keys in either list but must still be excluded.
        for delimiter in ["PROPERTIES", "END"] {
            assert!(is_reserved_org_property(delimiter));
        }
        assert!(
            is_reserved_org_property("id"),
            "matching is case-insensitive"
        );
        assert!(!is_reserved_org_property("HASH_sha512_256"));
    }

    /// A property obr does not model is read off the surface and written back,
    /// so a flush stops being a deletion.
    #[test]
    fn unknown_drawer_properties_are_harvested_and_re_emitted() {
        let mut issue = base_issue();
        issue.labels = vec!["alpha".to_string()];
        let plain = String::from_utf8(emit_issue_record(&issue).unwrap()).unwrap();
        assert!(!plain.contains("HASH_sha512_256"));

        // A surface another tool has annotated.
        let annotated = plain.replace(
            ":END:",
            ":HASH_sha512_256: 89dfd0a645113035878f1480a21e47bf\n:REVIEWED_BY: alice\n:END:",
        );
        let harvested = harvest_preserved_properties(&annotated);
        assert_eq!(
            harvested.get("bd-test").map(Vec::as_slice),
            Some(
                [
                    (
                        "HASH_sha512_256".to_string(),
                        "89dfd0a645113035878f1480a21e47bf".to_string()
                    ),
                    ("REVIEWED_BY".to_string(), "alice".to_string()),
                ]
                .as_slice()
            ),
            "both unknown properties survive, in document order"
        );

        let style = OrgStyle {
            preserved: harvested,
            ..OrgStyle::default()
        };
        let re_emitted = String::from_utf8(emit_issue_record_styled(&issue, &style).unwrap())
            .expect("valid utf-8");
        assert!(re_emitted.contains(":HASH_sha512_256: 89dfd0a645113035878f1480a21e47bf\n"));
        assert!(re_emitted.contains(":REVIEWED_BY: alice\n"));

        // Fixpoint: harvesting the re-emitted record yields the same map, so a
        // second flush reproduces the file rather than drifting.
        assert_eq!(harvest_preserved_properties(&re_emitted), style.preserved);
        assert_eq!(
            String::from_utf8(emit_issue_record_styled(&issue, &style).unwrap()).unwrap(),
            re_emitted
        );
    }

    /// obr's own properties are never harvested, however they are spelled.
    ///
    /// The retired spellings are the dangerous ones: eight parse arms are
    /// guarded, and a failed Rust match guard falls through to the wildcard, so
    /// `:CREATED_AT:` is reachable there whenever `:CREATED:` is present --
    /// which is always. Capturing them would resurrect them permanently.
    #[test]
    fn obr_owned_properties_are_never_harvested() {
        let mut issue = base_issue();
        issue.closed_at = Some(ts(1_700_000_100));
        issue.status = Status::Closed;
        let text = String::from_utf8(emit_issue_record(&issue).unwrap()).unwrap();
        assert!(
            harvest_preserved_properties(&text).is_empty(),
            "a record obr wrote itself has nothing to preserve"
        );

        // Now the same record as an older build wrote it, carrying both the
        // modern and retired spellings at once.
        let legacy = text.replace(
            ":END:",
            ":CREATED_AT: 2026-01-02T03:04:00+00:00\n\
             :UPDATED_AT: 2026-01-02T03:05:00+00:00\n\
             :CLOSED_AT: 2026-01-02T03:06:00+00:00\n\
             :CLOSED:    2026-01-02T03:06:00+00:00\n\
             :BEADS_SCHEMA_VERSION: 16\n\
             :END:",
        );
        assert!(
            harvest_preserved_properties(&legacy).is_empty(),
            "retired spellings must not be preserved, or they come back forever"
        );
    }

    /// With no declared column the heading is byte-identical to what every
    /// previous build wrote: alignment is opt-in, so upgrading obr reformats
    /// nobody's surface.
    #[test]
    fn tags_keep_the_fixed_gap_when_no_column_is_declared() {
        let mut issue = base_issue();
        issue.labels = vec!["alpha".to_string(), "beta".to_string()];
        let text = String::from_utf8(emit_issue_record(&issue).unwrap()).unwrap();
        let head = text.lines().next().unwrap();
        assert!(
            head.ends_with("Test Issue    :alpha:beta:"),
            "expected the historical four-space gap: {head:?}"
        );
    }

    /// With a column declared, the heading matches what Org's own
    /// `org--align-tags-here` produces: the line ends exactly on the column
    /// for a negative value, and a heading already past it clamps to a single
    /// space rather than being reflowed.
    #[test]
    fn tags_align_to_a_declared_column() {
        let style = OrgStyle {
            tags_column: Some(-97),
            ..OrgStyle::default()
        };
        let mut issue = base_issue();
        issue.labels = vec!["alpha".to_string(), "beta".to_string()];
        let text = String::from_utf8(emit_issue_record_styled(&issue, &style).unwrap()).unwrap();
        let head = text.lines().next().unwrap();
        assert_eq!(
            head.chars().count(),
            97,
            "line must end on the column: {head:?}"
        );
        assert!(head.ends_with(":alpha:beta:"));

        // A title long enough to overrun the column keeps exactly one space.
        issue.title = "x".repeat(120);
        let text = String::from_utf8(emit_issue_record_styled(&issue, &style).unwrap()).unwrap();
        let head = text.lines().next().unwrap();
        assert!(
            head.ends_with(" :alpha:beta:") && !head.ends_with("  :alpha:beta:"),
            "an overlong heading clamps to one space: {head:?}"
        );

        // A positive column is the column the tags START at.
        let text = String::from_utf8(
            emit_issue_record_styled(
                &base_issue_with_labels(),
                &OrgStyle {
                    tags_column: Some(40),
                    ..OrgStyle::default()
                },
            )
            .unwrap(),
        )
        .unwrap();
        let head = text.lines().next().unwrap();
        assert_eq!(
            head.find(":alpha:"),
            Some(40),
            "a positive column is where the tags begin: {head:?}"
        );
    }

    /// The declared column survives obr rewriting the file, and it is the
    /// first line so Emacs actually reads it.
    #[test]
    fn a_declared_column_round_trips_through_the_header() {
        let style = OrgStyle {
            tags_column: Some(-97),
            ..OrgStyle::default()
        };
        let header = String::from_utf8(org_file_header_styled(Some("nix"), &style)).unwrap();
        assert!(
            header.starts_with("# -*- org-tags-column: -97 -*-\n#+TITLE:"),
            "the file-local line must come first: {header:?}"
        );
        assert_eq!(org_tags_column_from_org(&header), Some(-97));

        // Emacs writes multi-variable blocks; obr must read its own value out.
        assert_eq!(
            org_tags_column_from_org("# -*- mode: org; org-tags-column: -80 -*-\n#+TITLE: x\n"),
            Some(-80)
        );
        // No block, a malformed block, and a non-numeric value are all "no
        // declared column" rather than errors.
        for text in [
            "#+TITLE: x\n",
            "# -*- org-tags-column -*-\n",
            "# -*- org-tags-column: wat -*-\n",
            "",
        ] {
            assert_eq!(org_tags_column_from_org(text), None, "{text:?}");
        }
    }

    fn base_issue_with_labels() -> Issue {
        let mut issue = base_issue();
        issue.labels = vec!["alpha".to_string(), "beta".to_string()];
        issue
    }

    /// *Every* drawer property sits in Org's property column, not just the
    /// timestamps. A drawer mixing columns is what Org's own
    /// `org--align-node-property` rewrites the moment the entry is
    /// re-indented, so a single-spaced key is a standing churn loop against
    /// Emacs. Generalizes `timestamp_properties_sit_in_orgs_property_column`
    /// to the whole drawer, so a future inline `format!(":KEY: {v}")` is
    /// caught wherever it is added.
    #[test]
    fn every_drawer_property_sits_in_orgs_property_column() {
        let mut issue = base_issue();
        issue.labels = vec!["alpha".to_string(), "beta".to_string()];
        issue.assignee = Some("alice".to_string());
        issue.owner = Some("bob".to_string());
        issue.sender = Some("carol".to_string());
        issue.created_by = Some("dave".to_string());
        issue.external_ref = Some("JIRA-1".to_string());
        issue.source_repo = Some("nix".to_string());
        issue.source_repo_path = Some("/tmp/nix".to_string());
        issue.estimated_minutes = Some(45);
        issue.original_size = Some(12);
        issue.original_type = Some("task".to_string());
        issue.pinned = true;
        issue.is_template = true;
        issue.ephemeral = true;
        issue.compaction_level = Some(2);
        issue.closed_at = Some(ts(1_700_000_100));
        issue.status = Status::Closed;
        issue.close_reason = Some("done".to_string());
        let text = String::from_utf8(emit_issue_record(&issue).unwrap()).unwrap();

        let mut checked = 0usize;
        let mut in_drawer = false;
        for line in text.lines() {
            match line {
                ":PROPERTIES:" => in_drawer = true,
                ":END:" => in_drawer = false,
                _ if in_drawer => {
                    let key_end = line[1..]
                        .find(':')
                        .unwrap_or_else(|| panic!("drawer line is not a property: {line:?}"));
                    let key_token = &line[..=key_end + 1];
                    let value = &line[key_token.len()..];
                    let pad = value.len() - value.trim_start().len();
                    assert_eq!(
                        key_token.len() + pad,
                        key_token.len().max(ORG_PROPERTY_KEY_WIDTH) + 1,
                        "value must start at Org's property column: {line:?}"
                    );
                    checked += 1;
                }
                _ => {}
            }
        }
        // Guards against the assertion passing vacuously if the drawer ever
        // stops being emitted: this issue populates every optional property.
        assert!(
            checked >= 20,
            "expected a fully populated drawer, checked only {checked} properties in:\n{text}"
        );
    }

    /// Every level-2 section heading is separated from its content by exactly
    /// one blank line.
    ///
    /// Not a style assertion. See `push_child_heading`: adjacency lets a
    /// planning keyword or drawer opener at the head of a body bind to the
    /// *heading* during parsing, where the reader never looks.
    #[test]
    fn level_two_headings_are_separated_from_their_content() {
        let mut issue = base_issue();
        issue.design = Some("the design".to_string());
        issue.acceptance_criteria = Some("the criteria".to_string());
        issue.notes = Some("the notes".to_string());
        issue.agent_context = Some("{\"k\":1}".to_string());
        issue.dependencies = vec![Dependency {
            issue_id: "bd-test".to_string(),
            depends_on_id: "bd-other".to_string(),
            dep_type: DependencyType::Blocks,
            created_at: ts(1_700_000_000),
            created_by: None,
            metadata: None,
            thread_id: None,
        }];
        let text = String::from_utf8(emit_issue_record(&issue).unwrap()).unwrap();

        let lines: Vec<&str> = text.lines().collect();
        let mut seen = 0usize;
        for (i, line) in lines.iter().enumerate() {
            if line.starts_with("** ") {
                seen += 1;
                assert_eq!(
                    lines.get(i + 1).copied(),
                    Some(""),
                    "level-2 heading {line:?} must be followed by a blank line in:\n{text}"
                );
                assert_ne!(
                    lines.get(i + 2).copied(),
                    Some(""),
                    "exactly one blank line, not two, after {line:?} in:\n{text}"
                );
            }
        }
        // Design, Acceptance Criteria, Notes, Agent Context, Dependencies —
        // covering both `push_text_child` and `push_json_child`.
        assert_eq!(seen, 5, "expected five level-2 sections in:\n{text}");
    }

    /// A child body whose first line is an Org planning keyword or a drawer
    /// opener survives the round trip.
    ///
    /// This is the regression that makes the blank line after a level-2
    /// heading a correctness fix rather than a cosmetic one: emitted
    /// adjacently, org2jsonl binds that first line to the heading's
    /// `planning` / `properties`, neither of which `parse_children` reads, so
    /// it is silently dropped on import and the surface is no longer a
    /// fixpoint. The stability probe cannot catch it — it renders the
    /// candidate body in a context that already has the blank line.
    #[test]
    fn a_planning_line_at_the_head_of_a_child_body_survives() {
        for body in [
            "SCHEDULED: <2026-01-01 Thu>\nafter",
            "DEADLINE: <2026-01-01 Thu>\nafter",
            ":PROPERTIES:\n:FOO: bar\n:END:\nafter",
            "CLOSED: [2026-01-01 Thu 09:00]\nafter",
        ] {
            let mut issue = base_issue();
            issue.notes = Some(body.to_string());
            let parsed = roundtrip(std::slice::from_ref(&issue));
            assert_eq!(
                parsed[0].notes.as_deref(),
                Some(body),
                "lost the head of a child body: {body:?}"
            );
        }
    }

    /// The rendered value sits in Org's own property column
    /// (`org-property-format`, `"%-10s %s"`), so `org-set-property` in Emacs
    /// rewrites the identical bytes.
    #[test]
    fn timestamp_properties_sit_in_orgs_property_column() {
        let mut issue = base_issue();
        issue.closed_at = Some(ts(1_700_000_100));
        issue.status = Status::Closed;
        issue.due_at = Some(ts(1_700_000_200));
        issue.defer_until = Some(ts(1_700_000_300));
        issue.deleted_at = Some(ts(1_700_000_400));
        issue.compacted_at = Some(ts(1_700_000_500));
        let text = String::from_utf8(emit_issue_record(&issue).unwrap()).unwrap();

        for key in [
            "ID",
            "CREATED",
            "MODIFIED",
            "FINISHED",
            "DUE",
            "DEFERRED",
            "DELETED",
            "COMPACTED",
        ] {
            let line = text
                .lines()
                .find(|line| line.starts_with(&format!(":{key}:")))
                .unwrap_or_else(|| panic!("no :{key}: line in:\n{text}"));
            let value_column = line.len() - line.trim_start_matches(|c| c != '[').len();
            let key_token = format!(":{key}:");
            let expected = key_token.len().max(ORG_PROPERTY_KEY_WIDTH) + 1;
            if key != "ID" {
                assert_eq!(
                    value_column, expected,
                    "value must start at Org's property column: {line:?}"
                );
            }
            assert!(
                line.starts_with(&format!("{key_token:<ORG_PROPERTY_KEY_WIDTH$}")),
                "key must be padded to Org's width: {line:?}"
            );
        }
        // Nothing writes a retired spelling any more — the pre-Org `*_AT`
        // set, nor the `:CLOSED:` that org-special-properties shadowed.
        for legacy in [
            ":CREATED_AT:",
            ":UPDATED_AT:",
            ":CLOSED_AT:",
            ":CLOSED:",
            ":DUE_AT:",
            ":DEFER_UNTIL:",
            ":DELETED_AT:",
            ":COMPACTED_AT:",
        ] {
            assert!(!text.contains(legacy), "legacy {legacy} written:\n{text}");
        }
    }

    /// `CLOSED` is in `org-special-properties` (verified against Org 9.7.11),
    /// so a drawer key of that name is shadowed by the planning-line keyword:
    /// `org-entry-get` returns nil and `org-entry-put` errors. No property
    /// this module writes may collide with that list.
    #[test]
    fn no_emitted_property_collides_with_org_special_properties() {
        /// `org-special-properties` as evaluated in Org 9.7.11 (Emacs 30) and
        /// confirmed byte-identical in Org 9.8.7 (Emacs 31).
        const ORG_SPECIAL_PROPERTIES: [&str; 14] = [
            "ALLTAGS",
            "BLOCKED",
            "CLOCKSUM",
            "CLOCKSUM_T",
            "CLOSED",
            "DEADLINE",
            "FILE",
            "ITEM",
            "PRIORITY",
            "SCHEDULED",
            "TAGS",
            "TIMESTAMP",
            "TIMESTAMP_IA",
            "TODO",
        ];

        // Every optional field populated, so the drawer is at its widest.
        let mut issue = base_issue();
        issue.status = Status::Closed;
        issue.title = "Wide".to_string();
        issue.labels = vec!["a".to_string()];
        issue.assignee = Some("alice".to_string());
        issue.owner = Some("bob".to_string());
        issue.estimated_minutes = Some(30);
        issue.created_by = Some("carol".to_string());
        issue.closed_at = Some(ts(1_700_000_100));
        issue.close_reason = Some("done".to_string());
        issue.closed_by_session = Some("sess".to_string());
        issue.due_at = Some(ts(1_700_000_200));
        issue.defer_until = Some(ts(1_700_000_300));
        issue.external_ref = Some("ext".to_string());
        issue.source_system = Some("sys".to_string());
        issue.source_repo = Some("repo".to_string());
        issue.source_repo_path = Some("path".to_string());
        issue.deleted_at = Some(ts(1_700_000_400));
        issue.deleted_by = Some("dave".to_string());
        issue.delete_reason = Some("obsolete".to_string());
        issue.original_type = Some("bug".to_string());
        issue.compaction_level = Some(1);
        issue.compacted_at = Some(ts(1_700_000_500));
        issue.compacted_at_commit = Some("abc123".to_string());
        issue.original_size = Some(4096);
        issue.sender = Some("erin".to_string());
        issue.ephemeral = true;
        issue.pinned = true;
        issue.is_template = true;
        let text = String::from_utf8(emit_issue_record(&issue).unwrap()).unwrap();

        for special in ORG_SPECIAL_PROPERTIES {
            assert!(
                !text.contains(&format!(":{special}:")),
                "emitted :{special}:, which Org shadows as a special property:\n{text}"
            );
        }
        // Positive control: the property that forced this rule is present
        // under its non-colliding name.
        assert!(text.contains(":FINISHED: ["), "got:\n{text}");
    }

    /// All seven timestamps survive a round trip, each under its own
    /// property.
    #[test]
    fn every_timestamp_property_roundtrips() {
        let mut issue = base_issue();
        issue.status = Status::Closed;
        issue.closed_at = Some(ts(1_700_000_100));
        issue.due_at = Some(ts(1_700_000_200));
        issue.defer_until = Some(ts(1_700_000_300));
        issue.deleted_at = Some(ts(1_700_000_400));
        issue.compacted_at = Some(ts(1_700_000_500));

        let got = &roundtrip(std::slice::from_ref(&issue))[0];
        assert_eq!(got.created_at, issue.created_at);
        assert_eq!(got.updated_at, issue.updated_at);
        assert_eq!(got.closed_at, issue.closed_at);
        assert_eq!(got.due_at, issue.due_at);
        assert_eq!(got.defer_until, issue.defer_until);
        assert_eq!(got.deleted_at, issue.deleted_at);
        assert_eq!(got.compacted_at, issue.compacted_at);
    }

    /// Seconds and sub-seconds are not representable, so the FIRST write
    /// drops them and the second write changes nothing. Minute precision
    /// becomes the data model after a round trip; see `docs/RESIDUALS.md`.
    #[test]
    fn org_timestamps_truncate_to_the_minute() {
        let mut issue = base_issue();
        // 22:13:37.123456789 and 22:13:59.999999999 — one minute, two
        // distinguishable instants going in.
        issue.created_at = Utc.timestamp_opt(1_700_000_017, 123_456_789).unwrap();
        issue.updated_at = Utc.timestamp_opt(1_700_000_039, 999_999_999).unwrap();

        let gen1 = issues_to_org_text(std::slice::from_ref(&issue)).expect("emit gen1");
        let parsed = org_text_to_issues(&gen1).expect("parse gen1");
        let gen2 = issues_to_org_text(&parsed).expect("emit gen2");
        assert_eq!(gen1, gen2, "truncation must converge on the first write");

        assert_eq!(parsed[0].created_at, ts(1_700_000_000));
        assert_eq!(parsed[0].updated_at, ts(1_700_000_000));
        // Both stamps fell in the same minute and are now indistinguishable,
        // which is exactly the last-write-wins hazard RESIDUALS records.
        assert_eq!(parsed[0].created_at, parsed[0].updated_at);
    }

    /// A `PLAN.org` written before the Org timestamps existed imports
    /// losslessly — every one of the seven `*_AT` spellings — and re-exports
    /// in the current form.
    #[test]
    fn legacy_at_properties_import_and_re_export_in_the_current_form() {
        let text = "#+TITLE: Obr Issues\n\n* DONE [#C] Legacy\n\
                    :PROPERTIES:\n\
                    :ID:       bd-legacy\n\
                    :CREATED_AT: 2023-11-14T22:13:00+00:00\n\
                    :UPDATED_AT: 2023-11-14T22:14:00+00:00\n\
                    :CLOSED_AT: 2023-11-14T22:15:00+00:00\n\
                    :DUE_AT: 2023-11-14T22:16:00+00:00\n\
                    :DEFER_UNTIL: 2023-11-14T22:17:00+00:00\n\
                    :DELETED_AT: 2023-11-14T22:18:00+00:00\n\
                    :COMPACTED_AT: 2023-11-14T22:19:00+00:00\n\
                    :END:\n";
        let parsed = org_text_to_issues(text).expect("a legacy surface must import");
        let got = &parsed[0];
        assert_eq!(got.created_at, utc("2023-11-14T22:13:00Z"));
        assert_eq!(got.updated_at, utc("2023-11-14T22:14:00Z"));
        assert_eq!(got.closed_at, Some(utc("2023-11-14T22:15:00Z")));
        assert_eq!(got.due_at, Some(utc("2023-11-14T22:16:00Z")));
        assert_eq!(got.defer_until, Some(utc("2023-11-14T22:17:00Z")));
        assert_eq!(got.deleted_at, Some(utc("2023-11-14T22:18:00Z")));
        assert_eq!(got.compacted_at, Some(utc("2023-11-14T22:19:00Z")));

        // Re-export is the current form only, and is itself a fixpoint.
        let gen1 = issues_to_org_text(&parsed).expect("re-export");
        assert!(gen1.contains(":CREATED:  ["), "got:\n{gen1}");
        assert!(gen1.contains(":MODIFIED: ["), "got:\n{gen1}");
        assert!(
            !gen1.contains("_AT:"),
            "legacy spelling re-emitted:\n{gen1}"
        );
        let gen2 = issues_to_org_text(&org_text_to_issues(&gen1).expect("reparse")).expect("emit");
        assert_eq!(gen1, gen2);
    }

    /// A file that somehow carries both spellings of one field resolves the
    /// same way whichever order they appear in: the current spelling wins.
    #[test]
    fn current_timestamp_spelling_shadows_the_legacy_one() {
        for drawer in [
            ":CREATED:  [2026-08-09 Sun 12:31]\n:CREATED_AT: 2001-01-01T00:00:00+00:00\n",
            ":CREATED_AT: 2001-01-01T00:00:00+00:00\n:CREATED:  [2026-08-09 Sun 12:31]\n",
        ] {
            let text = format!(
                "#+TITLE: Obr Issues\n\n* TODO [#C] Both\n:PROPERTIES:\n\
                 :ID:       bd-both\n{drawer}:END:\n"
            );
            let parsed = org_text_to_issues(&text).expect("import");
            assert_eq!(
                parsed[0].created_at,
                local_to_instant(&Local, PacificLike::naive("2026-08-09 12:31:00"))
                    .expect("2026-08-09 12:31 is not a DST edge in any zone"),
                "the current spelling must win regardless of drawer order"
            );
        }
    }

    /// `closed_at` is the one field with two retired spellings. All three are
    /// read, and every one of them re-exports as `:FINISHED:` with the
    /// instant intact.
    #[test]
    fn closed_at_reads_all_three_spellings_and_re_exports_as_finished() {
        for line in [
            ":FINISHED: 2023-11-14T22:15:00+00:00\n",
            ":CLOSED:   2023-11-14T22:15:00+00:00\n",
            ":CLOSED_AT: 2023-11-14T22:15:00+00:00\n",
        ] {
            let text = format!(
                "#+TITLE: Obr Issues\n\n* DONE [#C] Closed\n:PROPERTIES:\n\
                 :ID:       bd-closed\n:CREATED:  [2026-08-09 Sun 12:31]\n\
                 :MODIFIED: [2026-08-09 Sun 12:31]\n{line}:END:\n"
            );
            let parsed = org_text_to_issues(&text).unwrap_or_else(|e| panic!("{line:?}: {e}"));
            assert_eq!(
                parsed[0].closed_at,
                Some(utc("2023-11-14T22:15:00Z")),
                "{line:?} lost the close instant"
            );

            let gen1 = issues_to_org_text(&parsed).expect("re-export");
            assert!(gen1.contains(":FINISHED: ["), "{line:?} gave:\n{gen1}");
            assert!(!gen1.contains(":CLOSED:"), "{line:?} gave:\n{gen1}");
            assert!(!gen1.contains(":CLOSED_AT:"), "{line:?} gave:\n{gen1}");
            let gen2 =
                issues_to_org_text(&org_text_to_issues(&gen1).expect("reparse")).expect("emit");
            assert_eq!(gen1, gen2, "{line:?} is not a fixpoint");
        }
    }

    /// A surface from the unreleased build that spelled it `:CLOSED:` — an
    /// Org timestamp under the retired key — imports losslessly and comes
    /// back out as `:FINISHED:`. Scratch workspaces really do carry this.
    #[test]
    fn the_retired_closed_org_timestamp_imports_and_re_exports() {
        let text = "#+TITLE: Obr Issues\n\n* DONE [#C] Retired\n\
                    :PROPERTIES:\n\
                    :ID:       bd-retired\n\
                    :CREATED:  [2026-08-09 Sun 12:31]\n\
                    :MODIFIED: [2026-08-09 Sun 12:31]\n\
                    :CLOSED:   [2026-08-09 Sun 12:31]\n\
                    :END:\n";
        let expected = local_to_instant(&Local, PacificLike::naive("2026-08-09 12:31:00"))
            .expect("2026-08-09 12:31 is not a DST edge in any zone");
        let parsed = org_text_to_issues(text).expect("import");
        assert_eq!(parsed[0].closed_at, Some(expected));

        let gen1 = issues_to_org_text(&parsed).expect("re-export");
        assert!(
            gen1.contains(":FINISHED: [2026-08-09 Sun 12:31]"),
            "got:\n{gen1}"
        );
        assert!(!gen1.contains(":CLOSED:"), "got:\n{gen1}");
    }

    /// Precedence over the three spellings is total and order-independent:
    /// `FINISHED` beats `CLOSED` beats `CLOSED_AT`.
    #[test]
    fn finished_shadows_both_retired_closed_spellings() {
        const FINISHED: &str = ":FINISHED: 2003-03-03T03:03:00+00:00\n";
        const CLOSED: &str = ":CLOSED:   2002-02-02T02:02:00+00:00\n";
        const CLOSED_AT: &str = ":CLOSED_AT: 2001-01-01T01:01:00+00:00\n";

        // Every ordering of every combination, with the instant the winning
        // spelling carries.
        for (drawer, winner) in [
            (
                format!("{FINISHED}{CLOSED}{CLOSED_AT}"),
                "2003-03-03T03:03:00Z",
            ),
            (
                format!("{CLOSED_AT}{CLOSED}{FINISHED}"),
                "2003-03-03T03:03:00Z",
            ),
            (
                format!("{CLOSED}{FINISHED}{CLOSED_AT}"),
                "2003-03-03T03:03:00Z",
            ),
            (format!("{CLOSED}{CLOSED_AT}"), "2002-02-02T02:02:00Z"),
            (format!("{CLOSED_AT}{CLOSED}"), "2002-02-02T02:02:00Z"),
            (format!("{FINISHED}{CLOSED_AT}"), "2003-03-03T03:03:00Z"),
            (format!("{CLOSED_AT}{FINISHED}"), "2003-03-03T03:03:00Z"),
            (CLOSED_AT.to_string(), "2001-01-01T01:01:00Z"),
        ] {
            let text = format!(
                "#+TITLE: Obr Issues\n\n* DONE [#C] Both\n:PROPERTIES:\n\
                 :ID:       bd-both\n{drawer}:END:\n"
            );
            let parsed = org_text_to_issues(&text).expect("import");
            assert_eq!(
                parsed[0].closed_at,
                Some(utc(winner)),
                "wrong spelling won for drawer:\n{drawer}"
            );
        }
    }

    /// Fall-back: the local hour that happens twice resolves to the EARLIER
    /// instant, and re-rendering it is a fixpoint (both candidates render to
    /// the same local text, so the surface is stable even though the stored
    /// instant may shift by the offset delta on the way in).
    #[test]
    fn dst_fall_back_resolves_to_the_earlier_instant() {
        let daylight = utc("2026-11-01T08:30:00Z"); // 01:30 PDT
        let standard = utc("2026-11-01T09:30:00Z"); // 01:30 PST
        let rendered = "[2026-11-01 Sun 01:30]";
        assert_eq!(format_org_timestamp_in(&PacificLike, daylight), rendered);
        assert_eq!(format_org_timestamp_in(&PacificLike, standard), rendered);

        let parsed = parse_org_timestamp_in(&PacificLike, rendered).expect("ambiguous must parse");
        assert_eq!(
            parsed, daylight,
            "the earlier instant is the documented rule"
        );
        assert_eq!(
            format_org_timestamp_in(&PacificLike, parsed),
            rendered,
            "the ambiguous reading must still be a rendering fixpoint"
        );
    }

    /// Spring-forward: a local hour that never happens can only come from a
    /// hand-edit, and resolves to the first representable instant at or
    /// after it — the transition itself.
    #[test]
    fn dst_gap_resolves_to_the_first_representable_instant() {
        let transition = utc("2026-03-08T10:00:00Z"); // 03:00 PDT
        let parsed = parse_org_timestamp_in(&PacificLike, "[2026-03-08 Sun 02:30]")
            .expect("a nonexistent local reading must still import");
        assert_eq!(parsed, transition);
        assert_eq!(
            format_org_timestamp_in(&PacificLike, parsed),
            "[2026-03-08 Sun 03:00]",
            "the next flush rewrites the property to a reading that exists"
        );
    }

    /// The reader takes the shapes a hand-edit produces; the writer emits
    /// one of them and normalizes the rest on the next flush.
    #[test]
    fn timestamp_reader_accepts_hand_edited_shapes() {
        let noon = utc("2026-08-09T12:31:00Z");
        for text in [
            "[2026-08-09 Sun 12:31]",
            "<2026-08-09 Sun 12:31>",       // active
            "[2026-08-09 12:31]",           // weekday omitted
            "[2026-08-09 Wednesday 12:31]", // weekday wrong; recomputed on write
        ] {
            assert_eq!(
                parse_org_timestamp_in(&Utc, text),
                Some(noon),
                "must accept {text:?}"
            );
        }
        // Reading never truncates — that is what makes a legacy RFC3339 file
        // import losslessly. Only the write drops what Org cannot carry, so
        // a hand-written clock with seconds survives the import and is
        // normalized by the next flush.
        assert_eq!(
            parse_org_timestamp_in(&Utc, "[2026-08-09 Sun 12:31:44]"),
            Some(utc("2026-08-09T12:31:44Z"))
        );
        assert_eq!(
            format_org_timestamp_in(&Utc, utc("2026-08-09T12:31:44Z")),
            "[2026-08-09 Sun 12:31]"
        );
        assert_eq!(
            parse_org_timestamp_in(&Utc, "[2026-08-09 Sun]"),
            Some(utc("2026-08-09T00:00:00Z")),
            "a date with no clock is midnight"
        );
        // RFC3339 is accepted under either spelling of the key, so a
        // machine-written hand-edit imports too.
        assert_eq!(
            parse_timestamp_value("2026-08-09T12:31:00+00:00", "created_at").unwrap(),
            noon
        );
    }

    /// Shapes obr cannot store fail the import instead of being dropped
    /// silently by the next rewrite — the same choice the module makes for a
    /// broken JSON section.
    #[test]
    fn timestamp_reader_refuses_what_it_cannot_store() {
        for text in [
            "[2026-08-09 Sun 12:31 +1w]",                     // repeater
            "[2026-08-09 Sun 12:31 -2d]",                     // warning cookie
            "[2026-08-09 Sun 12:31]--[2026-08-10 Mon 12:31]", // range
            "[2026-08-09 Sun 25:00]",                         // not a clock
            "[2026-13-09 Sun 12:31]",                         // not a date
            "[2026-08-09 Sun 12:31",                          // unclosed
            "2026-08-09 Sun 12:31",                           // unbracketed
            "",
        ] {
            assert_eq!(
                parse_org_timestamp_in(&Utc, text),
                None,
                "accepted {text:?}"
            );
            assert!(
                parse_timestamp_value(text, "created_at").is_err(),
                "accepted {text:?}"
            );
        }
        let err = parse_timestamp_value("tomorrow", "due_at")
            .unwrap_err()
            .to_string();
        assert!(err.contains("due_at"), "must name the field: {err}");
        assert!(
            err.contains("Org timestamp"),
            "must say what is wanted: {err}"
        );
    }

    #[test]
    fn legacy_file_without_labels_property_reads_tags() {
        let text = "#+TITLE: Obr Issues\n\n* TODO [#C] Legacy    :alpha:beta:\n\
                    :PROPERTIES:\n:ID:       bd-legacy\n\
                    :CREATED_AT: 2023-11-14T22:13:20+00:00\n\
                    :UPDATED_AT: 2023-11-14T22:13:20+00:00\n:END:\n";
        let parsed = org_text_to_issues(text).unwrap();
        assert_eq!(
            parsed[0].labels,
            vec!["alpha".to_string(), "beta".to_string()]
        );
    }

    #[test]
    fn title_with_trailing_tag_pattern_roundtrips() {
        let mut issue = base_issue();
        issue.title = "Support the :provides: syntax:".to_string();
        let parsed = roundtrip(std::slice::from_ref(&issue));
        assert_eq!(parsed[0].title, issue.title);
        assert_eq!(parsed[0].labels, Vec::<String>::new());
    }

    #[test]
    fn agent_context_roundtrips_verbatim() {
        let mut issue = base_issue();
        issue.agent_context =
            Some("{\n  \"constraints\": [\"no unsafe\"],\n  \"workflow\": \"tdd\"\n}".to_string());
        let parsed = roundtrip(std::slice::from_ref(&issue));
        assert_eq!(parsed[0].agent_context, issue.agent_context);
    }

    #[test]
    fn description_with_list_and_src_block_survives() {
        let mut issue = base_issue();
        issue.description = Some(
            "Intro paragraph.\n\n- first item\n- second item\n\n#+begin_src rust\nfn main() {}\n#+end_src"
                .to_string(),
        );
        let parsed = roundtrip(std::slice::from_ref(&issue));
        assert_eq!(parsed[0].description, issue.description);
    }

    #[test]
    fn description_with_table_survives() {
        let mut issue = base_issue();
        issue.description = Some("| a | b |\n| 1 | 2 |".to_string());
        let parsed = roundtrip(std::slice::from_ref(&issue));
        assert_eq!(parsed[0].description, issue.description);
    }

    #[test]
    fn description_roundtrip_is_a_fixpoint() {
        let mut issue = base_issue();
        issue.description =
            Some("Before.\n\n- a\n-\n\nAfter with *bold* text.\n\n| x |\n".to_string());
        let gen1 = roundtrip(std::slice::from_ref(&issue));
        let gen2 = roundtrip(&gen1);
        assert_eq!(gen1[0].description, gen2[0].description);
        let gen3 = roundtrip(&gen2);
        assert_eq!(gen2[0].description, gen3[0].description);
    }

    /// A value carrying newlines and a `:END:` token must not be able to
    /// close the drawer early. Flattening the newlines is what achieves
    /// that: the value stays on its `:KEY:` line, where a `:END:` token is
    /// inert. The token itself is preserved — rewriting it was destroying
    /// data (see `end_token_in_values_is_preserved_and_roundtrips`).
    #[test]
    fn issue_type_cannot_inject_a_drawer_terminator() {
        let mut issue = base_issue();
        issue.issue_type = IssueType::Custom("evil\n:END:\ntype".to_string());
        let text = issues_to_org_text(&[issue]).unwrap();
        assert_eq!(
            text.lines().filter(|l| l.trim() == ":END:").count(),
            1,
            "exactly one line may be a drawer terminator: {text}"
        );
        // The drawer still parses, and everything after it is body, not
        // orphaned drawer lines.
        let parsed = org_text_to_issues(&text).unwrap();
        assert_eq!(parsed.len(), 1);
        // `IssueType::from_str` lower-cases custom types; the newlines are
        // what the drawer cannot carry, and the `:END:` token survives.
        assert_eq!(
            parsed[0].issue_type,
            IssueType::Custom("evil :end: type".to_string())
        );
    }

    /// `:END:` inside a property value is inert (it is never at line start),
    /// so it must survive verbatim. It used to be rewritten to `:END `, with
    /// no inverse on the read path — silently corrupting the authoritative
    /// `:LABELS:` payload, since `x:END:y` is a label `LabelValidator`
    /// accepts.
    #[test]
    fn end_token_in_values_is_preserved_and_roundtrips() {
        let mut issue = base_issue();
        issue.labels = vec!["x:END:y".to_string(), "plain".to_string()];
        issue.title = "Fix the :END: token".to_string();
        issue.close_reason = Some("closed by :END: handling".to_string());

        let text = issues_to_org_text(std::slice::from_ref(&issue)).unwrap();
        assert!(
            text.contains(r#":LABELS:   ["plain","x:END:y"]"#),
            "labels must be emitted verbatim: {text}"
        );
        assert_eq!(
            text.lines().filter(|l| l.trim() == ":END:").count(),
            1,
            "drawer must terminate exactly once: {text}"
        );

        let parsed = org_text_to_issues(&text).unwrap();
        assert_eq!(
            parsed[0].labels,
            vec!["plain".to_string(), "x:END:y".to_string()]
        );
        assert_eq!(parsed[0].title, issue.title);
        assert_eq!(parsed[0].close_reason, issue.close_reason);
        // And the whole record is a fixpoint from the first write.
        assert_eq!(issues_to_org_text(&parsed).unwrap(), text);
    }

    /// An issue that merely quotes git conflict markers must produce an
    /// export the sync safety layer's marker scan accepts, and the quoted
    /// text must round-trip. (Found on the real 549-issue tracker corpus:
    /// issues about merge conflicts quote marker syntax in descriptions.)
    #[test]
    fn quoted_conflict_markers_are_escaped_and_roundtrip() {
        let mut issue = base_issue();
        let desc =
            "resolve like:\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> feature-branch\ndone";
        issue.description = Some(desc.to_string());

        let record = emit_issue_record(&issue).unwrap();
        let text = String::from_utf8(record).unwrap();
        for line in text.lines() {
            assert!(
                !line.starts_with("<<<<<<<")
                    && !line.starts_with("=======")
                    && !line.starts_with(">>>>>>>"),
                "unescaped conflict marker in emission: {line}"
            );
        }

        let parsed = roundtrip(std::slice::from_ref(&issue));
        assert_eq!(parsed[0].description.as_deref(), Some(desc));
    }

    /// Multi-line close/delete reasons flatten to one line inside a drawer
    /// property (the sanitizer replaces newlines with spaces — six close
    /// reasons on the real corpus lost their list structure that way), so
    /// they travel as text children instead and round-trip exactly.
    /// Single-line reasons stay compact drawer properties.
    #[test]
    fn multiline_reasons_roundtrip_via_text_children() {
        let mut issue = base_issue();
        let close = "All done:\n- allowlist (a.1)\n- atomic export (a.2)";
        let delete = "superseded by:\nthe rewrite";
        issue.close_reason = Some(close.to_string());
        issue.delete_reason = Some(delete.to_string());

        let record = emit_issue_record(&issue).unwrap();
        let text = String::from_utf8(record).unwrap();
        assert!(!text.contains(":CLOSE_REASON:"), "{text}");
        assert!(!text.contains(":DELETE_REASON:"), "{text}");
        assert!(text.contains("** Close Reason"), "{text}");
        assert!(text.contains("** Delete Reason"), "{text}");

        let parsed = roundtrip(std::slice::from_ref(&issue));
        assert_eq!(parsed[0].close_reason.as_deref(), Some(close));
        assert_eq!(parsed[0].delete_reason.as_deref(), Some(delete));

        let mut single = base_issue();
        single.close_reason = Some("done".to_string());
        let text = String::from_utf8(emit_issue_record(&single).unwrap()).unwrap();
        assert!(text.contains(":CLOSE_REASON: done"), "{text}");
        assert!(!text.contains("** Close Reason"), "{text}");
        let parsed = roundtrip(std::slice::from_ref(&single));
        assert_eq!(parsed[0].close_reason.as_deref(), Some("done"));
    }

    /// A body whose only instability is its line endings (trailing newline,
    /// CRLF) must NOT take the example-block fallback: neither emission path
    /// can carry those bytes, so wrapping just unwraps again one generation
    /// later (seen as four dropped wrappers between generations of the
    /// real-corpus conversion). It normalizes once at the storage level and
    /// the file is a fixpoint from the first write.
    #[test]
    fn trailing_newline_only_instability_stays_native() {
        for (input, canon) in [("x\n", "x"), ("a\r\nb", "a\nb"), ("a\n \n", "a\n ")] {
            let mut issue = base_issue();
            issue.description = Some(input.to_string());
            let record = emit_issue_record(&issue).unwrap();
            let text = String::from_utf8(record).unwrap();
            if canon == "a\n " {
                // Still genuinely unstable (trailing-blank line): fallback,
                // but a fixpoint on the canonical form from generation one.
                assert!(text.contains("#+begin_example"), "{input:?}: {text}");
            } else {
                assert!(!text.contains("#+begin_example"), "{input:?}: {text}");
            }
            let gen1 = roundtrip(std::slice::from_ref(&issue));
            assert_eq!(gen1[0].description.as_deref(), Some(canon), "{input:?}");
            let gen2 = roundtrip(&gen1);
            assert_eq!(gen2[0].description.as_deref(), Some(canon), "{input:?}");
        }
    }

    /// The canonical body form is a fixpoint of itself. Everything else in
    /// this family depends on it: the emitter applies line-oriented
    /// transforms after canonicalizing, and the previous
    /// `lines().collect().join("\n")` canonicalization dropped one trailing
    /// newline per pass, so a second pass anywhere changed the text.
    #[test]
    fn canonical_body_text_is_idempotent() {
        for input in [
            "",
            "\n",
            "\n\n",
            "a",
            "a\n",
            "a\n\n",
            "a\n\n\n",
            "\n\na\n\n",
            "\r\n",
            "a\r\nb",
            "a\r\n\r\n",
            "a\n \n",
            " \n ",
            " \na",
            "\n \n\ta\n",
            "a\n\nb\n\n",
            "\n\n\n",
        ] {
            let once = canonical_body_text(input);
            let twice = canonical_body_text(&once);
            assert_eq!(once, twice, "canon must be idempotent on {input:?}");
        }
    }

    /// Bodies ending (or starting) in several newlines used to wrap in an
    /// example block on the first write and unwrap on the second: the
    /// wrapper's own `lines()` pass removed a newline the canonicalization
    /// had left behind. Edge newlines are not representable on the surface
    /// — the blank line around a body is its delimiter — so they normalize
    /// once, on the first flush, and the file is a fixpoint immediately.
    #[test]
    fn bodies_with_edge_newlines_are_a_fixpoint_from_the_first_write() {
        for input in ["a\n\n", "a\n\n\n", "\n\na\n\n", "a\r\n\r\n", "a\nb\n\n"] {
            let mut issue = base_issue();
            issue.description = Some(input.to_string());
            issue.design = Some(input.to_string());
            issue.notes = Some(input.to_string());
            issue.agent_context = Some(format!("{{\"k\": \"v\"}}{}", "\n\n"));
            let (text, parsed) = assert_first_write_fixpoint(&issue);
            let expected = canonical_body_text(input);
            assert_eq!(parsed.description.as_deref(), Some(expected.as_str()));
            assert_eq!(parsed.design.as_deref(), Some(expected.as_str()));
            assert_eq!(parsed.notes.as_deref(), Some(expected.as_str()));
            assert_eq!(parsed.agent_context.as_deref(), Some("{\"k\": \"v\"}"));
            assert!(
                !text.contains("#+begin_example"),
                "{input:?} needs no wrapper once canonical: {text}"
            );
        }
    }

    /// A body that canonicalizes to whitespace only has nothing the surface
    /// can carry: the Org parser yields no elements for it, so it reads back
    /// as absent. Emitting it wrote blank lines that vanished on the next
    /// flush.
    #[test]
    fn whitespace_only_bodies_are_not_emitted() {
        for input in ["", "\n", "\n\n", " ", " \n ", "\t"] {
            let mut issue = base_issue();
            issue.description = Some(input.to_string());
            issue.design = Some(input.to_string());
            issue.acceptance_criteria = Some(input.to_string());
            issue.notes = Some(input.to_string());
            issue.agent_context = Some(input.to_string());
            let (text, parsed) = assert_first_write_fixpoint(&issue);
            assert!(
                !text.contains("** Design")
                    && !text.contains("** Notes")
                    && !text.contains("** Acceptance Criteria")
                    && !text.contains("** Agent Context"),
                "{input:?} must emit no child sections: {text}"
            );
            assert_eq!(parsed.description, None, "{input:?}");
            assert_eq!(parsed.design, None, "{input:?}");
            assert_eq!(parsed.acceptance_criteria, None, "{input:?}");
            assert_eq!(parsed.notes, None, "{input:?}");
            assert_eq!(parsed.agent_context, None, "{input:?}");
        }
    }

    /// Property values are trimmed on write because the drawer parser trims
    /// on read: `" alice "` came back as `"alice"`, so the second write
    /// differed from the first on every padded string field.
    #[test]
    fn padded_property_values_normalize_on_the_first_write() {
        let mut issue = base_issue();
        issue.assignee = Some(" alice ".to_string());
        issue.owner = Some("\tbob\t".to_string());
        issue.external_ref = Some("  JIRA-1  ".to_string());
        issue.close_reason = Some(" done ".to_string());

        let (text, parsed) = assert_first_write_fixpoint(&issue);
        assert!(text.contains(":ASSIGNEE: alice\n"), "{text}");
        assert!(text.contains(":OWNER:    bob\n"), "{text}");
        assert!(text.contains(":EXTERNAL_REF: JIRA-1\n"), "{text}");
        assert!(text.contains(":CLOSE_REASON: done\n"), "{text}");
        assert_eq!(parsed.assignee.as_deref(), Some("alice"));
        assert_eq!(parsed.owner.as_deref(), Some("bob"));
        assert_eq!(parsed.external_ref.as_deref(), Some("JIRA-1"));
        assert_eq!(parsed.close_reason.as_deref(), Some("done"));
    }

    /// `Status::Pinned` with `pinned: false` — what `obr create --status
    /// pinned` stores — used to omit `:PINNED:` on the first write, gain the
    /// flag on import, and grow the line on the second.
    #[test]
    fn pinned_status_without_the_flag_is_a_fixpoint() {
        let mut issue = base_issue();
        issue.status = Status::Pinned;
        issue.pinned = false;
        let (text, parsed) = assert_first_write_fixpoint(&issue);
        assert!(text.contains(":PINNED:   true\n"), "{text}");
        assert_eq!(parsed.status, Status::Pinned);
        assert!(parsed.pinned);

        // The flag alone, without the status, is untouched by the change.
        let mut flagged = base_issue();
        flagged.pinned = true;
        let (_, parsed) = assert_first_write_fixpoint(&flagged);
        assert_eq!(parsed.status, Status::Open);
        assert!(parsed.pinned);
    }

    /// A reason whose canonical form is one line belongs in the drawer on
    /// the FIRST write. Classifying the raw value put `"done\n"` in a
    /// `** Close Reason` child, and the parse-trimmed `"done"` back in the
    /// drawer one generation later — a whole section of difference.
    #[test]
    fn reasons_are_classified_on_their_canonical_form() {
        let mut issue = base_issue();
        issue.close_reason = Some("done\n".to_string());
        issue.delete_reason = Some("obsolete\n\n".to_string());

        let (text, parsed) = assert_first_write_fixpoint(&issue);
        assert!(text.contains(":CLOSE_REASON: done\n"), "{text}");
        assert!(text.contains(":DELETE_REASON: obsolete\n"), "{text}");
        assert!(!text.contains("** Close Reason"), "{text}");
        assert!(!text.contains("** Delete Reason"), "{text}");
        assert_eq!(parsed.close_reason.as_deref(), Some("done"));
        assert_eq!(parsed.delete_reason.as_deref(), Some("obsolete"));

        // A genuinely multi-line reason still travels as a text child.
        let mut multi = base_issue();
        multi.close_reason = Some("done:\n- a\n- b\n".to_string());
        let (text, parsed) = assert_first_write_fixpoint(&multi);
        assert!(text.contains("** Close Reason"), "{text}");
        assert!(!text.contains(":CLOSE_REASON:"), "{text}");
        assert_eq!(parsed.close_reason.as_deref(), Some("done:\n- a\n- b"));
    }

    /// A level-2 section obr does not model is dropped on rewrite, so the
    /// import announces it instead of staying silent. The parse still
    /// succeeds: a stray heading is a hand-edit, not a corrupt file.
    #[test]
    fn unrecognized_child_section_warns_and_parses() {
        let text = "#+TITLE: Obr Issues\n\n* TODO [#C] T\n:PROPERTIES:\n:ID:       bd-x\n\
                    :CREATED_AT: 2023-11-14T22:13:20+00:00\n\
                    :UPDATED_AT: 2023-11-14T22:13:20+00:00\n:END:\n\n\
                    ** Scratch notes\nsome text\n";
        let parsed = org_text_to_issues(text).expect("stray sections must not fail the parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "bd-x");
        assert_eq!(parsed[0].notes, None);
    }

    /// A reserved JSON section whose block a hand-edit broke must fail the
    /// import, naming the issue and the section. Importing it as empty and
    /// flushing deleted every dependency or comment it held.
    #[test]
    fn reserved_section_without_a_json_block_is_refused() {
        for (section, body) in [
            ("Dependencies", "just prose, no block"),
            ("Comments", "#+begin_src JSON\n[]\n#+end_src"),
            ("Agent Context", "#+begin_example\n{}\n#+end_example"),
        ] {
            let text = format!(
                "#+TITLE: Obr Issues\n\n* TODO [#C] T\n:PROPERTIES:\n:ID:       bd-x\n\
                 :CREATED_AT: 2023-11-14T22:13:20+00:00\n\
                 :UPDATED_AT: 2023-11-14T22:13:20+00:00\n:END:\n\n\
                 ** {section}\n{body}\n"
            );
            let err = org_text_to_issues(&text).unwrap_err().to_string();
            assert!(err.contains("bd-x"), "must name the issue: {err}");
            assert!(err.contains(section), "must name the section: {err}");
            assert!(
                err.contains("begin_src json"),
                "must say what is wanted: {err}"
            );
        }
    }

    /// The well-formed shape is unaffected: obr's own emission still
    /// round-trips through the same guard.
    #[test]
    fn reserved_sections_with_valid_json_are_unchanged() {
        let mut issue = base_issue();
        issue.dependencies = vec![Dependency {
            issue_id: "bd-test".to_string(),
            depends_on_id: "bd-other".to_string(),
            dep_type: DependencyType::Blocks,
            created_at: ts(1_700_000_000),
            created_by: None,
            metadata: None,
            thread_id: None,
        }];
        issue.comments = vec![Comment {
            id: 1,
            issue_id: "bd-test".to_string(),
            author: "alice".to_string(),
            body: "hello".to_string(),
            created_at: ts(1_700_000_050),
        }];
        issue.agent_context = Some("{\n  \"skills\": [\"rust\"]\n}".to_string());
        let (_, parsed) = assert_first_write_fixpoint(&issue);
        assert_eq!(parsed.dependencies, issue.dependencies);
        assert_eq!(parsed.comments, issue.comments);
        assert_eq!(parsed.agent_context, issue.agent_context);
    }

    /// Optional string fields holding `Some("")` (the Go corpus is full of
    /// zero-value strings) are not emitted at all: an empty-valued property
    /// reads back as absent anyway, and emitting it made generation two of
    /// the real-corpus conversion differ from generation one by exactly the
    /// dropped `:COMPACTED_AT_COMMIT: ` lines. First write == fixpoint.
    #[test]
    fn empty_optional_properties_are_not_emitted() {
        let mut issue = base_issue();
        issue.compacted_at_commit = Some(String::new());
        issue.close_reason = Some(" ".to_string());
        let record = emit_issue_record(&issue).unwrap();
        let text = String::from_utf8(record).unwrap();
        assert!(
            !text.contains(":COMPACTED_AT_COMMIT:") && !text.contains(":CLOSE_REASON:"),
            "empty-valued properties must be skipped: {text}"
        );
        let parsed = roundtrip(std::slice::from_ref(&issue));
        assert_eq!(parsed[0].compacted_at_commit, None);
        assert_eq!(parsed[0].close_reason, None);
    }

    /// Quoted conflict markers must stay escaped on the example-block
    /// fallback path too: the safety layer's marker scan reads raw lines
    /// before any Org parsing, so verbatim markers inside a block would
    /// still poison every future import. (Found live: the first corpus
    /// conversion with the fallback produced a file `sync --import-only`
    /// refused as conflicted.)
    #[test]
    fn quoted_conflict_markers_in_fallback_bodies_are_escaped_and_roundtrip() {
        let mut issue = base_issue();
        let desc = "resolve like:\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> feature-branch\nnote trailing blank:\n ";
        issue.description = Some(desc.to_string());

        let record = emit_issue_record(&issue).unwrap();
        let text = String::from_utf8(record).unwrap();
        assert!(
            text.contains("#+begin_example"),
            "trailing-blank line must force the fallback: {text}"
        );
        for line in text.lines() {
            assert!(
                !line.starts_with("<<<<<<<")
                    && !line.starts_with("=======")
                    && !line.starts_with(">>>>>>>"),
                "unescaped conflict marker in fallback emission: {line}"
            );
        }

        let parsed = roundtrip(std::slice::from_ref(&issue));
        assert_eq!(parsed[0].description.as_deref(), Some(desc));
    }

    /// Texts the plain Org layer cannot carry byte-exactly (trailing
    /// whitespace-only lines, leading-whitespace lines, ambiguous list
    /// markers) take the verbatim example-block fallback and are preserved
    /// exactly — a strictly stronger contract than the earlier
    /// normalize-once behavior.
    #[test]
    fn unrepresentable_whitespace_shapes_are_preserved_verbatim() {
        for desc in ["A\n ", "a\n *", "a\n0.\na"] {
            let mut issue = base_issue();
            issue.description = Some(desc.to_string());
            let gen1 = roundtrip(std::slice::from_ref(&issue));
            assert_eq!(gen1[0].description.as_deref(), Some(desc), "gen1: {desc:?}");
            let gen2 = roundtrip(&gen1);
            assert_eq!(gen2[0].description.as_deref(), Some(desc), "gen2: {desc:?}");
        }
    }

    /// The real-corpus drift reproducer: pasted code under a genuine list
    /// bullet gained two indentation spaces per import/flush cycle
    /// (unbounded) through the delegated writer. The stability check must
    /// route such bodies to the example-block fallback: byte-exact from
    /// generation one.
    #[test]
    fn code_under_list_bullet_is_stable_via_fallback() {
        let mut issue = base_issue();
        let desc = "Phase 4 (update_issue at line 388):\n- Inside the mutate() closure:\n  if updates.expect_unassigned {\n  match trimmed {\n      None => { /* unassigned */ }\n        Some(current) if !updates.claim_exclusive => {\n  /* idempotent */\n      }\n  }";
        issue.description = Some(desc.to_string());

        let gen1 = roundtrip(std::slice::from_ref(&issue));
        assert_eq!(gen1[0].description.as_deref(), Some(desc), "gen1 exact");
        let gen2 = roundtrip(&gen1);
        assert_eq!(gen2[0].description.as_deref(), Some(desc), "gen2 exact");
        let gen3 = roundtrip(&gen2);
        assert_eq!(gen3[0].description.as_deref(), Some(desc), "gen3 exact");
    }

    /// Stable structured bodies keep their native Org form — the fallback
    /// only fires when reconstruction is not byte-exact.
    #[test]
    fn stable_structured_bodies_stay_native_org() {
        let mut issue = base_issue();
        issue.description = Some(
            "Intro.\n\n- first\n- second\n\n#+begin_src rust\nfn main() {}\n#+end_src".to_string(),
        );
        let record = emit_issue_record(&issue).unwrap();
        let text = String::from_utf8(record).unwrap();
        assert!(
            !text.contains("#+begin_example"),
            "stable body must not take the fallback: {text}"
        );
    }

    #[test]
    fn empty_issue_set_emits_header_only() {
        let text = issues_to_org_text(&[]).unwrap();
        assert_eq!(text.as_bytes(), org_file_header());
    }

    #[test]
    fn emission_is_deterministic_and_pure() {
        let mut issue = base_issue();
        issue.labels = vec!["b".to_string(), "a".to_string()];
        issue.description = Some("text".to_string());
        let a = emit_issue_record(&issue).unwrap();
        let b = emit_issue_record(&issue).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn export_format_dispatch() {
        assert_eq!(
            ExportFormat::for_path(Path::new("issues.org")),
            ExportFormat::Org
        );
        assert_eq!(
            ExportFormat::for_path(Path::new("issues.ORG")),
            ExportFormat::Org
        );
        assert_eq!(
            ExportFormat::for_path(Path::new("issues.jsonl")),
            ExportFormat::Jsonl
        );
        assert_eq!(
            ExportFormat::for_path(Path::new("issues")),
            ExportFormat::Jsonl
        );
        // Staged temp names resolve to their wire format.
        assert_eq!(
            ExportFormat::for_path(Path::new("issues.org.tmp")),
            ExportFormat::Org
        );
        assert_eq!(
            ExportFormat::for_path(Path::new("issues.org.12345.tmp")),
            ExportFormat::Org
        );
        assert_eq!(
            ExportFormat::for_path(Path::new("issues.jsonl.tmp")),
            ExportFormat::Jsonl
        );
        assert_eq!(
            ExportFormat::for_path(Path::new("issues.jsonl.12345.tmp")),
            ExportFormat::Jsonl
        );
        assert_eq!(ExportFormat::Org.temp_extension(), "org.tmp");
        assert_eq!(ExportFormat::Jsonl.temp_extension(), "jsonl.tmp");
        assert_eq!(ExportFormat::Org.wire_extension(), "org");
        assert_eq!(ExportFormat::Jsonl.wire_extension(), "jsonl");
    }

    #[test]
    fn missing_id_names_heading_ordinal_and_title() {
        let text = "#+TITLE: Obr Issues\n\n* TODO [#C] Good\n:PROPERTIES:\n:ID:       bd-1\n:END:\n\n\
                    * TODO [#C] Broken heading\n:PROPERTIES:\n:OWNER: nobody\n:END:\n";
        let err = org_text_to_issues(text).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("#2"), "must name the ordinal: {msg}");
        assert!(msg.contains("Broken heading"), "must name the title: {msg}");
    }

    #[test]
    fn priority_roundtrip() {
        for i in 0..=4 {
            let mut issue = base_issue();
            issue.priority = Priority(i);
            let parsed = roundtrip(&[issue]);
            assert_eq!(parsed[0].priority, Priority(i));
        }
    }

    #[test]
    fn out_of_range_priority_collapses_to_medium() {
        let mut issue = base_issue();
        issue.priority = Priority(9);
        let parsed = roundtrip(&[issue]);
        assert_eq!(parsed[0].priority, Priority::MEDIUM);
    }

    /// The keyword tables the tokenizer is driven by must be exactly the set
    /// the emitter writes and the parser maps back — that equality is what
    /// makes `keyword_to_status`'s unknown arm unreachable in practice, and
    /// it is the check the deleted "legacy raw status names" compatibility
    /// pretended to provide (those spellings were never in the tables, so a
    /// file using them parsed as Open with the keyword pulled into the
    /// title).
    #[test]
    fn keyword_tables_are_exactly_the_emitted_status_set() {
        let statuses = [
            Status::Open,
            Status::InProgress,
            Status::Blocked,
            Status::Deferred,
            Status::Draft,
            Status::Closed,
            Status::Tombstone,
            Status::Pinned,
        ];
        let mut emitted: Vec<String> = statuses
            .iter()
            .map(|status| {
                let issue = Issue {
                    status: status.clone(),
                    ..base_issue()
                };
                let keyword = status_to_keyword(&issue).expect("standard status");
                assert_eq!(
                    &keyword_to_status(&keyword).expect("keyword maps back"),
                    status,
                    "{keyword} must map back to {status:?}"
                );
                keyword
            })
            .collect();
        emitted.sort();

        let mut tabled: Vec<String> = ORG_TODO_KEYWORDS
            .iter()
            .chain(ORG_DONE_KEYWORDS)
            .map(|k| (*k).to_string())
            .collect();
        tabled.sort();
        assert_eq!(
            emitted, tabled,
            "the tokenizer's keyword tables and the emitted keywords must match"
        );
    }

    #[test]
    fn unknown_keyword_is_refused_by_name() {
        let err = keyword_to_status("IN_PROGRESS").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("IN_PROGRESS"), "must name the keyword: {msg}");
    }

    #[test]
    fn both_schema_version_spellings_are_accepted() {
        for key in ["OBR_SCHEMA_VERSION", "BEADS_SCHEMA_VERSION"] {
            let text = format!(
                "* TODO [#C] T\n:PROPERTIES:\n:{key}: 17\n:ID:       bd-x\n\
                 :CREATED_AT: 2023-11-14T22:13:20+00:00\n\
                 :UPDATED_AT: 2023-11-14T22:13:20+00:00\n:END:\n"
            );
            let parsed = org_text_to_issues(&text).unwrap();
            assert_eq!(parsed[0].id, "bd-x");
        }
    }

    proptest! {
        /// B4: sanitize/unsanitize must be exact inverses on the emitter's
        /// domain, which is canonical body text — the escapers are line-wise
        /// and never see anything else.
        #[test]
        fn sanitize_unsanitize_are_inverses(s in "([^\r\n]{0,40}\n){0,8}[^\r\n]{0,40}") {
            let canon = canonical_body_text(&s);
            prop_assert_eq!(unsanitize_org_text(&sanitize_org_text(&canon)), canon.clone());
        }

        /// Block escape pair used for agent_context; same domain as above.
        #[test]
        fn block_escape_pair_is_symmetric(s in "([^\r\n]{0,40}\n){0,8}[^\r\n]{0,40}") {
            let canon = canonical_body_text(&s);
            prop_assert_eq!(unescape_block_lines(&escape_block_lines(&canon)), canon.clone());
        }

        /// Prose descriptions always round-trip exactly. Lines start with a
        /// letter (digit-initial lines can parse as ordered-list markers,
        /// whose indentation normalizes — see
        /// `list_marker_description_normalizes_once_then_stabilizes`) and
        /// carry no trailing whitespace (normalized away — see
        /// `description_trailing_whitespace_is_normalized`).
        #[test]
        fn plain_description_roundtrips(
            desc in "[a-zA-Z]([a-zA-Z0-9 .,;]{0,57}[a-zA-Z0-9.,;])?(\n[a-zA-Z]([a-zA-Z0-9 .,;]{0,57}[a-zA-Z0-9.,;])?){0,4}"
        ) {
            let mut issue = base_issue();
            issue.description = Some(desc.clone());
            let parsed = roundtrip(&[issue]);
            prop_assert_eq!(parsed[0].description.as_deref(), Some(desc.as_str()));
        }

        /// Titles round-trip regardless of trailing-tag-like shapes.
        #[test]
        fn titles_roundtrip(title in "[a-zA-Z0-9 :_@#%.-]{1,40}") {
            // Heading titles are whitespace-trimmed by any Org parser; only
            // trim-stable titles are representable.
            let title = title.trim().to_string();
            prop_assume!(!title.is_empty());
            let mut issue = base_issue();
            issue.title = title.clone();
            let parsed = roundtrip(&[issue]);
            prop_assert_eq!(&parsed[0].title, &title);
        }

        /// The full-domain round-trip property, over the *un-normalized*
        /// domain: padded titles and property values, bodies with
        /// leading/trailing and interior blank lines, whitespace-only bodies,
        /// reasons that canonicalize from multi-line to single-line,
        /// `Status::Pinned` with `pinned: false`, and labels carrying `:END:`.
        ///
        /// Two assertions, and the first is the product invariant: the file
        /// is a fixpoint from the FIRST write. The second compares fields
        /// against [`normalized_for_org`], the explicit statement of what
        /// the surface normalizes once — so an emitter that fails to apply
        /// one of those normalizations fails the test instead of being
        /// hand-fixed by the generator.
        #[test]
        fn full_domain_issue_roundtrip(
            status_idx in 0usize..8,
            pinned_flag in proptest::bool::ANY,
            priority in 0i32..5,
            raw_title in " {0,2}[a-zA-Z0-9 ★é:_%.-]{1,24} {0,2}",
            desc_lines in proptest::collection::vec("[a-zA-Z0-9 ,*.:#+_-]{0,30}", 0..4),
            lead_newlines in 0usize..3,
            trail_newlines in 0usize..3,
            label_seeds in proptest::collection::vec("[a-z][a-z0-9:_-]{0,10}", 0..4),
            hostile_label in proptest::bool::ANY,
            assignee in proptest::option::of(" {0,2}[a-z]{0,6} {0,2}"),
            reason in proptest::option::of("[a-zA-Z0-9 ,.]{0,20}(\n[a-zA-Z0-9 ,.]{0,20}){0,2}\n{0,2}"),
            with_dep in proptest::bool::ANY,
            with_comment in proptest::bool::ANY,
            with_ctx in proptest::bool::ANY,
        ) {
            let statuses = [
                Status::Open, Status::InProgress, Status::Blocked, Status::Deferred,
                Status::Draft, Status::Closed, Status::Tombstone, Status::Pinned,
            ];
            // An all-whitespace title is not an issue title in any format;
            // everything else about the padding is the emitter's problem.
            prop_assume!(!raw_title.trim().is_empty());

            let mut labels: Vec<String> = label_seeds;
            if hostile_label {
                // Valid under LabelValidator (alphanumerics and colons), and
                // the exact shape the drawer sanitizer used to mangle.
                labels.push("a:END:b".to_string());
            }
            labels.sort();
            labels.dedup();

            let mut issue = base_issue();
            issue.status = statuses[status_idx].clone();
            issue.pinned = pinned_flag;
            issue.priority = Priority(priority);
            issue.title = raw_title;
            issue.labels = labels;
            issue.assignee = assignee;
            issue.close_reason = reason.clone();
            issue.delete_reason = reason;

            let body = format!(
                "{}{}{}",
                "\n".repeat(lead_newlines),
                desc_lines.join("\n"),
                "\n".repeat(trail_newlines),
            );
            if !body.is_empty() {
                issue.description = Some(body.clone());
                issue.notes = Some(body);
            }
            if with_dep {
                issue.dependencies = vec![Dependency {
                    issue_id: "bd-test".to_string(),
                    depends_on_id: "bd-other".to_string(),
                    dep_type: DependencyType::Blocks,
                    created_at: ts(1_700_000_000),
                    created_by: None,
                    metadata: None,
                    thread_id: None,
                }];
            }
            if with_comment {
                issue.comments = vec![Comment {
                    id: 1,
                    issue_id: "bd-test".to_string(),
                    author: "prop".to_string(),
                    body: "multi\nline *comment*".to_string(),
                    created_at: ts(1_700_000_050),
                }];
            }
            if with_ctx {
                issue.agent_context =
                    Some("{\n  \"skills\": [\"rust\", \"org\"]\n}\n\n".to_string());
            }

            // 1. The file is a fixpoint from the first write.
            let gen1 = issues_to_org_text(std::slice::from_ref(&issue)).expect("emit gen1");
            let parsed = org_text_to_issues(&gen1).expect("parse gen1");
            let gen2 = issues_to_org_text(&parsed).expect("emit gen2");
            prop_assert_eq!(&gen1, &gen2);

            // 2. The stored record is the input under exactly one documented
            //    normalization step.
            let expected = normalized_for_org(&issue);
            let got = &parsed[0];
            prop_assert_eq!(&got.status, &expected.status);
            prop_assert_eq!(got.priority, expected.priority);
            prop_assert_eq!(got.pinned, expected.pinned);
            prop_assert_eq!(&got.title, &expected.title);
            prop_assert_eq!(&got.description, &expected.description);
            prop_assert_eq!(&got.notes, &expected.notes);
            prop_assert_eq!(&got.assignee, &expected.assignee);
            prop_assert_eq!(&got.close_reason, &expected.close_reason);
            prop_assert_eq!(&got.delete_reason, &expected.delete_reason);
            prop_assert_eq!(&got.labels, &expected.labels);
            prop_assert_eq!(&got.dependencies, &expected.dependencies);
            prop_assert_eq!(&got.comments, &expected.comments);
            prop_assert_eq!(&got.agent_context, &expected.agent_context);
        }

        /// Labels (including colon-bearing ones) round-trip via :LABELS:.
        #[test]
        fn labels_roundtrip(labels in proptest::collection::vec("[a-z][a-z0-9:_-]{0,15}", 0..5)) {
            let mut sorted: Vec<String> = labels.clone();
            sorted.sort();
            sorted.dedup();
            let mut issue = base_issue();
            issue.labels = sorted.clone();
            let parsed = roundtrip(&[issue]);
            prop_assert_eq!(&parsed[0].labels, &sorted);
        }
    }
}
