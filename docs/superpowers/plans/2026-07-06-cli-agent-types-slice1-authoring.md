# CLI Agent Types — Slice 1 (Authoring) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user author all three agentic-worker types (`single_turn`, `agent_graph`, `deep_worker`) — including memory and knowledge — entirely from the CLI (`gtc worker`), producing a runner-loadable `.gtpack` identical to the Designer's.

**Architecture:** Extract the Designer's pure pack-assembly logic into a new standalone crate `greentic-dw-authoring` whose assembly is **DB-free** (knowledge text supplied by the caller, not read from sqlite). A canonical `WorkerSpec` (YAML/JSON) is the single authoring format; the Designer and the CLI both project their state into it. `greentic-dw` gains a `worker` subcommand built on the crate; `gtc worker` passes through to `greentic-dw`. The Designer is refactored onto the crate with a parity test guaranteeing it emits the same pack as before.

**Tech Stack:** Rust (edition 2021, rustc 1.95.0), `serde`/`serde_json`/`serde_yaml_bw`, `greentic-pack-lib` (imported as `greentic_pack`), `greentic-types`, `greentic-aw-runtime` (`AgentConfig`), `clap`, `schemars` (schema), `anyhow`/`thiserror`.

## Global Constraints

- Rust toolchain **1.95.0** pinned via `rust-toolchain.toml` (do not edit).
- `#![forbid(unsafe_code)]` at the new crate root.
- **No `unwrap()` / `panic!()` / `expect()` in non-test code** — use `anyhow`/`thiserror`.
- Conventional Commits (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`).
- English only in source, tests, comments, tracing.
- The new crate's `greentic-*` deps must match the versions the Designer already pins (verbatim): `greentic-types = "=1.2.0-research.1"`, `greentic-pack-lib = "=1.2.0-research.0"` (lib name `greentic_pack`), `greentic-flow = "=1.2.0-research.0"`, `greentic-dw-types = "1.1.0-dev"`, `greentic-aw-runtime` at the Designer's git rev (`greentic-designer/Cargo.toml:73`).
- The emitted `.gtpack` MUST be runner-loadable: a `manifest.cbor` decodable via `greentic_types::decode_pack_manifest`, plus `dw-agents.json`, plus `flows/main.ygtc`.
- `greentic-dev` stays a pass-through: **no** agent semantics land there. Agent authoring lives in `greentic-dw` + `greentic-dw-authoring`.
- `git commit` in each repo's own worktree; never bypass hooks with `--no-verify`.

## Design decisions locked by code-grounding (read before Task 1)

1. **Standalone crate, not a workspace member.** `greentic-designer` is a single crate (no `[workspace]`). Create `greentic-dw-authoring/` as a sibling directory with its own `Cargo.toml`; the Designer adds it as `path = "../greentic-dw-authoring"`.
2. **DB-free assembly seam.** The Designer's real workhorse `dw_application_pack::build_dw_pack_from_answer_document` is DB-bound (reads `knowledge_documents` from sqlite). The crate must NOT depend on sqlx. The crate's assembly takes knowledge as already-extracted text: `Vec<KnowledgeInput { id: String, text: String }>`. The Designer supplies these from its DB; the CLI supplies them by reading local files.
3. **One canonical `WorkerSpec`.** Neither the Designer's untyped `serde_json::Value` AnswerDocument nor greentic-dw's typed `AnswerDocument` is reused as the authoring format. The crate owns `WorkerSpec` (a typed struct, `schemars::JsonSchema`), and an internal `WorkerSpec → serde_json::Value` projection that reproduces the shape `dw_form_to_answer_doc::convert` produces today.
4. **Port injector fns individually.** `pack_via_packc/mod.rs` is coupled to Designer internals, but `embed_dw_agents`, `inject_dw_agent_graph_node`, `inject_operala_call_node`, `inject_dw_agent_nodes` each only need `serde_json`/`serde_yaml_bw`/`cbor_flow_post`/`greentic_aw_runtime`. Move those four fns (+ `DwAgentInjection`) into the crate, not the whole module.
5. **Port `cbor_flow_post` + `loadable` + `slugify`** into the crate (they are pure and required by the assembly).
6. **Executing-node mapping** (from `dw_form_to_answer_doc.rs:131-142`): `AgentGraph → {"kind":"dw.agent_graph"}`, `DeepWorker → {"kind":"operala.call","deep_worker":<config>}`, `SingleTurn → none` (single_turn flow is synthesized by `single_turn_main_ygtc`).

---

## Task Group 1 — The `greentic-dw-authoring` crate

### Task 1: Scaffold the crate

**Files:**
- Create: `greentic-dw-authoring/Cargo.toml`
- Create: `greentic-dw-authoring/src/lib.rs`
- Create: `greentic-dw-authoring/rust-toolchain.toml` (copy from a sibling repo, pin 1.95.0)

**Interfaces:**
- Produces: crate `greentic_dw_authoring` compiling with the pinned deps.

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "greentic-dw-authoring"
version = "0.1.0"
edition = "2021"
rust-version = "1.95.0"
description = "Author agentic-worker packs (single_turn / agent_graph / deep_worker) from a WorkerSpec"
license = "Apache-2.0"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml_bw = "2"
schemars = "0.8"
anyhow = "1"
thiserror = "2"
blake3 = "1"
time = { version = "0.3", features = ["formatting"] }
semver = "1"
greentic-types = "=1.2.0-research.1"
greentic-pack-lib = "=1.2.0-research.0"
greentic-flow = "=1.2.0-research.0"
greentic-dw-types = "1.1.0-dev"
greentic-extension-sdk-contract = { workspace = false, version = "*" } # match designer Cargo.toml:XX exact pin
greentic-aw-runtime = { git = "…", rev = "…", features = ["test-mock"] } # copy verbatim from greentic-designer/Cargo.toml:73

[dev-dependencies]
zip = "2"
```

> Copy the exact `greentic-aw-runtime` and `greentic-extension-sdk-contract` dependency lines from `greentic-designer/Cargo.toml` (lines 73 and the sdk-contract line) rather than guessing the rev/version.

- [ ] **Step 2: Write `src/lib.rs` skeleton**

```rust
#![forbid(unsafe_code)]

pub mod model;      // WorkerSpec + sub-types (Task 2)
pub mod project;    // WorkerSpec -> answer Value (Task 4)
pub mod assemble;   // answer Value + knowledge -> .gtpack (Tasks 5-8)
pub mod slug;       // ported slugify (Task 3)

pub use model::*;
```

- [ ] **Step 3: Verify it compiles**

Run: `cd greentic-dw-authoring && cargo build`
Expected: PASS (empty modules; create empty `model.rs`/`project.rs`/`assemble.rs`/`slug.rs` with `//! placeholder` so the mods resolve).

- [ ] **Step 4: Commit**

```bash
git add greentic-dw-authoring/
git commit -m "feat(dw-authoring): scaffold crate"
```

### Task 2: `WorkerSpec` model

**Files:**
- Create: `greentic-dw-authoring/src/model.rs`
- Test: `greentic-dw-authoring/tests/model.rs`

**Interfaces:**
- Produces: `WorkerSpec`, `AgentKind`, `LlmRef`, `ToolRef`, `MemorySpec`, `KnowledgeSpec`, `EmbeddingRef`, `AgentGraphSpec`, `Specialist`, `DeepWorkerSpec`, `KnowledgeInput`. All `Serialize + Deserialize + JsonSchema`, `#[serde(rename_all = "snake_case")]` on `AgentKind`.

- [ ] **Step 1: Write the failing test**

```rust
// tests/model.rs
use greentic_dw_authoring::{AgentKind, WorkerSpec};

#[test]
fn worker_spec_round_trips_yaml() {
    let yaml = r#"
apiVersion: greentic.ai/v1
kind: single_turn
name: triage
llm: { provider: openai, model: gpt-4o, credential_ref: llm-openai }
instructions: "You are a triage agent."
tools: [ web.search ]
"#;
    let spec: WorkerSpec = serde_yaml_bw::from_str(yaml).expect("parse");
    assert_eq!(spec.kind, AgentKind::SingleTurn);
    assert_eq!(spec.name, "triage");
    assert_eq!(spec.llm.provider, "openai");
    assert_eq!(spec.tools, vec!["web.search".to_string()]);
}

#[test]
fn deep_worker_defaults() {
    let spec: WorkerSpec = serde_yaml_bw::from_str(
        "apiVersion: greentic.ai/v1\nkind: deep_worker\nname: r\nllm: {provider: openai, model: gpt-4o}\ninstructions: x\ndeep_worker: {}\n",
    ).unwrap();
    let dw = spec.deep_worker.unwrap();
    assert_eq!(dw.iteration_budget, 8);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p greentic-dw-authoring --test model`
Expected: FAIL (types not defined).

- [ ] **Step 3: Implement `model.rs`**

```rust
//! The canonical authoring format for an agentic worker.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    #[default]
    SingleTurn,
    AgentGraph,
    DeepWorker,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerSpec {
    #[serde(default)]
    pub kind: AgentKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    pub llm: LlmRef,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemorySpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<KnowledgeSpec>,
    #[serde(default)]
    pub guardrails: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_graph: Option<AgentGraphSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_worker: Option<DeepWorkerSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LlmRef {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MemorySpec {
    #[serde(default)]
    pub short_term: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_term: Option<ProviderRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderRef {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeSpec {
    pub provider: String,
    pub embedding: EmbeddingRef,
    #[serde(default = "default_top_k")]
    pub top_k: u32,
    /// Local file paths to bake into the corpus (CLI); ignored when `KnowledgeInput`s are supplied directly.
    #[serde(default)]
    pub documents: Vec<String>,
}
fn default_top_k() -> u32 { 5 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingRef {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentGraphSpec {
    pub coordinator: Coordinator,
    pub specialists: Vec<Specialist>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Coordinator {
    #[serde(default)]
    pub instructions: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Specialist {
    pub name: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeepWorkerSpec {
    #[serde(default = "default_iteration_budget")]
    pub iteration_budget: u32,
    #[serde(default)]
    pub reflection: bool,
    #[serde(default)]
    pub delegation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planning_model: Option<String>,
}
fn default_iteration_budget() -> u32 { 8 }
impl Default for DeepWorkerSpec {
    fn default() -> Self {
        Self { iteration_budget: 8, reflection: false, delegation: false, planning_model: None }
    }
}

/// Knowledge document text supplied by the caller (DB for the Designer, local files for the CLI).
#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeInput {
    pub id: String,
    pub text: String,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p greentic-dw-authoring --test model`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add greentic-dw-authoring/src/model.rs greentic-dw-authoring/tests/model.rs
git commit -m "feat(dw-authoring): WorkerSpec model"
```

### Task 3: Port `slugify`

**Files:**
- Create: `greentic-dw-authoring/src/slug.rs`
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces: `pub fn slugify(input: &str) -> String`

- [ ] **Step 1: Copy the function.** Open `greentic-designer/src/orchestrate/util.rs`, find `pub fn slugify`, and copy its body verbatim into `src/slug.rs`. Add its existing unit tests (or write one: `assert_eq!(slugify("Support Triage!"), "support-triage")`).
- [ ] **Step 2: Run** `cargo test -p greentic-dw-authoring slug` — Expected: PASS.
- [ ] **Step 3: Commit** `git commit -m "refactor(dw-authoring): port slugify"`

### Task 4: Projection `WorkerSpec → answer Value`

**Files:**
- Create: `greentic-dw-authoring/src/project.rs`
- Test: `greentic-dw-authoring/tests/project.rs`

**Interfaces:**
- Consumes: `WorkerSpec`, `slug::slugify`.
- Produces: `pub fn to_answer_document(spec: &WorkerSpec) -> Result<serde_json::Value, ProjectError>` and `pub fn executing_node(spec: &WorkerSpec) -> Option<serde_json::Value>`. `ProjectError` (thiserror) with `MissingLlm` — retained for parity even though `WorkerSpec.llm` is non-optional (validation lives in Task 9).

- [ ] **Step 1: Write the failing test**

```rust
// tests/project.rs
use greentic_dw_authoring::{project, AgentKind, DeepWorkerSpec, WorkerSpec, LlmRef};

fn base(kind: AgentKind) -> WorkerSpec {
    WorkerSpec {
        kind, name: "w".into(), description: None, tenant: None,
        llm: LlmRef { provider: "openai".into(), model: "gpt-4o".into(), credential_ref: None },
        instructions: "do things".into(), tools: vec![], memory: None, knowledge: None,
        guardrails: vec![], agent_graph: None, deep_worker: None,
    }
}

#[test]
fn single_turn_has_no_executing_node() {
    assert!(project::executing_node(&base(AgentKind::SingleTurn)).is_none());
}
#[test]
fn agent_graph_executing_node() {
    let n = project::executing_node(&base(AgentKind::AgentGraph)).unwrap();
    assert_eq!(n["kind"], "dw.agent_graph");
}
#[test]
fn deep_worker_executing_node_carries_config() {
    let mut s = base(AgentKind::DeepWorker);
    s.deep_worker = Some(DeepWorkerSpec { iteration_budget: 12, ..Default::default() });
    let n = project::executing_node(&s).unwrap();
    assert_eq!(n["kind"], "operala.call");
    assert_eq!(n["deep_worker"]["iteration_budget"], 12);
}
#[test]
fn answer_document_has_manifest_id_and_display_name() {
    let doc = project::to_answer_document(&base(AgentKind::SingleTurn)).unwrap();
    assert!(doc["manifest_id"].is_string());
    assert_eq!(doc["display_name"], "w");
}
```

- [ ] **Step 2: Run** `cargo test -p greentic-dw-authoring --test project` — Expected: FAIL.
- [ ] **Step 3: Implement `project.rs`.** Port the body of `dw_form_to_answer_doc.rs::convert` (`greentic-designer/src/orchestrate/dw_form_to_answer_doc.rs:39`), adapting: read from `WorkerSpec` fields instead of `DwFormState`; reproduce the same top-level JSON keys (`manifest_id`, `display_name`, `manifest{metadata,capability_plan,defaults,behavior_scaffold}`, `provider_overrides`, `locale`, `tenant`, optional `extension_tools`, `guardrails`); use `slug::slugify` for `resolve_manifest_id`. Implement `executing_node` exactly per the mapping in Design Decision 6.

```rust
//! Project a WorkerSpec into the untyped "answer document" the pack assembler consumes.
use crate::model::{AgentKind, WorkerSpec};
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("missing LLM binding")]
    MissingLlm,
}

