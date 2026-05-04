PR-GTC-03 — Bug remediation plan for audit findings

Summary

This PR plan covers the 11 bug findings from the March 28, 2026 static analysis report.

The intent is not to land every fix in one unreviewable blob. The intent is to make each bug:

1. reproducible with a deterministic test where feasible,
2. fixed with the smallest safe code change, and
3. protected with a regression test that goes green afterward.

Scope

In scope:

- BUG-001 through BUG-011 from the audit report
- unit tests inside `src/bin/gtc.rs` where the target remains private
- integration tests under `tests/` where subprocess or filesystem behavior matters
- small refactors needed to make security-sensitive behavior testable

Out of scope:

- performance findings
- architecture findings except where a tiny refactor is needed to make a bug testable

General test policy

- Prefer deterministic tests over flaky concurrency demonstrations.
- For security bugs, a red test may validate the unsafe behavior structurally rather than depending on exploitation.
- If a bug cannot be reproduced safely in CI as an actual failure mode, the red test should prove the insecure precondition and the green test should prove the hardened invariant.
- New tests that mutate process environment must use the shared env lock until production code stops mutating global env.

Implementation order

1. BUG-002 redirect credential leak
2. BUG-004 tar symlink traversal
3. BUG-005 download integrity verification
4. BUG-003 private key permissions
5. BUG-008 prompt choice off-by-one
6. BUG-007 symlink-following directory walks
7. BUG-009 extraction permissions
8. BUG-010 tenant placeholder replacement
9. BUG-001 unsafe env mutation
10. BUG-006 zeroization of secrets
11. BUG-011 intentional `leak_str` behavior review

Tracking table

| ID | Title | Red test | Fix | Green condition |
| --- | --- | --- | --- | --- |
| BUG-001 | Thread-unsafe env mutation | env-mutation tests expose missing guard coverage | stop mutating global env in prod; lock or isolate tests | no unlocked env-mutating tests; prod path passes env explicitly |
| BUG-002 | Auth leaked on redirects | cross-host redirect test observes bearer on second hop | strip auth on cross-origin redirects | same-host keeps auth, cross-host drops it |
| BUG-003 | Private keys world-readable | Unix permission test sees non-0600 key files | chmod key files to 0600 after write | key files are 0600 on Unix |
| BUG-004 | Tar symlink traversal | crafted tar escapes extraction root | reject symlink and hardlink entries, verify target stays inside root | archive no longer writes outside root |
| BUG-005 | Missing SHA-256 verification | installer accepts wrong digest | hash downloaded artifact and compare with manifest | mismatch fails, match succeeds |
| BUG-006 | Secrets never zeroed | regression test proves secret wrapper is used through credential collection | move secrets to zeroizing wrappers and avoid global env | secret-handling code uses zeroizing types and drops promptly |
| BUG-007 | Recursive walk follows symlinks | symlink cycle test recurses forever or overflows without guard | use `DirEntry::file_type()` and skip symlinks | cycle is skipped and traversal terminates |
| BUG-008 | `prompt_choice` accepts `0` | unit test with input `0` returns first option | reject zero explicitly | input `0` errors |
| BUG-009 | Overly broad execute bits | extracted text file gets executable world perms | only chmod files that should be executable | non-binaries stay non-executable |
| BUG-010 | Fragile `{tenant}` replacement | unit test with placeholder at URL end fails to substitute | replace all `{tenant}` occurrences | all placeholder positions substitute |
| BUG-011 | Intentional `leak_str` leak | characterization test documents one-way allocation behavior | keep as-is or replace with owned storage if clap API allows | chosen behavior documented and stable |

BUG-001 — Thread-unsafe `env::set_var` in async/test context

Current code

- Production credential prompts in `src/bin/gtc.rs` call `unsafe { env::set_var(...) }`.
- Several tests mutate env without `env_test_lock()`, including `key_resolution_prefers_cli_then_env`, `resolve_companion_binary_uses_env_override`, and `resolve_companion_binary_falls_back_to_cargo_home_bin`.

Red test

- Add a unit test helper that scans the test module source text and fails if a test contains `set_var` or `remove_var` without also acquiring `env_test_lock()`.
- Add focused regression tests for key resolution and companion binary resolution that use the lock today.

Why this red test

- A real data race reproduction would be nondeterministic and not suitable for CI.
- A structural test is acceptable here because Rust 2024 already tells us the operation is unsafe; the invariant we need is serialized mutation now, followed by removal of global env mutation in production.

Fix

- Introduce small credential carrier structs:
  - `AwsCredentialEnv`
  - `AzureCredentialEnv`
  - `GcpCredentialEnv`
- Make prompt functions return these values instead of mutating process env.
- Thread those values into the process launch points and apply them with `Command::env`.
- For tests that must still touch env, require `env_test_lock()`.
- Add a tiny helper for temporary env overrides in tests so cleanup happens in `Drop`.

Green condition

- No production credential prompt writes to global env.
- All env-mutating tests use the shared lock or an equivalent serial helper.
- Existing credential-related tests still pass after the refactor.

Suggested tests

