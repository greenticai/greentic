Status: Canonical in this repo
Scope: Repo-local happy path for moving from authoring into setup/start
Implementation owner: Mixed ownership; this doc is canonical for the repo-local workflow framing

# Happy Path: Build An App

This is the simplest current repo-local authoring path to think with.

It is intentionally high-level and decision-supportive. When exact structured
fields matter, inspect the current schema first.

## 1. Create Or Prepare The Component

Start with the smallest reusable unit of work.

Use a component when you need one concrete behavior such as:

- calling a service
- transforming data
- templating output
- rendering a card

If you are still trying to decide sequencing or branching, you are probably not
done with the component/flow split yet.

## 2. Inspect The Current Schema

Before writing answers by hand, inspect the current wizard schema:

```bash
gtc wizard --schema
```

If the current work depends on downstream wizard structure, treat the schema as
the strongest available truth.

## 3. Create Or Update The Flow

Once the components are clear, define how they connect.

Use a flow for:

- sequencing
- branching
- state transitions
- deterministic routing

## 4. Add Or Update Steps

Refine the flow with the steps it actually needs.

Do not invent step structure from memory. Use current schema and current flow
tooling expectations when those are available.

## 5. Create Or Update The Pack

Package the application logic into the right distribution unit.

Use an application pack when you are packaging business-facing logic. Use an
extension pack when you are packaging cross-cutting capability.

## 6. Create Or Update The Bundle

Assemble the runnable system:

- application pack(s)
- extension pack(s)
- bundle-level composition needed for setup and start

## 7. Run Setup

Prepare the bundle for execution:

```bash
gtc setup ./some-bundle --answers <setup-answers.json>
```

Current repo-local docs treat setup as preparation, not execution.

## 8. Run Start

Launch the prepared bundle:

```bash
gtc start ./some-bundle
```

If you need a specific deployment target, pass `--target` explicitly rather
than assuming an interactive or inferred default will be available.

## Working Rule

At every stage:

- check schema first when it exists
- treat demos as inspiration, not syntax authority
- keep setup and start distinct from authoring