pub fn executing_node(spec: &WorkerSpec) -> Option<Value> {
    match spec.kind {
        AgentKind::SingleTurn => None,
        AgentKind::AgentGraph => Some(json!({ "kind": "dw.agent_graph" })),
        AgentKind::DeepWorker => {
            let dw = spec.deep_worker.clone().unwrap_or_default();
            Some(json!({ "kind": "operala.call", "deep_worker": dw }))
        }
    }
}

pub fn to_answer_document(spec: &WorkerSpec) -> Result<Value, ProjectError> {
    // ... port dw_form_to_answer_doc::convert here, reading from WorkerSpec ...
    // Returns the same JSON shape the Designer produces today.
    todo!("port convert() body per Task 4 Step 3")
}
```

> The `todo!` above is a landmark for the implementer — replace it with the ported body in this same step; do not commit a `todo!`.

- [ ] **Step 4: Run** `cargo test -p greentic-dw-authoring --test project` — Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "feat(dw-authoring): WorkerSpec projection to answer document"`

### Task 5: Port `cbor_flow_post`

**Files:**
- Create: `greentic-dw-authoring/src/cbor_flow_post.rs`
- Modify: `greentic-dw-authoring/src/lib.rs` (add `pub(crate) mod cbor_flow_post;`)

**Interfaces:**
- Produces: `inject_sidecar(pack_bytes: &[u8], name: &str, contents: &[u8]) -> Result<Vec<u8>, PostProcessError>`, `populate_manifest_flows(...)`, `PostProcessError`. Use the exact signatures from `greentic-designer/src/orchestrate/cbor_flow_post.rs`.

