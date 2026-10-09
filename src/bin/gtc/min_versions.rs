//! The minimum companion-binary versions `gtc doctor` asserts.
//!
//! # Why this exists
//!
//! `gtc doctor` used to answer one question per companion binary: *is the file
//! there, and does `--version` exit 0?* That is a packaging question. The
//! question an operator is actually asking is *can this toolchain execute the
//! pack I have?* — and the two came apart badly enough to burn a whole day:
//!
//! A valid `.gtbundle` passed every gate in the product. `gtc doctor` reported
//! 11/11 binaries OK, including `greentic-start: OK (greentic-start 1.1.41)`;
//! `greentic-bundle doctor` reported 5/5; `greentic-setup` reported every step
//! done; `greentic-start doctor` was clean; `gtc start` booted and served a 200
//! on `/healthz`. The flow then died on its second node with:
//!
//! ```text
//! flow execution failed: adapter call failed: component 'var' not found in pack
//! ```
//!
//! The pack used the builtin `var.set`, and greentic-start 1.1.41 ships no
//! handler for it. Proof, taken from the two binaries themselves:
//!
//! ```text
//! strings greentic-start-1.1.41              | grep -c var_set   -> 0
//! strings greentic-start-1.2.0-dev.32809817892 | grep -c var_set -> 2
//! ```
//!
//! The same bundle runs to completion under the second binary. Nothing in the
//! product reported the difference, because nothing in the product was looking
//! at the version — only at the file.
//!
//! # What a number in this table means
//!
//! **A minimum here is a claim about RUNTIME CAPABILITY, not about packaging.**
//! It says: *below this version, a pack this toolchain will happily build and
//! boot fails at execution time.* It is not "the newest release", not "the
//! version we ship together", and not "the version in `Cargo.toml`". Those are
//! packaging facts and they do not belong here — asserting one would make
//! `gtc doctor` red for a mixed-but-working toolchain, which trains operators to
//! ignore the only signal that would have caught the real failure above.
//!
//! **Raising a number here is how a newly required builtin gets enforced.** When
//! the pack format starts emitting a builtin that an older runtime cannot
//! dispatch — `var.set` is the worked example; `mcp`, `approval.call` and
//! `state.get`/`state.set` have each gone the same way before — raise the
//! affected binary's minimum to the first version that carries the handler, and
//! say in `reason` which builtin bought the number. A reader six months from now
//! must be able to tell what the number is FOR, or the next person to touch it
//! will either bump it for a packaging reason or be afraid to touch it at all.
//!
//! **Every binary doctor probes has a row, including the ones with no minimum.**
//! `minimum: None` is a deliberate statement — "no runtime-capability floor has
//! been established for this binary" — and it is visible in the table. A binary
//! merely absent from the table would be indistinguishable from one nobody
//! remembered to add.
//!
//! **Do not add a number you have not verified.** The verification is the one
//! run above: find the oldest version whose binary actually carries the handler
//! (`strings <bin> | grep <symbol>` is enough to bracket it), and confirm a pack
//! using the builtin runs to completion under it and fails under its
//! predecessor. A guessed floor refuses toolchains that work.

use semver::Version;

/// One binary's declared runtime-capability floor.
pub(super) struct MinimumVersion {
    /// The logical companion-binary name (one of the `*_BIN` constants).
    pub(super) binary: &'static str,
    /// The lowest version known to carry the capability named in `reason`.
    ///
    /// `None` means no floor has been established — see the module docs. It is
    /// not "unknown"; it is "nobody has had a reason to assert one yet".
    pub(super) minimum: Option<&'static str>,
    /// What runtime capability the number buys, and how it was established.
    ///
    /// Read by a human, not by code. Keep it specific enough that the next
    /// person can tell whether their bump belongs here.
    pub(super) reason: &'static str,
    /// What the operator should actually run to fix it.
    pub(super) upgrade_hint: &'static str,
}

/// The default upgrade instruction. The toolchain is installed as a set, so
/// moving one companion forward means moving the release forward.
const TOOLCHAIN_UPGRADE_HINT: &str =
    "Run `gtc update` (or `gtc install --channel <channel>`) to move the toolchain forward.";

