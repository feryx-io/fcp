# Archived status documents

These documents are **historical snapshots**, kept for reference. They describe the
codebase as it stood in December 2025 – January 2026 and are **contradicted by the
current code** — do not plan work from them.

For current status, see [`STATUS.md`](../../STATUS.md) at the repository root.

| Document | Snapshot date | Why archived |
|---|---|---|
| `PROGRESS_AND_NEXT_STEPS.md` | 2026-01-09 | Claims "60–70% of Phase 1"; its gap table marks crates as "not started" that have long since shipped |
| `PROGRESS_ASSESSMENT.md` | 2026-01-23 | Claims "~65% through Phase 1", "no Dockerfile", "no README" — all long outdated |
| `ARCHITECTURE_FIXES_PLAN.md` | 2026-01-21 | All three bugs it plans fixes for were fixed shortly after (commit b1abc10) |
| `TOMORROW_PLAN.md` / `TOMORROW_PLAN_ORIGINAL.md` | Jan 2026 | Day plans whose deliverables (voicemail crate, extension targets) shipped months ago |
| `AUDIT_AND_TEST_PLAN.md` | early 2026 | Its blocking bugs (hangup BYE, voicemail playback, job handlers) are fixed in current code; its test-gap matrix predates later work. Its live-test phase plan (A–I) may still be useful as a template |
| `INTEGRATION_STATUS.md` | 2025-12-24 | Describes a mid-refactor `fcp-media` whose compile errors were resolved; the media integration has since shipped |
