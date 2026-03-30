# Fuzzing

This directory contains lightweight `cargo-fuzz` targets for parser-heavy entry
points that should never panic on malformed CLI input.

Targets:

- `parse_start_request`
- `parse_stop_request`

Run locally with:

```bash
cargo install cargo-fuzz
cargo fuzz run parse_start_request
cargo fuzz run parse_stop_request
```