/// No floor asserted — the standing reason, so a `None` row reads as a decision.
const NO_FLOOR_ESTABLISHED: &str = "No runtime-capability floor established: no pack-execution failure has been traced to a \
     version of this binary yet. Add one only with the verification described in the module docs.";

/// The single minimum-version table.
///
/// Every binary `gtc doctor` probes appears here exactly once. See the module
/// docs before adding or raising a number: an entry is a claim about runtime
/// capability, not about packaging.
pub(super) const MINIMUM_VERSIONS: &[MinimumVersion] = &[
    MinimumVersion {
        binary: crate::DEV_BIN,
        minimum: None,
        reason: NO_FLOOR_ESTABLISHED,
        upgrade_hint: TOOLCHAIN_UPGRADE_HINT,
    },
    MinimumVersion {
        binary: crate::OP_BIN,
        minimum: None,
        reason: NO_FLOOR_ESTABLISHED,
        upgrade_hint: TOOLCHAIN_UPGRADE_HINT,
    },
    MinimumVersion {
        binary: crate::BUNDLE_BIN,
        minimum: None,
        reason: NO_FLOOR_ESTABLISHED,
        upgrade_hint: TOOLCHAIN_UPGRADE_HINT,
    },
    MinimumVersion {
        binary: crate::COMPONENT_BIN,
        minimum: None,
        reason: NO_FLOOR_ESTABLISHED,
        upgrade_hint: TOOLCHAIN_UPGRADE_HINT,
    },
    MinimumVersion {
        binary: crate::FLOW_BIN,
        minimum: None,
        reason: NO_FLOOR_ESTABLISHED,
        upgrade_hint: TOOLCHAIN_UPGRADE_HINT,
    },
    MinimumVersion {
        binary: crate::PACK_BIN,
        minimum: None,
        reason: NO_FLOOR_ESTABLISHED,
        upgrade_hint: TOOLCHAIN_UPGRADE_HINT,
    },
    MinimumVersion {
        binary: crate::RUNNER_BIN,
        minimum: None,
        // greentic-runner looks like the obvious next row to gain a number, and
        // deliberately does not get one. Checked 2026-08-26 with the same probe
        // that bracketed greentic-start: `greentic-runner 1.3.0-research.0` and
        // `greentic-runner 1.2.0-dev.0` BOTH carry the `var.set` handler, so no
        // version in circulation is below a floor there and asserting one would
        // be a guess with nothing behind it.
        //
        // Note also that `greentic-start` does not shell out to the
        // `greentic-runner` BINARY — it links `greentic-runner-host` as a Cargo
        // dependency, so the engine that raised `component 'var' not found in
        // pack` lives inside greentic-start itself and is at a different version
        // than whatever `greentic-runner --version` reports. A floor on this row
        // would therefore not have caught the reported failure even if it had
        // been set correctly.
        reason: NO_FLOOR_ESTABLISHED,
        upgrade_hint: TOOLCHAIN_UPGRADE_HINT,
    },
    MinimumVersion {
        binary: crate::SECRETS_BIN,
        minimum: None,
        reason: NO_FLOOR_ESTABLISHED,
        upgrade_hint: TOOLCHAIN_UPGRADE_HINT,
    },
    MinimumVersion {
        binary: crate::SETUP_BIN,
        minimum: None,
        reason: NO_FLOOR_ESTABLISHED,
        upgrade_hint: TOOLCHAIN_UPGRADE_HINT,
    },
    MinimumVersion {
        binary: crate::DEPLOYER_BIN,
        minimum: None,
        reason: NO_FLOOR_ESTABLISHED,
        upgrade_hint: TOOLCHAIN_UPGRADE_HINT,
    },
    MinimumVersion {
        binary: crate::START_BIN,
        // 1.1.41 has no `var.set` handler and fails a pack using it at its
        // second node with `component 'var' not found in pack`;
        // 1.2.0-dev.32809817892 runs the same bundle to completion. Expressed as
        // the `-dev` prerelease on purpose: semver orders
        // `1.2.0-dev.32809817892` ABOVE `1.2.0-dev` (same prefix, longer
        // prerelease) and `1.2.0` above both, so every build carrying the
        // handler satisfies it while 1.1.41 does not. A bare `1.2.0` here would
        // reject the very dev builds that fixed the bug.
        minimum: Some("1.2.0-dev"),
        reason: "Dispatches the `var.set` builtin. Below this, a pack using it builds, boots and \
                 serves /healthz, then fails at execution with `component 'var' not found in \
                 pack`.",
        upgrade_hint: TOOLCHAIN_UPGRADE_HINT,
    },
];

