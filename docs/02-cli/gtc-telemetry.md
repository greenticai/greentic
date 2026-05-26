Status: Canonical in this repo
Scope: Telemetry env vars honored by `gtc start` and the runner / provider components it launches
Implementation owner: `gtc` for env passthrough; `greentic-runner-host` for the host tracing pipeline; `greentic-telemetry` for the guest fallback emitter

# `gtc` telemetry env vars

> **Companion components.** This page assumes the operator is running:
>
> - `greentic-runner` **>= 0.5.18** (adds `TelemetryStream` that parses guest
>   stdout lines and re-emits them through the host `tracing` pipeline; see
>   `crates/greentic-runner-host/src/telemetry_scan.rs`).
> - `greentic-telemetry` **>= 0.5.4** (adds the structured fallback line
>   format used below: RFC3339 timestamp, `[component]` prefix from
>   `set_component_name`, `file:line` via `#[track_caller]`,
>   `span-start` / `span-end` lifecycle with `id` and `duration_ms`,
>   `set_min_level` floor, and the explicit-`Location` `_at` variants that
>   keep span-end attribution symmetric with span-start).
>
> Older runner / telemetry versions still work, but lines will lack the
> timestamp prefix and `[component]` tag, and `span-end` lines will be
> missing entirely or attributed to the wrapper. Upgrade both pieces
> together for the full table below to apply.

The runner and every provider component see your shell environment, so any
of the variables below set before `gtc start` flow through. Defaults are
chosen so the common case (local dev, console logs) needs **zero env
vars**.

## You usually only need one

To ship guest events to an OTLP collector, set the exporter:

```sh
export TELEMETRY_EXPORT=otlp-grpc
gtc start ./mybundle
```

That's it. `OTLP_ENDPOINT` defaults to `http://localhost:4317` (grpc) /
`http://localhost:4318` (http), which is where Tempo, Jaeger, and the
OpenTelemetry Collector listen out of the box.

## The full table

| Variable                       | When to set                                                                                                                                                | Default                                                |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| `TELEMETRY_EXPORT`             | When you want logs/spans to leave the host process. Values: `json-stdout`, `otlp-grpc`, `otlp-http`.                                                       | unset = local console only (still structured)          |
| `OTLP_ENDPOINT`                | When your collector is not on localhost.                                                                                                                   | `http://localhost:4317` (grpc) / `:4318` (http)        |
| `OTLP_HEADERS`                 | Only when shipping to a **managed** collector that gates on a token (Honeycomb, Grafana Cloud, Datadog, ...). Format: `key=value,key=value`, URL-decoded.  | unset                                                  |
| `OTLP_COMPRESSION`             | High-volume shipping over WAN where bandwidth matters. Local collectors don't need it. Value: `gzip`.                                                      | unset                                                  |
| `TELEMETRY_SAMPLING`           | Production with high event rates where you don't want every span exported. Values: `always_on`, `always_off`, `parent`, `traceidratio:0.5`.                | `always_on`                                            |
| `GREENTIC_TELEMETRY_FILE_LINE` | Drop the `file:line` suffix from guest fallback lines when a tight loop is flooding the log. Values: `on` / `off`.                                         | `on`                                                   |
| `GREENTIC_TELEMETRY_LEVEL`     | Verbosity floor inside each component, before any stdout write. Values: `trace` `debug` `info` `warn` `error`.                                             | `trace`                                                |
| `RUST_LOG`                     | Verbosity floor at the host, applied after a guest line is parsed. Standard `tracing-subscriber` directive syntax.                                         | `info`                                                 |

The component identity (`[messaging.webchat-gui]` prefix on every line) is
registered automatically by `provider_common::telemetry` on the first
telemetry call; there is no env knob for it because it must match the
provider's canonical id.

## What you should see

Without any env vars, three files land in `<bundle>/logs/`:

| File           | What                                                                |
| -------------- | ------------------------------------------------------------------- |
| `operator.log` | Host-side structured log; every line carries `src/<file>:<line>`.   |
| `trace.log`    | `tracing` subscriber output (guest fallback + native host spans).   |
| `flow.log`     | One `[NODE]` line per flow step (`Ok` / `Error` / `Wait`).          |

Sample `trace.log` line for a guest fallback event:

```text
2026-05-25T10:30:42.123Z DEBUG components/foo/src/lib.rs:160 span-start: send_payload provider=messaging.webchat-gui fields={"id":"1","event_kind":"send_payload"}
```

With `TELEMETRY_EXPORT=otlp-grpc` plus a collector at the default endpoint,
the same events become OTLP spans/log records with the `provider`,
`source`, and `fields` attributes attached.

## Levels

Two knobs, applied in series. An event has to pass both to reach the
exporter:

- `GREENTIC_TELEMETRY_LEVEL` — floor **inside the component**. Cheapest;
  the line is never formatted. Reach for it first when a tight loop is
  hot.
- `RUST_LOG` — floor **at the host**, after the line is parsed. Use it
  when you want per-target precision (e.g.
  `RUST_LOG="warn,messaging_webchat_gui=debug"`).

```sh
# Quiet one component; keep the rest at defaults.
GREENTIC_TELEMETRY_LEVEL=info gtc start ./mybundle

# Quiet everything except one target.
RUST_LOG="warn,messaging_webchat_gui=debug" gtc start ./mybundle
```

### Always-silenced crates

Regardless of `RUST_LOG`, the host clamps these noisy upstream crates to
`warn` (raising them is intentionally not supported — they drown the log):

```
wasmtime, wasmtime_wasi, wasi_common, cranelift_codegen, cranelift_wasm,
regalloc2, h2, hyper, hyper_util, rustls, tokio_util, want, mio, tower
```

If you genuinely need wasmtime-level traces, edit `NOISY_TRACE_TARGETS` in
`greentic-start::build_trace_filter` rather than reaching for `RUST_LOG`.

## Troubleshooting

- "I set `TELEMETRY_EXPORT` but nothing reaches my collector": double-check
  `OTLP_ENDPOINT`. The runner banner prints the endpoint it is using on
  boot; mismatches show up as connection errors in the runner's own
  stderr.
- "Logs are too noisy": `GREENTIC_TELEMETRY_FILE_LINE=off` removes the
  per-line caller location. For an even bigger drop, set
  `TELEMETRY_SAMPLING=traceidratio:0.1` to keep only 10% of spans.
- "Two components emit similar messages and I can't tell which is which":
  every line is prefixed with `[<provider>]` after the level. Filter by
  that. If a line has no `[<provider>]` prefix it came from the runner
  itself, not a component.
