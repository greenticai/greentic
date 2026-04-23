Status: Canonical in this repo
Scope: Validated structured examples referenced by canonical docs
Implementation owner: Repo-local documentation tooling and validation

# Validated Examples

This folder contains the small set of structured examples that are treated as
canonical and automatically validated in this repo.

## Current Coverage

- [`wizard-launcher-minimal.answers.json`](./wizard-launcher-minimal.answers.json)
  Smallest validated launcher-style wizard answers document for the current
  generated wizard schema.
- [`wizard-launcher-bundle.answers.json`](./wizard-launcher-bundle.answers.json)
  Validated launcher-style example that sets `answers.selected_action` to
  `bundle`.

## Scope Rule

Only add examples here when both of these are true:

- the example is referenced by canonical docs
- the repo can validate it automatically against current generated schema or a
  comparably strong local contract

Examples that are merely illustrative but not yet automatically validated
should stay out of this folder.