- `credential_prompts_return_env_pairs_without_global_mutation`
- `env_mutating_tests_require_env_lock`
- `resolve_tenant_key_prefers_cli_then_env_locked`

BUG-002 — Bearer token leaked on HTTP redirects

Current code

- `fetch_https_bytes` in `src/bin/gtc.rs` manually follows redirects and always sends `Authorization: Bearer {key}`.

Red test

- Extract request execution behind a tiny helper so it can be tested with a local HTTP server.
- Add an integration test with two servers:
  - server A returns `302 Location: serverB/...`
  - server B records whether `Authorization` arrived
- Current behavior should show the token on server B, which is the bug.

Fix

- Track the original authority of the starting URL.
- Only attach `Authorization` when the request target authority matches the original authority.
- Keep `Accept`, `User-Agent`, and GitHub API version headers on all hops if still needed.

Green condition

- Cross-origin redirect drops `Authorization`.
- Same-origin redirect preserves `Authorization`.

Suggested tests

- `fetch_https_bytes_keeps_auth_on_same_host_redirect`
- `fetch_https_bytes_drops_auth_on_cross_host_redirect`

BUG-003 — Private key files written with default permissions

Current code

- `generate_dev_admin_cert_bundle` writes `ca.key`, `server.key`, and `client.key` with `fs::write`.

Red test

- Add a Unix-only unit test that calls `ensure_admin_certs_ready` or `generate_dev_admin_cert_bundle` and inspects file mode bits.
- The current test should fail because the mode inherits the process umask instead of being forced to `0600`.

Fix

- Add a helper like `write_private_key_file(path, pem)`:
  - write file contents
  - on Unix set permissions to `0o600`
- Use that helper for all private key outputs.

Green condition

- `ca.key`, `server.key`, and `client.key` are `0600` on Unix.
- Existing cert generation tests still pass.

Suggested tests

- `generated_admin_private_keys_are_owner_only`

BUG-004 — Tar extraction vulnerable to symlink traversal

Current code

- `extract_tar_archive` sanitizes `..` with `safe_join` but still creates symlinks and unpacks through them.

Red test

- Add a unit test that constructs a tar archive in memory with:
  - symlink entry `escape -> <outside dir>`
  - file entry `escape/pwned.txt`
- Current behavior should write outside the extraction root on Unix and therefore fail the safety expectation.

Fix

- Reject `tar::EntryType::Symlink` and `tar::EntryType::Link`.
- Before unpacking a normal file, ensure parent directories under `out_dir` are not symlinks.
- Keep the `safe_join` check as the first line of defense.

Green condition

- Crafted archive no longer writes outside `out_dir`.
- Symlink and hardlink entries are skipped or rejected with a clear error.

Suggested tests

- `extract_tar_archive_rejects_symlink_entries`
- `extract_tar_archive_does_not_write_through_symlink_parent`

BUG-005 — Download integrity not verified

Current code

- `ToolInstallTarget.sha256` is parsed but unused.
- `sha256_file` already exists and can be reused after being made streaming-safe later if desired.

Red test

- Add a unit test around the installer path that stages a fake downloaded artifact plus a manifest target with an incorrect digest.
- Current behavior accepts the artifact, which should fail the test.

Fix

- After download, compute the artifact digest and compare against the manifest’s `sha256`.
- Normalize accepted formats once:
  - either raw hex in manifest
  - or `sha256:<hex>`
- Fail fast with an integrity-specific error message before extraction or install.

Green condition

- Matching digest installs successfully.
- Mismatched digest returns a clear error and leaves no installed binary behind.

Suggested tests

- `install_tenant_tool_reference_rejects_sha256_mismatch`
- `install_tenant_tool_reference_accepts_matching_sha256`

BUG-006 — Cloud credentials never zeroed from memory

Current code

- `prompt_secret` and `prompt_optional_secret` return plain `String`.
- Production code then writes those secrets into process-global env.

Red test

- This should not attempt to inspect raw process memory in CI.
- Instead add a compile-time or unit-level regression test around the secret-handling API surface:
  - secret-returning helpers use `zeroize::Zeroizing<String>` or `secrecy::SecretString`
  - secret-bearing structs do not expose `Debug`

Fix

- Introduce `zeroize` and wrap prompt-returned secrets in `Zeroizing<String>`.
- Refactor credential prompts to return typed credential values instead of mutating env.
- Apply secrets to child process `Command` objects as late as possible.
- Zeroize buffers immediately after child process configuration is complete.

Green condition

- Secret-bearing prompt helpers and credential structs use zeroizing wrappers.
- No secret path requires long-lived plain `String` storage beyond unavoidable library boundaries.

Suggested tests

- `secret_prompt_returns_zeroizing_string`
- `cloud_credential_structs_do_not_implement_debug`

Note

- This is a hardening change, not a bug that can be “seen go red” by observing memory directly in normal CI.

BUG-007 — Recursive directory walking follows symlinks

Current code

- `collect_bundle_entries` and `recurse_files` use `path.is_dir()`.
- `dir_declares_static_routes` already uses `entry.file_type()` and is in better shape than the audit suggests.

Red test