/// The row for `binary`, if it has one.
pub(super) fn minimum_for(binary: &str) -> Option<&'static MinimumVersion> {
    MINIMUM_VERSIONS.iter().find(|row| row.binary == binary)
}

/// What doctor learned about one binary's version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum VersionVerdict {
    /// No floor is asserted for this binary, so its version cannot be wrong.
    NoMinimum,
    /// A floor is asserted and this binary meets it.
    Satisfied,
    /// A floor is asserted and this binary is below it. Actionable, and the one
    /// verdict that fails `gtc doctor`.
    TooOld {
        installed: String,
        minimum: &'static str,
    },
    /// A floor is asserted and no version could be read out of `--version`.
    ///
    /// Deliberately NOT a failure. A locally built or overridden companion
    /// prints things like `greentic-dev local-test`, which is a legitimate and
    /// common state; turning every such install red would train operators to
    /// ignore doctor, which is the failure mode this whole file exists to
    /// prevent. It is still reported as its own status rather than as OK,
    /// because "we could not check" and "we checked and it is fine" are
    /// different facts and collapsing them is what produced the original bug.
    Unreadable { minimum: &'static str },
}

/// Extract the first parseable semver from a `--version` line.
///
/// Handles the shapes these binaries actually print — `greentic-start 1.1.41`,
/// `greentic-start 1.2.0-dev.32809817892`, `gtc v1.2.0` — and returns `None`
/// for a line carrying no version at all.
///
/// Deliberately NOT `install.rs::parse_first_semver`, which accepts only
/// digits-and-dots and so cannot see a prerelease at all; and deliberately not
/// `install.rs::semver_compare`, whose field-wise numeric compare reads
/// `1.2.0-dev.32809817892` as `[1, 2, 0, 32809817892]` and would therefore rank
/// a dev build ABOVE the stable release it precedes. Both are correct for
/// choosing an installable stable release, and both give the wrong answer to
/// the question this module asks.
pub(super) fn parse_version(text: &str) -> Option<Version> {
    text.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|ch: char| ch == '(' || ch == ')' || ch == ',');
        let token = token.strip_prefix('v').unwrap_or(token);
        Version::parse(token).ok()
    })
}

