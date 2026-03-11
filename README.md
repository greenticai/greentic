# Greentic - The Digital Workers OS

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
[![WASM](https://img.shields.io/badge/runtime-WASM%20%7C%20WASIp2-green)]()
[![Deterministic](https://img.shields.io/badge/model-Deterministic-important)]()
[![AI](https://img.shields.io/badge/AI-Just--Enough-orange)]()

## Deterministic Digital Workers with 'Just Enough AI' for Enterprise Automation

------------------------------------------------------------------------

# Why Greentic?

AI demos are easy.

Production-grade AI infrastructure is not.

Most “agentic” platforms allow LLMs to call tools dynamically. This is
powerful — but unpredictable. Even a 0.1% hallucination or misexecution
rate becomes unacceptable at enterprise scale.

Greentic was designed with one core principle:

> Deterministic by default. **Just enough AI** to add real business value.

Greentic gives you:

-   Predictable orchestration
-   Explicit capability control
-   Self-describing components
-   Multi-tenant governance
-   Secure AI integration

This is infrastructure for serious production systems.

------------------------------------------------------------------------

Digital Workers need to interact with humans. That is why Greentic speaks over 50 languages: Hi, Hello, Hola, Bonjour, Hallo, Ciao, Olá, Привет, Здравейте, Γειά, Merhaba, مرحبا, שלום, नमस्ते, হ্যালো, ਸਤ ਸ੍ਰੀ ਅਕਾਲ, નમસ્તે, வணக்கம், నమస్తే, ನಮಸ್ಕಾರ, നമസ്കാരം, ආයුබෝවන්, வணக்கம், สวัสดี, Xin chào, Halo, Hai, Kumusta, 你好, 你好嗎, こんにちは, こんにちは, 안녕하세요, Сәлем, Салом, Салом, Сәләм, Сайн байна уу, မင်္ဂလာပါ, ជំរាបសួរ, ສະບາຍດີ, გამარჯობა, Բարև, Salut, Ahoj, Čau, Cześć, Szia, Bună, Zdravo, Bok, Pozdrav, Hej, Hei, Halló, Dia dhuit, Ola, Sawubona, Jambo.

------------------------------------------------------------------------

# What Is a Digital Worker?

A **digital worker** is a deterministic flow that handles a complete
task from start to finish.

It typically combines:

-   Message or event intake
-   Explicit orchestration logic
-   Component/tool execution
-   Optional AI steps where useful

The goal is controllable automation with predictable behavior.

The more boring and repetitive the task, the better to migrate it to digital workers. If you don't know how to do it, how are you going to ask AI to do it for you? Will it do a brilliant job or an aweful job. You don't know. If you are bored of doing it, do you mind if digital workers do it for you?

------------------------------------------------------------------------

# Core Concepts

-   **Components:** Self-describing executable units with explicit capabilities.
-   **Flows:** Deterministic orchestration graphs connecting components.
-   **Packs/Bundles:** Distribution and deployment grouping layers.
-   **Operator:** Runtime boundary that enforces capability, tenancy and other controls.

------------------------------------------------------------------------

# High-Level Features

## 🧱 Component-Based Architecture

-   WebAssembly (WASM) components (100x smaller than Docker)
-   WASIp2 sandboxing (100x faster and more secure than Docker)
-   Everything is self-describing
-   Explicit lifecycle (setup / update / remove)
-   Capability-based security model - you can only do what you were approved to do

## 🔁 Deterministic Orchestration

-   Flow graph execution
-   Explicit transitions
-   Session support
-   Shared state support
-   Canonical CBOR runtime format (faster and smaller)

## 💬 Messaging & Events

-   Slack, Teams, Webex, WhatsApp, Telegram, WebChat
-   Webhooks, Email, SMS & Timers
-   Adaptive Card (=mini-apps) with translation/downscaling
-   Session-based workflows
-   Stateless event flows

## 🤖 AI — Controlled & Explicit

-   Chat2Flow (intent → flow routing)
-   Chat2Data (natural language → system dialect like guardrailed SQL and MCP) - commercial
-   Fast2Flow (<100ms natural language routing with LLM fallback) - commercial
-   Explicit LLM components
-   Capability-bound AI
-   No unbounded autonomous agents

## ⚡ MCP Without the Overhead

-   WASIX-based MCP (KBs to MB)
-   No JSON-RPC or SSE (no network server in front of an API server)
-   Millisecond local execution
-   Everything remains a component
-   OpenAPI and Swagger to MCP in seconds - commercial

## 🔌 Extensible by Design

Extension packs enable:

-   Secrets backends
-   Redis/shared state
-   OpenTelemetry
-   OAuth providers
-   Access policies (personalise it)
-   Routing engines (personalise it)
-   Audit/Compliance/Analytics (personalise it)
-   Terraform/K8S/other deployers
-   Anything you want [within reason ;-)]

------------------------------------------------------------------------

# Installation

Install Greentic via cargo-binstall (cargo install cargo-binstall):

``` bash
cargo binstall gtc
gtc install
```

Why the explicit version? Current `gtc` release on crates.io is pre-release
(`1.0.0-alpha1`), and `cargo-binstall gtc` (without `--version`) may fail with
`no version matching requirement '*'`.

If you still prefer to install from source:

``` bash
git clone https://github.com/greenticai/greentic.git
cd greentic/greentic
cargo install --path . --locked
gtc install
```
Run dependency checks:

``` bash
gtc doctor
```

Install modes:

``` bash
# Public tools only
gtc install

# Tenant-authorized install (key via env)
export GREENTIC_ACME_KEY=ghp_xxxxxx
gtc install --tenant acme

# Tenant-authorized install (key via flag)
gtc install --tenant acme --key ghp_xxxxxx
```

Tenant key env var format:

- `GREENTIC_<TENANT>_KEY`
- Tenant normalization: uppercase, non-alphanumeric as `_`, collapse repeated `_`, trim leading/trailing `_`

Artifact install locations:

- Tools: `$CARGO_HOME/bin` (fallback `~/.cargo/bin`)
- Components: `~/.greentic/artifacts/components/<name>/...`
- Packs: `~/.greentic/artifacts/packs/<name>/...`
- Bundles: `~/.greentic/artifacts/bundles/<name>/...`
- Windows root: `%USERPROFILE%\\.greentic\\artifacts\\...`

Exit policy:

- If public tools install fails, tenant install is skipped and `gtc` exits with the same non-zero code.
- Tenant artifacts are installed best-effort per item, but overall exit is non-zero if any tenant artifact fails.

------------------------------------------------------------------------

# Prerequisites

Install Rust 1.91 or better via `rustup` if needed as well as binstall:

``` bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install 1.91.0
rustup target add wasm32-wasip2
cargo install cargo-binstall
```

Confirm installation:

``` bash
cargo --version
```

------------------------------------------------------------------------

# Quickstart (5-Minute Demo)

``` bash
# Make your first bundle
gtc op demo new myfirst.gtbundle
# Setup demo environment
gtc op demo setup --bundle ./myfirst.gtbundle --tenant default --team default
# Start operator
gtc op demo start --bundle ./myfirst.gtbundle --tenant default --team default --cloudflared off
```

In your browser go to:
[Open Greentic Webchat at localhost:8080](http://localhost:8080)

You now have a running deterministic digital worker runtime.

------------------------------------------------------------------------

# CLI Overview

## Development Commands

``` bash
gtc dev wizard
gtc dev pack new --help
gtc dev flow add-step --help
gtc dev flow update-step --help
gtc dev flow remove-step --help
gtc dev cbor <file>.cbor --help
```

## Operator Commands

``` bash
gtc wizard
gtc op demo setup --bundle <something>.gtbundle --tenant default --team default
gtc op demo start --bundle <something>.gtbundle --tenant default --team default --cloudflared off
```

Operator supports:

-   Local demo CLI
-   Production mTLS REST API

------------------------------------------------------------------------

# Architecture Overview

Greentic builds digital workers in layers:

    Component → Flow → Pack → Bundle → Operator

## Component

-   WASM module
-   Self-describing contract
-   Explicit capabilities
-   Deterministic lifecycle

## Flow

-   Graph of nodes
-   YAML authoring → CBOR production
-   Explicit transitions

## Pack

-   ZIP distribution unit
-   Components + flows
-   Versioned & validated
-   `greentic-pack doctor`

## Bundle

-   Defines deployed packs
-   Configures tenant/team access
-   Enables extensions and providers (messaging/events)

## Operator

-   Setup phase (QA, config, warmup)
-   Start phase (serve traffic)
-   Capability enforcement
-   WASM JIT caching

------------------------------------------------------------------------

# Messaging vs Events

## Messaging

-   Session-based workflows
-   Adaptive card support
-   Provider-specific translation / downscaling (=WhatsApp does not support cards)
-   Multi-step orchestration

## Events

-   Fire-and-forget
-   Timers
-   Webhooks
-   SMS
-   Email
-   Stateless execution

------------------------------------------------------------------------

# Deterministic Model

Greentic avoids:

-   Autonomous tool-calling LLM agents
-   Unbounded execution graphs
-   Ambient authority

Instead:

-   Flows define execution paths
-   AI for routing and execution is optional
-   Capabilities are declared upfront
-   Configuration is versioned & validated

Enterprise-ready by design.

------------------------------------------------------------------------

# Multi-Tenancy

Hierarchy:

-   Global
-   Tenant
-   Team
-   User

Operator denies everything by default. Access must be explicitly
granted.

------------------------------------------------------------------------

# Performance Model

-   WASM JIT warmup
-   Millisecond execution
-   No JSON-RPC latency
-   Local MCP execution
-   Deterministic payload passing

------------------------------------------------------------------------

# Comparison

| Feature             | Greentic | LangChain | n8n | Zapier |
|---------------------|----------|-----------|-----|--------|
| Deterministic flows | ✅       | ❌        | ⚠️   | ⚠️     |
| Capability sandbox  | ✅       | ❌        | ❌   | ❌     |
| Sandboxed runtime   | ✅       | ❌        | ❌   | ❌     |
| Just-Enough AI      | ✅       | ❌        | ❌   | ❌     |
| Multi-tenant infra  | ✅       | ⚠️        | ❌   | ❌     |
| Secure MCP          | ✅       | ❌        | ❌   | ❌     |

------------------------------------------------------------------------

# Repository Structure

-   `greentic-interfaces` - shared wasm interfaces
-   `greentic-types` - shared structures
-   `greentic-component` - everything component related
-   `greentic-flow` - everything flows related
-   `greentic-pack` - everything pack related
-   `greentic-operator` - executing bundles with packs
-   `greentic-dev` - developer tools
-   `greentic-mcp` - everything mcp related
-   `greentic-messaging-providers` - Teams, Slack, Webex, etc.
-   `greentic-events-providers` - Webhook, timer, SMS, email, etc.
-   Extension repos like oAuth, State, Session, Telemetry, etc.
-   `component-*` - open source components

------------------------------------------------------------------------

# Sponsors

-   [Greentic AI Ltd](https://greentic.ai) - the company behind Greentic
-   [3Point.ai](https://3point.ai) with 3AIgent powered by Greentic - get AI ROI quickly
-   [DataArt](https://dataart.com) - core contributors and certified technical consultants
-   [Become a sponsor](mailto:sponsor@greentic.ai)

------------------------------------------------------------------------

# Contributing

1.  Fork
2.  Create feature branch
3.  Add tests
4.  Run `cargo fmt` & `cargo clippy`
5.  Open PR

Please include:

-   Design explanation
-   Migration notes (if applicable)
-   Test coverage

------------------------------------------------------------------------

# Governance & Versioning

-   Semantic versioning
-   Stable `component@0.6.0` contract
-   Controlled migration paths
-   Explicit deprecations

------------------------------------------------------------------------

# Security

Greentic enforces:

-   Capability-based execution
-   WASIp2 sandboxing
-   No ambient authority
-   Multi-tenant isolation

Report vulnerabilities responsibly (see SECURITY.md).

------------------------------------------------------------------------

# Maintainers

Greentic is maintained by the Greentic core team and contributors.

Community governance roadmap coming soon.

------------------------------------------------------------------------

# License

See LICENSE for details.

------------------------------------------------------------------------

# Final Perspective

Greentic is not a demo framework.

It is deterministic digital worker operating system designed for
enterprise-scale production systems.

If you want AI — without losing control — Greentic is your foundation.

Visit [Greentic.ai](https://greentic.ai)
