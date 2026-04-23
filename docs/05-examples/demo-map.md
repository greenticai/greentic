Status: Operational guidance in this repo
Scope: How to use greentic-demo and other demo material safely from this repo
Implementation owner: Mixed ownership; this doc is only the repo-local usage policy

# Demo Map

`greentic-demo` exists and is useful, but it is not the default source of truth
for this repository.

## What `greentic-demo` Is Good For

Use `greentic-demo` for:

- seeing end-to-end sample digital-worker compositions
- understanding product-level scenarios
- finding inspiration for bundle structure or answers-file shape
- discovering candidate patterns worth checking against current repo docs

## What Is Safe To Copy Conceptually

It is usually safe to copy the high-level idea of:

- a user journey
- a demo scenario
- a rough pack/bundle composition pattern
- the fact that a certain kind of worker is possible in Greentic

## What Must Always Be Re-Validated

Always re-check these against this repo’s current docs, schemas, and code before
you copy or document them:

- CLI syntax
- `answers.json` keys or structure
- setup/start behavior
- extension and deployer assumptions
- current terminology
- any path, filename, or generated artifact contract

## Practical Rule

Treat demo repos as inspiration-first.

That means:

1. start with the current canonical docs in this repo
2. use demos to understand the scenario, not to prove the syntax
3. re-validate against current code and schema output before editing docs or implementation here

## What Not To Do

Do not:

- copy a demo command into canonical docs without re-checking it
- assume old answers files are still valid
- treat a screenshot or README snippet as stronger evidence than current repo code
- use demos to overrule a current repo-owned schema or canonical doc