- Add a Unix-only test directory with a symlink cycle, for example `root/a/loop -> root`.
- Current `collect_bundle_entries` or `recurse_files` behavior will recurse through the symlink path.
- To keep CI deterministic, the test should target a new non-recursive helper or a bounded traversal so it fails by counting unexpected symlink-derived entries rather than by stack overflow.

Fix

- Replace `path.is_dir()` with `entry.file_type()?.is_dir()`.
- Explicitly skip `file_type.is_symlink()`.
- Apply the same rule to every recursive walk helper.

Green condition

- Traversal skips symlink cycles and terminates normally.
- Real files are still discovered.

Suggested tests

- `collect_bundle_entries_skips_symlink_cycles`
- `recurse_files_skips_symlinked_directories`

BUG-008 — `prompt_choice` silently accepts input `0`

Current code

- `prompt_choice` uses `choice.saturating_sub(1)`.

Red test

- Extract the numeric parsing into a pure helper:
  - `parse_prompt_choice(input, len) -> Result<usize, String>`
- Add a unit test showing `"0"` currently maps to index `0`.

Fix

- Reject values where `choice == 0 || choice > options.len()`.
- Keep prompt I/O and parsing separate so future tests stay easy.

Green condition

- `"0"` is rejected.
- `"1"` maps to index `0`.
- Out-of-range values still error cleanly.

Suggested tests

- `parse_prompt_choice_rejects_zero`
- `parse_prompt_choice_accepts_first_option`

BUG-009 — Extraction grants execute bits too broadly

Current code

- `set_executable_if_unix` currently ORs in `0o755` for every extracted file.
- It is called from both zip and tar extraction.

Red test

- Add a Unix-only test that extracts an archive containing `config.json` and confirms the extracted file becomes executable today.

Fix

- Narrow executable-bit application to actual executable artifacts.
- Good first pass:
  - only set execute bits for files in known binary install flows
  - do not chmod during generic archive extraction
- If executable normalization is still needed, make it opt-in with a dedicated parameter from the install path.

Green condition

- Non-binary files remain non-executable after extraction.
- Tool binaries installed from archive still work.

Suggested tests

- `extract_zip_bytes_does_not_mark_text_files_executable`
- `extract_tar_archive_does_not_mark_text_files_executable`

BUG-010 — Tenant placeholder replacement is fragile

Current code

- `rewrite_store_tenant_placeholder` only replaces `"/{tenant}/"`.

Red test

- Add tests for placeholder positions not covered today:
  - end of URL
  - beginning of path segment
  - repeated placeholder

Fix

- Replace every `{tenant}` occurrence, not just slash-delimited ones.

Green condition

- All placeholder positions are substituted.
- Existing test continues to pass.

Suggested tests

- `rewrite_store_tenant_placeholder_replaces_trailing_placeholder`
- `rewrite_store_tenant_placeholder_replaces_multiple_placeholders`

BUG-011 — Intentional memory leak via `leak_str`

Current code

- `leak_str` leaks strings for clap builder APIs.

Assessment

- This is informational, not a security or correctness bug in the same class as the others.
- A “bug present / bug fixed / green” flow is optional here.

Red test

- Prefer a characterization test only if we keep this design.
- Example: test that localized CLI creation succeeds repeatedly and returns stable command names.

Fix options

- Option A: keep `leak_str` and document why.
- Option B: replace with owned storage if clap usage allows it cleanly without complicating the builder.

Recommended path

- Keep `leak_str` in this PR unless another bug fix naturally removes it.
- Add a short code comment documenting the deliberate tradeoff.

Green condition

- Behavior remains stable and documented.

Suggested tests

- `build_cli_can_be_constructed_multiple_times_with_localized_strings`

PR slicing

This should be delivered as a stack, not one mega-commit.

Suggested slice plan:

1. Redirect + placeholder safety
   - BUG-002
   - BUG-010

2. Archive extraction safety
   - BUG-004
   - BUG-009
   - BUG-007

3. Tool install integrity
   - BUG-005

4. Admin cert hardening
   - BUG-003

5. Prompt correctness
   - BUG-008

6. Env mutation and secret handling
   - BUG-001
   - BUG-006

7. Documentation-only follow-up
   - BUG-011

Definition of done

- Every bug in this document has:
  - a linked test name
  - a code owner area
  - a red-to-green validation path
- No fix relies on a flaky race reproduction.
- New tests pass on Linux; Unix-only tests are clearly annotated.
- Security fixes fail closed with explicit error messages.

File touch expectations

- `src/bin/gtc.rs`
- `tests/gtc_router_integration.rs`
- possible new `tests/install_security.rs` or `tests/archive_security.rs`
- `Cargo.toml` only if we add `zeroize` or a test helper crate

Notes from repo review

- BUG-007 is partially overstated in the audit: `dir_declares_static_routes` already uses `entry.file_type()` rather than `path.is_dir()`. The risky recursive walkers are `collect_bundle_entries` and `recurse_files`.
- BUG-001 is fully credible in this tree: unlocked env mutation exists in tests today.
- BUG-003, BUG-004, BUG-005, BUG-008, BUG-009, and BUG-010 all map directly to current code with straightforward regression coverage.