/// Judge one binary's reported `--version` line against the table.
pub(super) fn verdict(binary: &str, version_line: &str) -> VersionVerdict {
    let Some(row) = minimum_for(binary) else {
        return VersionVerdict::NoMinimum;
    };
    let Some(minimum) = row.minimum else {
        return VersionVerdict::NoMinimum;
    };
    let Ok(required) = Version::parse(minimum) else {
        // An unparseable literal in our own table is a bug in this file, not a
        // fact about the operator's install. Refusing their toolchain over it
        // would be the wrong party to punish; `the_table_parses` catches it.
        return VersionVerdict::NoMinimum;
    };
    let Some(installed) = parse_version(version_line) else {
        return VersionVerdict::Unreadable { minimum };
    };
    if installed >= required {
        VersionVerdict::Satisfied
    } else {
        VersionVerdict::TooOld {
            installed: installed.to_string(),
            minimum,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_parses() {
        for row in MINIMUM_VERSIONS {
            let Some(minimum) = row.minimum else {
                continue;
            };
            assert!(
                Version::parse(minimum).is_ok(),
                "{}: minimum {minimum:?} is not a semver version; \
                 an unparseable literal here silently disables the check",
                row.binary,
            );
        }
    }

    #[test]
    fn every_probed_binary_has_exactly_one_row() {
        // The list doctor probes, mirrored. A binary probed but absent from the
        // table is indistinguishable from one deliberately given no floor, so
        // the two lists have to be checked against each other.
        for binary in [
            crate::DEV_BIN,
            crate::OP_BIN,
            crate::BUNDLE_BIN,
            crate::COMPONENT_BIN,
            crate::FLOW_BIN,
            crate::PACK_BIN,
            crate::RUNNER_BIN,
            crate::SECRETS_BIN,
            crate::SETUP_BIN,
            crate::DEPLOYER_BIN,
            crate::START_BIN,
        ] {
            let rows = MINIMUM_VERSIONS
                .iter()
                .filter(|row| row.binary == binary)
                .count();
            assert_eq!(
                rows, 1,
                "{binary} should have exactly one row, found {rows}"
            );
        }
        assert_eq!(
            MINIMUM_VERSIONS.len(),
            11,
            "a row was added without adding the binary to the probe list above",
        );
    }

    #[test]
    fn every_row_explains_itself() {
        for row in MINIMUM_VERSIONS {
            assert!(
                !row.reason.trim().is_empty(),
                "{}: a row with no reason cannot be maintained",
                row.binary,
            );
            assert!(
                !row.upgrade_hint.trim().is_empty(),
                "{}: a failing row must tell the operator what to run",
                row.binary,
            );
        }
    }

    #[test]
    fn parse_version_reads_the_shapes_these_binaries_print() {
        assert_eq!(
            parse_version("greentic-start 1.1.41"),
            Some(Version::parse("1.1.41").expect("literal"))
        );
        assert_eq!(
            parse_version("greentic-start 1.2.0-dev.32809817892"),
            Some(Version::parse("1.2.0-dev.32809817892").expect("literal"))
        );
        assert_eq!(
            parse_version("gtc v1.2.0"),
            Some(Version::parse("1.2.0").expect("literal"))
        );
        assert_eq!(parse_version("greentic-dev local-test"), None);
        assert_eq!(parse_version(""), None);
    }

    /// The exact regression that motivated this module.
    #[test]
    fn greentic_start_1_1_41_is_too_old_for_var_set() {
        assert_eq!(
            verdict(crate::START_BIN, "greentic-start 1.1.41"),
            VersionVerdict::TooOld {
                installed: "1.1.41".to_string(),
                minimum: "1.2.0-dev",
            },
        );
    }

    /// The dev build that fixed it must SATISFY the floor. This is the assertion
    /// that a bare `1.2.0` minimum would break: semver ranks every
    /// `1.2.0-dev.*` below `1.2.0`, so the fix would read as too old.
    #[test]
    fn the_dev_build_that_carries_var_set_satisfies_the_floor() {
        assert_eq!(
            verdict(crate::START_BIN, "greentic-start 1.2.0-dev.32809817892"),
            VersionVerdict::Satisfied,
        );
        assert_eq!(
            verdict(crate::START_BIN, "greentic-start 1.2.0"),
            VersionVerdict::Satisfied,
        );
        assert_eq!(
            verdict(crate::START_BIN, "greentic-start 1.3.7"),
            VersionVerdict::Satisfied,
        );
    }

    #[test]
    fn an_unreadable_version_is_reported_but_does_not_claim_too_old() {
        assert_eq!(
            verdict(crate::START_BIN, "greentic-start local-test"),
            VersionVerdict::Unreadable {
                minimum: "1.2.0-dev"
            },
        );
    }

    #[test]
    fn a_binary_with_no_floor_is_never_too_old() {
        assert_eq!(
            verdict(crate::BUNDLE_BIN, "greentic-bundle 0.0.0"),
            VersionVerdict::NoMinimum,
        );
        assert_eq!(
            verdict("not-a-companion", "whatever 9.9.9"),
            VersionVerdict::NoMinimum
        );
    }
}