- [ ] **Step 1: `git show` the source**, then copy `cbor_flow_post.rs` verbatim into the crate. Rewrite any `crate::orchestrate::…` imports to the crate-local paths. Keep its existing tests.
- [ ] **Step 2: Run** `cargo test -p greentic-dw-authoring cbor_flow_post` — Expected: PASS.
- [ ] **Step 3: Commit** `git commit -m "refactor(dw-authoring): port cbor_flow_post"`

### Task 6: Port the DW injector functions

**Files:**
- Create: `greentic-dw-authoring/src/inject.rs`
- Modify: `greentic-dw-authoring/src/lib.rs`

**Interfaces:**
- Consumes: `cbor_flow_post::inject_sidecar`, `greentic_aw_runtime::AgentConfig`.
- Produces (exact signatures copied from `pack_via_packc/mod.rs`):
  - `pub fn embed_dw_agents(pack_path: &Path, agents: &BTreeMap<String, greentic_aw_runtime::AgentConfig>) -> Result<(), String>`
  - `pub fn inject_dw_agent_graph_node(ygtc_text: &str, pack_id: &str) -> Result<String, String>`
  - `pub fn inject_operala_call_node(ygtc_text: &str, target: &str, deep_worker: &serde_json::Value) -> Result<String, String>`
  - `pub fn inject_dw_agent_nodes(ygtc_text: &str, injections: &[DwAgentInjection]) -> Result<String, String>` + `pub struct DwAgentInjection { pub node_id: String, pub agent_id: String, pub successor_id: Option<String> }`

