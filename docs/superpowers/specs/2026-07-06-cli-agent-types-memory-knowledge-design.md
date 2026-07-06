# CLI Agent Types + Memory + Knowledge (target: gtc 1.1.2)

**Status:** Design — ready for review
**Date:** 2026-07-06
**Author:** Bima Pangestu (with Claude)
**Repos touched:** `greentic` (gtc CLI), `greentic-dw` (authoring wizard + new shared crate), `greentic-designer` (refactor onto shared crate), `greentic-runner` (feature passthrough + release variant)
**Base:** `greentic` `origin/main` @ `b6f390d` (v1.1.0)

---

## 1. Problem & Goal

Today the three agentic-worker types — **single_turn**, **agent_graph**, **deep_worker** — plus agent **memory** and **knowledge/RAG** can only be authored in the **web designer/composer**. The `gtc` CLI has **zero** agent-type / memory / knowledge surface (verified: `gtc` v1.1.0 source has no `AgentKind`, no memory/knowledge command; `gtc wizard` just launches `greentic-pack`/`greentic-bundle` wizards).

**Goal for 1.1.2:** let a user create, configure, and build all three worker types — including memory and knowledge — entirely from the CLI, producing a runner-loadable `.gtpack`, with real runtime retrieval available on a shipped runner build.

**Non-goal (deferred to Slice 3 / 1.1.3):** knowledge/RAG on the **deep_worker** execution path (greentic-dw deep-loop has no retrieval wiring yet) and `greentic-dw serve` HTTP ingress.

### Success criteria
- `gtc worker new` (interactive) and `gtc worker build <spec>` (declarative) both produce a runner-loadable `.gtpack` for each of the 3 kinds.
- The `.gtpack` is **structurally equivalent** to what the designer emits (same `manifest.cbor` + `dw-agents.json` + `flows/main.ygtc` executing-node shape).
- A worker declaring memory/knowledge, run via `gtc start` against the shipped knowledge-enabled runner + embedding env, retrieves from its corpus (single_turn / agent_graph).
- Designer continues to emit identical packs after being refactored onto the shared crate (parity is enforced by test, not by hand).

---

## 2. Verified current state (grounding)

| Fact | Evidence |
|---|---|
| `gtc` = v1.1.0 on `origin/main`; no agent/memory/knowledge CLI surface | `greentic/Cargo.toml`; full-source search (zero hits) |
| `gtc wizard` → `greentic-dev wizard` (a launcher delegating to `greentic-pack`/`greentic-bundle`); `gtc dw publish/install` = store client in greentic-dev (no build) | `greentic/src/bin/gtc/router.rs:59-68`; `greentic-dev/src/dw_cmd.rs`, `src/wizard/mod.rs` |
| Full DW pack-assembly (`AgentKind → AnswerDocument → .gtpack`) lives **only** inside `greentic-designer/src/orchestrate/`; never extracted to a shared crate (code comments anticipate promoting to `greentic-pack-lib`) | `greentic-designer/src/orchestrate/{dw_form.rs,dw_form_to_answer_doc.rs,dw_pack.rs,runner_sidecar/loadable.rs,dw_application_pack.rs}` |
| Canonical 3-way enum `AgentKind{SingleTurn,AgentGraph,DeepWorker}` is designer-only | `greentic-designer/src/orchestrate/dw_form.rs:17-22` |
| Runtime config the runner consumes: `AgentConfig`, `MemorySettings`, `KnowledgeSettings` | `greentic-runner/crates/greentic-aw-runtime/src/config.rs:96,48,68` |
| Sidecar pack-loadable gap **RESOLVED**: designer now emits `manifest.cbor` (via `greentic_pack::builder::PackBuilder`) + `dw-agents.json` + inline flow | `greentic-designer` research @ `25ef8c06`, PR #858; `runner_sidecar/materialize.rs`, `dw_pack.rs:29,157` |
| Deployed runner is built with default features `["verify","agentic-worker"]` only; `knowledge-chronicle` / `long-term-chronicle` are opt-in and **not surfaced on the binary crate**; release workflow passes no `--features` | `greentic-runner/crates/greentic-runner/Cargo.toml:100`; `.github/workflows/release-binaries.yml`, `ci.yml:24-33` |
| Retrieval code path is live but silently disabled without feature+env (`GREENTIC_KNOWLEDGE_EMBED_*`) | `greentic-aw-runtime/src/loop.rs:199-213`; `greentic-runner-host/src/runner/knowledge_mount.rs` |
| Short-term working memory = always-on in-memory provider; but dw.agent needs a state store (server: `GREENTIC_AW_REDIS_URL`; else `desktop-agent-ephemeral` feature) | `agent_node.rs:1087,899,916,954` |
| deep_worker path has **no** knowledge wiring; `greentic-dw serve` on main lacks `--port`/`/healthz` | `greentic-dw/crates/greentic-dw-runtime/src/deep_loop.rs`; `greentic-dw-cli/src` (no serve on main) |

