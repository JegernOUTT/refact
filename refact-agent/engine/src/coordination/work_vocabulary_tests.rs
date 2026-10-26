//! One vocabulary across the tool, HTTP and GUI surfaces (M10).
//!
//! The work projection names its states, freshness and disclosure once, in
//! `coordination::types`. Every other surface is a copy, and a copy drifts:
//! the GUI omitted `closed` and rejected the whole page for it, and the
//! `work_overview` schema omitted it and refused to filter on it.
//!
//! These tests pin each copy back to the enum. The tool schema is now
//! generated from `WorkStateTag::ALL`, so it cannot drift; the GUI contract is
//! a separate language and is pinned by reading the file.
//!
//! Surfaces that deliberately carry no work vocabulary, and so are not pinned
//! here: the TUI renders a single chat session and never lists project work,
//! so it has no state/freshness/disclosure of its own to keep in step. It
//! names its own `SessionState`, which the catalog already maps into this
//! vocabulary, so pinning it would assert a synonym rather than forbid one.
//!
//! The six evidence verdicts are pinned here too. They had a GUI surface all
//! along — the goal receipt card — but it re-derived the verdict by matching
//! words in the engine's reason string, so rewording a sentence changed what
//! the user was told happened. The verdict now crosses as a value and is
//! pinned to `EvidenceVerdict::ALL` below.
//!
//! Concepts that reach the tool and HTTP surfaces but have no GUI panel yet —
//! merge state and dirty-baseline refusal, `done` vs `finalized`,
//! recovery/reconciliation actions, handoff-brief signal and disagreement
//! status. They are omitted here on purpose: each belongs to the panel of its
//! own milestone, and inventing a second work-shaped list for them is the
//! parallel dashboard M10 exists to avoid.