- [ ] **Step 1: Copy the four fns + `DwAgentInjection`** from `greentic-designer/src/orchestrate/pack_via_packc/mod.rs` (lines 512, 1826-1829, 1851, 1921, 1990) into `inject.rs`. Rewrite imports to crate-local `cbor_flow_post`. Copy any of their unit tests.
- [ ] **Step 2: Write a smoke test** asserting `inject_operala_call_node("id: main\ntype: messaging\nstart: x\nnodes: {}", "tgt", &json!({"iteration_budget":8}))` returns `Ok` containing `operala.call`.
- [ ] **Step 3: Run** `cargo test -p greentic-dw-authoring inject` — Expected: PASS.
- [ ] **Step 4: Commit** `git commit -m "refactor(dw-authoring): port DW node injectors"`

### Task 7: Port `loadable` (runner-loadable manifest.cbor)

**Files:**
- Create: `greentic-dw-authoring/src/loadable.rs`
- Modify: `greentic-dw-authoring/src/lib.rs`

**Interfaces:**
- Consumes: `cbor_flow_post`, `greentic_types::{PackManifest, decode_pack_manifest, encode_pack_manifest}`.
- Produces: `pub fn make_runner_loadable(pack_path: &Path, pack_id: &str) -> Result<(), LoadableError>`, `pub(crate) fn single_turn_main_ygtc(agent_id: &str) -> Result<String, String>`, `LoadableError`.