**Consequence:** "expose memory+knowledge via CLI with real retrieval" is not merely a surfacing job — it requires **productionizing the runner build** (Slice 2). Short-term memory is essentially surfacing-only.

---

## 3. Approach (decided: Approach A)

**Extract the designer's authoring/assembly pipeline into a new shared crate; `greentic-dw` owns the CLI wizard; `gtc` routes to it.** This is the reuse-first, anti-divergence choice: it gives the three agent types a single canonical home and keeps the designer and CLI in lockstep, paying down the current 3-divergent-models debt (designer `DwFormState` vs runner `AgentConfig` vs greentic-dw `DigitalWorkerManifest`).

Rejected alternatives:
- **B** — build the CLI on greentic-dw's existing `DigitalWorkerManifest`: risks a permanent third model and re-implementing designer graph logic → divergence.
- **C** — thin CLI straight onto runtime `AgentConfig` + `PackBuilder`: smallest surface but re-derives agent_graph construction outside the designer → duplication.

---

## 4. Slice 1 — CLI authoring

### 4.1 New shared crate `greentic-dw-authoring`
Home for the code extracted (mechanically, behavior-preserving) from `greentic-designer/src/orchestrate/`:
- **Type model:** `AgentKind{SingleTurn,AgentGraph,DeepWorker}` + the worker config structs (`MemorySection`, `KnowledgeBinding`, `DeepWorkerConfig`, `ProviderBinding`, tool/guardrail refs). These move here (or into `greentic-dw-types`); designer re-exports/adapts.
- **Projection:** `WorkerSpec → AnswerDocument` (the existing `dw_form_to_answer_doc` logic, generalized off `DwFormState`).
- **Assembly:** `AnswerDocument → .gtpack` — `dw_pack.rs` (`greentic_pack::builder::{PackBuilder,PackMeta,FlowBundle}` → `manifest.cbor`), `loadable.rs` (synthesize `flows/main.ygtc` + inline compiled flow), `embed_dw_agents` (`dw-agents.json`), knowledge corpus baking.
- **Executing-node injection:** `dw.agent` (single_turn), `dw.agent_graph` (agent_graph), `operala.call` (deep_worker) — reuse `inject_dw_agent_graph_node` / `inject_operala_call_node`.
- **Validation:** spec validation + local PDF/text extraction (reuse designer's `extract` + `lopdf`).

**Extraction risk mitigation:** only the relatively pure modules move (`dw_pack`, `loadable`, `dw_form_to_answer_doc`, `embed_dw_agents` already use `greentic_pack::builder` in-process and are projection-shaped). Web/DB glue (session persistence, HTTP routes, knowledge-library DB) **stays** in the designer; the designer calls the shared crate with an in-memory `WorkerSpec` built from its `DwFormState`. Designer refactor + CLI land in one coordinated release.

### 4.2 `WorkerSpec` — the single authoring format (surfaces A **and** B)
The interactive wizard and the declarative file path produce/consume the **same** `WorkerSpec`. The wizard is an interactive front-end that emits a `WorkerSpec`, then builds it.

```yaml
apiVersion: greentic.dev/v1
kind: single_turn            # | agent_graph | deep_worker
name: support-triage
llm:      { provider: openai, model: gpt-4o, credential_ref: llm-openai }
instructions: |              # system prompt
  You are a support triage agent...
tools:    [ web.search, hubspot.contacts ]
memory:
  short_term: { enabled: true }
  long_term:  { provider: chronicle, credential_ref: chronicle-main }
knowledge:                   # RAG (auto pre-retrieval)
  provider: chronicle
  embedding: { provider: openai, model: text-embedding-3-small, credential_ref: embed-openai }
  top_k: 5
  documents: [ ./kb/policies.pdf, ./kb/faq.md ]
guardrails: [ pii-redact ]

# type-specific block (only the one matching `kind`):
agent_graph:
  coordinator: { instructions: "route to the right specialist" }
  specialists:
    - { name: billing, instructions: "...", tools: [stripe.lookup] }
    - { name: tech,     instructions: "...", tools: [docs.search]  }
deep_worker:
  iteration_budget: 8
  reflection: true
  planning_model: gpt-4o
```

Design note: the agent_graph shape is **coordinator + specialists** (the proven "form-first" designer model, PR #861), not an arbitrary canvas — this keeps CLI graph authoring tractable while staying faithful to the runtime (supervisor routing + shared blackboard).

### 4.3 CLI verbs — new `gtc worker` tree → `greentic-dw`
`gtc` gains a pass-through verb `worker` routed to the `greentic-dw` binary (add `DW_BIN` in `router.rs`, mirroring `gtc setup`→`greentic-setup`). Not overloaded onto `gtc dw` (store publish/install, greentic-dev) or `gtc wizard` (pack/bundle launcher).

| Command | Behavior |
|---|---|
| `gtc worker init <single_turn\|agent_graph\|deep_worker>` | scaffold a starter `WorkerSpec` YAML for that kind |
| `gtc worker new` | interactive wizard (surface **A**) → emit `WorkerSpec` + build `.gtpack` |
| `gtc worker build <spec.yaml> [-o out.gtpack]` | declarative build (surface **B**) |
| `gtc worker validate <spec.yaml>` | validate without building |

`greentic-dw` implements `worker {init,new,build,validate}` on the shared crate. The wizard reuses greentic-dw's existing answers/schema-driven pattern (`--answers` for non-interactive/testable runs).

### 4.4 Data flow
```
gtc worker new/build
  → WorkerSpec (YAML/JSON, on disk)
    → greentic_dw_authoring: WorkerSpec → AnswerDocument
      → assembly: manifest.cbor + dw-agents.json + flows/main.ygtc(executing-node) + knowledge corpus
        → <name>.gtpack   (runner-loadable; verified via greentic_types::decode_pack_manifest)
```
Knowledge documents are read locally, text-extracted + chunked, baked into the pack corpus — the CLI analog of the designer knowledge library.

### 4.5 Error handling (anyhow/thiserror, no panics)
Explicit, non-silent failures: unknown `kind`; missing `llm`; `agent_graph` requires ≥2 specialists + consistent branch labels; `deep_worker.iteration_budget` within bounds; **knowledge document path not found → hard error, not silent skip**; unresolved `credential_ref` → warning at build, hard error at run. Reuse validation extracted from the designer.

---

## 5. Slice 2 — Runtime productionization

Makes real retrieval work on a **shipped** runner. Prerequisite (assumed sorted): CI/release access to the cross-org private `greentic-biz/greentic-chronicle-ext` deps.

### 5.1 Binary-crate feature passthrough
`greentic-runner/crates/greentic-runner/Cargo.toml` (mirror existing `verify`/`telemetry` passthroughs):
```toml
knowledge-chronicle     = ["greentic-runner-host/knowledge-chronicle"]
long-term-chronicle     = ["greentic-runner-host/long-term-chronicle"]
desktop-agent-ephemeral = ["greentic-runner-host/desktop-agent-ephemeral"]
```
+ `#![recursion_limit = "512"]` in `src/main.rs` (Chronicle+SurrealDB futures exceed the default 128 — proven during S0 investigation). Build env needs `clang` for RocksDB.

### 5.2 `runner-full` release artifact
Add a release build variant `greentic-runner-full` built with `knowledge-chronicle,long-term-chronicle`. The default `greentic-runner` stays `verify,agentic-worker` (byte-identical, backward-compatible). Two shipped artifacts; the knowledge-enabled one is opt-in for operators who need memory/RAG.

### 5.3 `gtc` env surfacing + guidance
- `gtc start`: when the pack **declares** `memory.long_term` or `knowledge`, check required env (`GREENTIC_KNOWLEDGE_EMBED_{BASE_URL,API_KEY,MODEL}`, `GREENTIC_CHRONICLE_*`, `GREENTIC_AW_REDIS_URL`); if the running binary lacks the feature or env is missing, emit a **clear warning + pointer to `runner-full`** instead of a silent no-op RAG.
- `gtc doctor`: add a "knowledge/memory readiness" check (feature detectable? env set?).
- `gtc setup`: option to write embedding/chronicle env into the bundle config.

### 5.4 Short-term memory
Already works on stock (in-memory provider). dw.agent needs a state store: server → Redis; local `gtc start` → `desktop-agent-ephemeral` (no-infra). `gtc start` surfaces the choice.

---

## 6. Testing

**Slice 1**
- `greentic-dw-authoring` unit tests: `WorkerSpec → AnswerDocument → .gtpack` per kind; assert `manifest.cbor` decodes via `greentic_types::decode_pack_manifest`, `dw-agents.json` present, knowledge corpus baked (mirror designer sidecar tests). Snapshot spec→pack.
- **Parity test:** a `WorkerSpec` equivalent to a designer `DwFormState` produces a structurally-equivalent pack — the guard that keeps A from diverging.
- greentic-dw CLI tests: `worker init/validate/build` on valid + invalid specs (error messages); wizard via `--answers` (non-interactive).

**Slice 2**
- CI-full build check: `cargo build -p greentic-runner --features knowledge-chronicle,long-term-chronicle`.
- `gtc doctor` / `gtc start` env-gating tests: pack declares knowledge without env → warning + non-zero/handled exit.

**Live-verify (manual, external creds — final Slice 2 gate)**
`gtc worker new` a knowledge worker → `gtc start` with `runner-full` + embedding key → ask a corpus-only question → grounded answer (single_turn / agent_graph).

---

## 7. Risks & honest caveats

1. **Designer extraction** is the biggest Slice 1 risk (`orchestrate/` is entangled with web/DB). Mitigation: extract only the pure projection/assembly modules; leave glue in designer; enforce parity by test. Designer + CLI ship in one coordinated release.
2. **agent_graph authoring in CLI** is complex (supervisor/parallel/routes). Mitigation: MVP = coordinator+specialists form (proven), reuse extracted graph logic; arbitrary canvas stays designer-only.
3. **Slice 2 cross-org access** is an org/ops prerequisite (assumed sorted). If it slips, Slice 1 still ships independently (authoring parity); retrieval lights up once `runner-full` deploys.
4. **deep_worker knowledge gap:** deep_worker workers are authorable + runnable via CLI in 1.1.2, but knowledge auto-retrieval is only effective on single_turn/agent_graph until Slice 3. **Must be documented** so users aren't misled.

---

## 8. Decomposition & sequencing

- **Slice 1** (CLI authoring) — independent, ships the headline feature. Order: (1a) extract shared crate + refactor designer onto it with parity test; (1b) `greentic-dw worker {init,new,build,validate}`; (1c) `gtc worker` routing.
- **Slice 2** (runtime productionization) — feature passthrough + `runner-full` artifact + `gtc` env surfacing; gated by cross-org access + live-verify.
- **Slice 3** (deferred, 1.1.3) — deep_worker RAG + `greentic-dw serve` HTTP.

Each slice gets its own implementation plan.

---

## 9. Docs (docs.greentic.ai, parallel deliverable)

Net-new pages in `greentic-docs` (Astro Starlight, English + 7 locales + `astro.config.mjs` sidebar):
- **Agent types** — single_turn / agent_graph / deep_worker: what each is, when to use, how the pack differs.
- **Agent memory** — short-term (working) vs long-term (Chronicle); env + build requirements.
- **Knowledge / RAG** — attaching a corpus, embedding provider, top_k; the `runner-full` requirement; the deep_worker caveat (§7.4).
- Update `concepts/agentic-workers.mdx` + `cli/*` to reference `gtc worker` (mark "1.1.2").

Docs describe **shipped/verified behavior**; anything 1.1.2-gated is labeled as such.
