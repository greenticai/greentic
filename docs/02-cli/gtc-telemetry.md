Status: Canonical in this repo
Scope: Telemetry env vars honored by `gtc start` and the runner / provider components it launches
Implementation owner: `gtc` for env passthrough; `greentic-runner-host` for the host tracing pipeline; `greentic-telemetry` for the guest fallback emitter

# `gtc` telemetry env vars

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
| `GREENTIC_TELEMETRY_FILE_LINE` | When tight `Trace`-level loops flood the log with `file:line` suffixes. Values: `on`, `off` (also `0/1`, `true/false`, `yes/no`).                          | `on`                                                   |

The component identity (`[messaging.webchat-gui]` prefix on every line) is
registered automatically by `provider_common::telemetry` on the first
telemetry call; there is no env knob for it because it must match the
provider's canonical id.

## What you should see

Without any env vars, guest fallback lines land at the runner's stdout
through the host tracing pipeline in this shape:

```text
2026-05-25T10:30:42.123Z DEBUG components/foo/src/lib.rs:160 span-start: send_payload provider=messaging.webchat-gui fields={"id":"1","event_kind":"send_payload"}
```

With `TELEMETRY_EXPORT=otlp-grpc` plus a collector at the default endpoint,
the same events become OTLP spans/log records with the `provider`,
`source`, and `fields` attributes attached.

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