- [ ] **Step 1: Copy `loadable.rs`** from `greentic-designer/src/orchestrate/runner_sidecar/loadable.rs` verbatim; rewrite `crate::orchestrate::cbor_flow_post` → crate-local. Keep the full `build_runner_manifest` `PackManifest` literal (all 15 fields) and `single_turn_main_ygtc` (the inline flow JSON at `:116-128`). Keep its tests (they use `greentic_flow` — add as dev-dep if needed).
- [ ] **Step 2: Run** `cargo test -p greentic-dw-authoring loadable` — Expected: PASS.
- [ ] **Step 3: Commit** `git commit -m "refactor(dw-authoring): port runner-loadable manifest builder"`

### Task 8: The DB-free assembler `build_worker_pack`

**Files:**
- Create: `greentic-dw-authoring/src/assemble.rs`
- Test: `greentic-dw-authoring/tests/assemble.rs`

**Interfaces:**
- Consumes: `project::to_answer_document`/`executing_node`, `inject::*`, `loadable::*`, `greentic_pack::builder::{PackBuilder, PackMeta, FlowBundle}`, `model::KnowledgeInput`, `greentic_aw_runtime::AgentConfig`.
- Produces:
  - `pub fn build_worker_pack(spec: &WorkerSpec, knowledge: &[KnowledgeInput], out_dir: &Path) -> Result<WorkerPack, AssembleError>`
  - `pub struct WorkerPack { pub pack_path: PathBuf, pub pack_id: String }`
  - `pub struct AssembleError` (thiserror)
  - `pub fn agent_configs(spec: &WorkerSpec) -> BTreeMap<String, greentic_aw_runtime::AgentConfig>` — the CLI's analog of `dw_form_to_agent_config` (build one `AgentConfig` per agent: single = one; graph = coordinator + specialists; deep = one).

This is the crate's centerpiece: it reproduces `materialize_worker_pack`'s orchestration (grounding A.5) but DB-free and file-free-of-sqlx.

- [ ] **Step 1: Write the failing test**

```rust
// tests/assemble.rs
use greentic_dw_authoring::{assemble, AgentKind, LlmRef, WorkerSpec};
use std::io::Read;

fn spec(kind: AgentKind) -> WorkerSpec { /* same builder as tests/project.rs `base` */ todo!() }

fn read_zip_entry(pack: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    let f = std::fs::File::open(pack).unwrap();
    let mut zip = zip::ZipArchive::new(f).unwrap();
    let mut file = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    Some(buf)
}

#[test]
fn single_turn_pack_is_runner_loadable() {
    let dir = tempdir_like(); // std::env::temp_dir().join(unique)
    let out = assemble::build_worker_pack(&spec(AgentKind::SingleTurn), &[], &dir).unwrap();
    let cbor = read_zip_entry(&out.pack_path, "manifest.cbor").expect("manifest.cbor present");
    greentic_types::decode_pack_manifest(&cbor).expect("decodes");
    assert!(read_zip_entry(&out.pack_path, "dw-agents.json").is_some());
    assert!(read_zip_entry(&out.pack_path, "flows/main.ygtc").is_some());
}
```

- [ ] **Step 2: Run** `cargo test -p greentic-dw-authoring --test assemble` — Expected: FAIL.
- [ ] **Step 3: Implement `build_worker_pack`.** Orchestration (mirroring grounding A.5, DB-free):
  1. `let answer = project::to_answer_document(spec)?;`
  2. Build the base pack with `PackBuilder` (port the relevant bits of `dw_pack.rs::build_dw_pack` — `PackMeta` construction at `dw_pack.rs:226-249`, `PackBuilder::new(meta).with_flow(...).build(out_path)`), writing a real flow rather than the stub: for `SingleTurn` use `loadable::single_turn_main_ygtc(display_name)`; for `AgentGraph`/`DeepWorker` start from a minimal messaging flow and apply `inject_dw_agent_graph_node` / `inject_operala_call_node` with `executing_node(spec)`.
  3. Bake knowledge: if `!knowledge.is_empty()`, chunk each `KnowledgeInput.text` and write `knowledge_corpus.json` + `assets/knowledge/*.txt` into the pack via `cbor_flow_post::inject_sidecar` (port the corpus-writing logic from `dw_application_pack.rs`, but taking `KnowledgeInput` text instead of DB rows).
  4. `loadable::make_runner_loadable(&pack_path, &pack_id)?;`
  5. `inject::embed_dw_agents(&pack_path, &agent_configs(spec))?;`
  6. Return `WorkerPack { pack_path, pack_id }`.
