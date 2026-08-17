# jsonl_surface_symlink

- **Finding**: `fm-state_files-jsonl-crlf-line-endings` (D-SURFACE
  symlink subset) — the workspace-root surface `PLAN.org` is a symlink
  to `store/PLAN.org`, and the CRLF defect lives in the target.
- **Why it exists** (obr-4y0): six content/mode detectors —
  `jsonl_bom`, `jsonl_crlf`, `jsonl_eof_newline`, `jsonl_size`,
  `jsonl.duplicate_ids`, `permissions.jsonl_world_writable` — opened
  with `symlink_metadata(...).is_symlink() -> push ok`. That guard was
  written when the export could only live inside `.obr/`. D-SURFACE
  moved the export to a workspace-root path that
  `sync::path::is_workspace_surface_path` explicitly permits to be a
  symlink, so on any symlinked surface all six had exactly one possible
  answer regardless of the file's real bytes or mode. A symlink carries
  neither content nor mode; the detectors resolve it and inspect the
  target.
- **Keeps the default surface**: this fixture carries `.no_jsonl_pin`,
  so the harness does NOT repin the export to `.obr/issues.jsonl`. The
  whole point is the post-D-SURFACE shape.
- **Detect contract**: `jsonl_crlf` fires `warn` with
  `details.path` = the resolved target, `details.surface_path` = the
  configured surface, and `details.resolved_via_symlink == true`. The
  detectors with nothing to report (`jsonl_bom`, `jsonl_size`,
  `permissions.jsonl_world_writable`) must stay `ok` — following the
  link must not turn every check into a warning.
- **Repair contract**: `--repair` rewrites the TARGET's bytes and
  leaves the operator's symlink in place. Writing at the link path
  instead would either replace the link with a regular file or push
  bytes through it.
- **Undo contract**: `doctor undo` byte-restores the CRLF target; the
  symlink is untouched throughout.
