---
name: sdd
description: "Spec-Driven Development workflow for significant development efforts in Pitwall: where the spec lives, what it contains, and how the work is tracked in plan.md checkboxes"
targets: ["*"]
---

# Spec-Driven Development

For significant development efforts, we use spec-driven development (SDD) to guide the work.

It is fine to skip SDD for small, simple changes.

## Feature Directory

The specs and related artifacts for each development effort live in a subdirectory of `docs/spec/` called the
"feature directory". New feature directories are named `<YYYY>-<MM>-<DD>-<name>` (the start date), so their age and
order are easy to see. Older specs are single files in `docs/spec/` (for example `editor.md`); leave them where they
are, and turn one into a feature directory only when a new effort grows it. `docs/spec/tile-sdk/` predates this
naming and keeps its name.

## Artifacts

Within the feature directory, we store the following artifacts:

- `frd.md` (Functional Requirements Document): the functional specification. Focus on what users and authors need
  (requirements, user stories, personas) and avoid technical detail unless it is fundamental to the requirements.
- `hla.md` (High-Level Architecture): the overall topology, components, major interfaces, crates involved and how the
  work fits Pitwall's architecture (`docs/spec/architecture.md`). No low-level detail such as code samples or exact
  versions unless it is critical to the architecture. Pseudocode for critical algorithms is allowed.
- `plan.md`: the detailed technical plan, with checkboxes for tracking the work as it is completed. Every item has a
  definition of done that can be checked objectively. If phasing is appropriate, describe the phases here, in one plan
  file or several. **This is where progress is tracked**: tick a box in the same pull request that finishes the item,
  and link that pull request next to it.

A spec that was written before this skill (such as `docs/spec/tile-sdk/`, whose `README.md` holds the requirements and
the architecture) doesn't need splitting into `frd.md` and `hla.md`; give it a `plan.md` and keep the rest.

Most efforts involve one or more low-level design (LLD) documents with more detailed specifications for specific
components, algorithms or interfaces. Store them in the feature directory with descriptive names (e.g.
`something-lld.md`). As a rule, the plan includes writing any missing LLD documents as part of the work, and links to
them.

Other artifacts (background research, UI concepts, API specifications, data models, diagrams, measurements) may be
stored in the feature directory as needed.

## Tracking progress

- `plan.md` is the single place for progress. Don't keep a second tracker elsewhere.
- Each checkbox item names its definition of done. Tick it only when that holds on `main`, and add the pull request
  number: `- [x] WIT 0.1 compiles and a hello tile renders (#123)`.
- Agents working in parallel each tick only their own items, so concurrent edits rarely touch the same lines.
- When a pull request changes what was planned, update the plan in the same pull request (add, split or strike items,
  with a short reason).
- Pitwall's rule that a feature is done only when its website docs match applies here too: a phase's docs items are
  part of its checklist.

## Branching Model

Work driven via SDD is done in one or more feature branches. The general pattern is:

1. Create an initial feature branch, named after the feature directory (extra info such as a phase is fine).
2. Create the feature directory and its artifacts in this branch.
3. If review is needed before implementation, open a draft pull request so the specs can be reviewed.
4. The first push of work uses that existing branch.
5. The SDD artifacts are merged together with the work itself.
6. Further work per the specs is done in additional feature branches, tracked in the existing plan files. Changing the
   artifacts when requirements, architecture or plan change is encouraged.
7. If later work supersedes unfinished work in an existing feature directory, update that directory's specs to say the
   remaining work is superseded and by what.

Pitwall's usual rules apply to every branch: Conventional Commits, no `cargo fmt`, and pull requests merged to `main`
when CI passes.