- [ ] **Step 4: Implement `agent_configs`.** Port `dw_form_to_agent_config.rs` mapping, reading from `WorkerSpec` (map `llm`→`LlmProviderRef`, `tools`→`Vec<ToolRef>`, `memory`→`MemorySettings`, `knowledge`→`KnowledgeSettings`, `guardrails`). Reference the runtime structs at `greentic-runner/crates/greentic-aw-runtime/src/config.rs:96,48,68`.
- [ ] **Step 5: Run** `cargo test -p greentic-dw-authoring --test assemble` — Expected: PASS.
- [ ] **Step 6: Commit** `git commit -m "feat(dw-authoring): DB-free worker pack assembler"`

### Task 9: Spec validation

**Files:**
- Create: `greentic-dw-authoring/src/validate.rs`
- Test: `greentic-dw-authoring/tests/validate.rs`

**Interfaces:**
- Produces: `pub fn validate(spec: &WorkerSpec) -> Result<(), Vec<ValidationError>>`, `pub struct ValidationError { pub field: String, pub message: String }`.

- [ ] **Step 1: Write failing tests** for: empty `name` → error; `agent_graph` kind with `< 2` specialists → error; `agent_graph` with duplicate specialist names → error; `deep_worker.iteration_budget == 0` or `> 100` → error; `single_turn` with an `agent_graph` block set → warning-free but error if kind/block mismatch. Assert error `field`s.
- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement `validate`** with explicit, non-panicking checks. No silent skips.
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "feat(dw-authoring): WorkerSpec validation"`

---

## Task Group 2 — Refactor the Designer onto the crate

### Task 10: Add the crate as a Designer dependency + adapter

**Files:**
- Modify: `greentic-designer/Cargo.toml` (add `greentic-dw-authoring = { path = "../greentic-dw-authoring" }`)
- Create: `greentic-designer/src/orchestrate/dw_authoring_adapter.rs` — `pub fn worker_spec_from_form(form: &DwFormState) -> WorkerSpec` and `pub async fn knowledge_inputs(db, tenant, team, ids) -> Vec<KnowledgeInput>` (reads `knowledge_documents` via the existing `knowledge_documents::get_with_text`).

**Interfaces:**
- Consumes: `greentic_dw_authoring::{WorkerSpec, KnowledgeInput, assemble}`, existing `DwFormState`, existing `knowledge_documents` repo.
- Produces: an adapter that lets the Designer build packs through the crate.

- [ ] **Step 1: Write a failing test** in the Designer: `worker_spec_from_form(&DwFormState::default())` produces a `WorkerSpec` with `kind == SingleTurn`.
- [ ] **Step 2: Run** `cargo test -p greentic-designer --features test-mock dw_authoring_adapter` — Expected: FAIL.
- [ ] **Step 3: Implement the adapter** — map each `DwFormState` field to the `WorkerSpec` equivalent (this is the inverse of the CLI's projection and the parity anchor).
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** (in the greentic-designer worktree) `git commit -m "feat(designer): dw-authoring adapter"`

### Task 11: Route `materialize_worker_pack` through the crate + parity test

**Files:**
- Modify: `greentic-designer/src/orchestrate/runner_sidecar/materialize.rs`
- Test: `greentic-designer/tests/dw_authoring_parity.rs`

**Interfaces:**
- Consumes: `dw_authoring_adapter`, `greentic_dw_authoring::assemble::build_worker_pack`.

- [ ] **Step 1: Write the parity test (failing).** For each kind, build a representative `DwFormState`; produce a pack the **old** way (current `materialize_worker_pack`) and the **new** way (adapter → `build_worker_pack`); assert the two `.gtpack`s have byte-equal `manifest.cbor` (decoded + re-encoded to normalize), equal `dw-agents.json`, and equal `flows/main.ygtc`.
- [ ] **Step 2: Run** — Expected: FAIL (new path not wired).
- [ ] **Step 3: Re-point `materialize_worker_pack`** internals: replace the hand-orchestrated body (grounding A.5 steps 1-7) with `let spec = worker_spec_from_form(form); let ki = knowledge_inputs(db, tenant, team, &spec.document_ids()).await; let pack = build_worker_pack(&spec, &ki, dir)?;` then keep the Designer-specific `gtbind::write_gtbind` tail. Preserve `WorkerPackArtifacts`.
- [ ] **Step 4: Run** the parity test + the full existing sidecar tests — Expected: PASS.
- [ ] **Step 5: Run** `bash ci/local_check.sh` in greentic-designer (fmt + clippy -D + tests + FE build). Expected: PASS.
- [ ] **Step 6: Commit** `git commit -m "refactor(designer): assemble worker packs via greentic-dw-authoring"`

### Task 12: Delete the now-duplicated Designer code

**Files:**
- Modify/Delete: the Designer copies of the ported logic (`dw_form_to_answer_doc.rs` projection body, the four injector fns in `pack_via_packc/mod.rs`, `runner_sidecar/loadable.rs`, `cbor_flow_post.rs`) — replace their bodies with re-exports of the crate, or delete and update call sites, whichever keeps the tree compiling.

- [ ] **Step 1:** For each duplicated item, replace the Designer definition with `pub use greentic_dw_authoring::… as …;` (or delete + fix imports). Keep anything still used only by the Designer.
- [ ] **Step 2: Run** `bash ci/local_check.sh` — Expected: PASS.
- [ ] **Step 3: Commit** `git commit -m "refactor(designer): drop code now owned by greentic-dw-authoring"`

---

## Task Group 3 — `greentic-dw worker` CLI

### Task 13: Add `worker` deps + subcommand types

**Files:**
- Modify: `greentic-dw/crates/greentic-dw-cli/Cargo.toml` (add `greentic-dw-authoring` path/version dep)
- Modify: `greentic-dw/crates/greentic-dw-cli/src/cli_types.rs` (add `Worker(WorkerArgs)` to `Command`; define `WorkerArgs` with a `WorkerSub` enum)
- Modify: `greentic-dw/crates/greentic-dw-cli/src/lib.rs` (add `mod worker;`)

**Interfaces:**
- Produces: clap `WorkerArgs { #[command(subcommand)] cmd: WorkerSub }`, `enum WorkerSub { Init { kind: String, out: Option<PathBuf> }, New { .. }, Build { spec: PathBuf, out: Option<PathBuf> }, Validate { spec: PathBuf } }`.

- [ ] **Step 1: Write a failing test** in `greentic-dw-cli`: parsing `["greentic-dw","worker","validate","spec.yaml"]` via `Cli::parse_from` yields `Command::Worker(WorkerArgs { cmd: WorkerSub::Validate { .. } })`.
- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Add the enum variants + `WorkerArgs`/`WorkerSub`** in `cli_types.rs` (mirror the `WizardArgs` style, `cli_types.rs:59-118`).
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "feat(dw-cli): worker subcommand types"`

### Task 14: Dispatch + `run_worker`

**Files:**
- Create: `greentic-dw/crates/greentic-dw-cli/src/worker.rs`
- Modify: `greentic-dw/crates/greentic-dw-cli/src/wizard.rs` (add `Command::Worker(a) => worker::run_worker(a)` to the `run` match at `wizard.rs:40-42`)

**Interfaces:**
- Consumes: `greentic_dw_authoring::{WorkerSpec, validate, assemble}`, `WorkerArgs`/`WorkerSub`.
- Produces: `pub fn run_worker(args: WorkerArgs) -> Result<(), CliError>`.

- [ ] **Step 1: Write failing tests** (`worker.rs` `#[cfg(test)]`):
  - `run_worker(validate spec)` on a valid YAML file → `Ok`; on an invalid one → `Err` with a message naming the field.
  - `run_worker(build spec)` writes a `.gtpack` at the requested path, and `greentic_types::decode_pack_manifest` succeeds on its `manifest.cbor`.
  - `run_worker(init single_turn)` writes a YAML file that `serde_yaml_bw::from_str::<WorkerSpec>` parses.
- [ ] **Step 2: Run** `cargo test -p greentic-dw-cli worker` — Expected: FAIL.
- [ ] **Step 3: Implement `run_worker`.**
  - `Init` → write a starter `WorkerSpec` (serialize a `Default`-ish spec for the kind) to `out` (default `./<kind>-worker.yaml`).
  - `Validate` → load YAML → `validate::validate` → print errors or "ok".
  - `Build` → load YAML → `validate` → read `knowledge.documents` local files, extract text (plain text as-is; `.pdf` via a `pdf-extract`/`lopdf` helper — reuse the Designer's `extract` approach, add the dep), build `Vec<KnowledgeInput>` → `assemble::build_worker_pack` → report path.
  - `New` → interactive prompts building a `WorkerSpec`, then the `Build` path. Mirror the `run_wizard` harness (`wizard.rs:45`) for `--answers`/non-interactive/`--schema` (`schema_for!(WorkerSpec)`).
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Run** `bash ci/local_check.sh` in greentic-dw. Expected: PASS.
- [ ] **Step 6: Commit** `git commit -m "feat(dw-cli): gtc worker init/new/build/validate"`

---

## Task Group 4 — `gtc worker` routing

### Task 15: Add `DW_BIN` + router arm

**Files:**
- Modify: `greentic/src/bin/gtc.rs` (add `const DW_BIN: &str = "greentic-dw";` near `gtc.rs:95-106`)
- Modify: `greentic/src/bin/gtc/router.rs` (`use super::…DW_BIN`; add `"worker" => Some((DW_BIN, tail.to_vec())),` to `route_passthrough_subcommand` at `:59-71`)
- Test: `greentic/src/bin/gtc/router.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: existing `route_passthrough_subcommand` pattern.

- [ ] **Step 1: Write a failing test:** `route_passthrough_subcommand("worker", &["build".into(),"s.yaml".into()], "en")` returns `Some(("greentic-dw", vec!["build","s.yaml"]))`.
- [ ] **Step 2: Run** `cargo test -p gtc router` — Expected: FAIL.
- [ ] **Step 3: Add `DW_BIN` + the match arm.**
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "feat(gtc): route worker subcommand to greentic-dw"`

### Task 16: Register the clap subcommand + dispatch

**Files:**
- Modify: `greentic/src/bin/gtc/cli.rs` (add a `worker` `.subcommand(...)` mirroring the `dev` block at `:920-928`, using `cmd_args.clone()`)
- Modify: `greentic/src/bin/gtc/commands.rs` (add `"worker"` to the passthrough tuple pattern at `:134`, skipping the wizard/setup-only branches)
- Modify: `greentic/src/bin/gtc/process.rs` (add `DW_BIN` to `is_greentic_companion_binary` `matches!` at `:427-437` and, if env-override desired, to `companion_binary_env_override` at `:486-501`)

**Interfaces:**
- Consumes: `passthrough`, `route_passthrough_subcommand`.

- [ ] **Step 1: Write a failing test** (or a shell smoke): `gtc worker --help` forwards to `greentic-dw worker --help`. Unit-test `is_greentic_companion_binary("greentic-dw") == true`.
- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement** the three edits.
- [ ] **Step 4: Run** `cargo test -p gtc` — Expected: PASS.
- [ ] **Step 5: Manual smoke:** build `gtc` + `greentic-dw`, put both on PATH, run `gtc worker init single_turn` → a YAML file appears; `gtc worker build <that>.yaml` → a `.gtpack`.
- [ ] **Step 6: Run** `bash ci/local_check.sh` in greentic. Expected: PASS.
- [ ] **Step 7: Commit** `git commit -m "feat(gtc): register worker passthrough command"`

---

## Self-Review

**Spec coverage:** Slice 1 §4 (shared crate, WorkerSpec, `gtc worker` verbs, data flow, error handling, all 3 kinds) → Task Groups 1-4. Parity test (§6) → Task 11. DB-free knowledge seam → Design Decision 2 + Task 8. Slice 2 (runtime productionization) and Slice 3 (deep_worker RAG) are **out of scope for this plan** (separate plans, per spec §8).

**Known landmarks left for the implementer (not placeholders — each has a precise source to port):** Task 4 Step 3 `to_answer_document` body (port `convert`), Task 8 Step 3 corpus-writing (port from `dw_application_pack.rs`), Task 14 PDF extraction (reuse Designer `extract`). Each names the exact source file+lines to copy from; the implementer must replace the `todo!()`/prose landmarks with the ported code in the same step and must not commit a `todo!()`.

**Type consistency:** `WorkerSpec`/`AgentKind`/`KnowledgeInput` defined in Task 2 are referenced consistently in Tasks 4, 8-11, 13-14. `build_worker_pack(spec, knowledge, out_dir) -> WorkerPack` signature is stable across Tasks 8, 11, 14.

**Open decision for review:** the `apiVersion` string in `WorkerSpec` YAML examples (`greentic.ai/v1`) is cosmetic — confirm the desired value. `WorkerSpec` currently omits per-tool extension bindings (`extension_tools`) and opening-message/vertical metadata that `DwFormState` carries; if CLI-authored workers must reach full Designer parity on those, add them to `WorkerSpec` before Task 11's parity test (otherwise the parity test must scope them out explicitly).

---

## Execution Handoff

See the skill's execution options after review.
