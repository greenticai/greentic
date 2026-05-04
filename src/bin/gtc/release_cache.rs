use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::ArgMatches;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use greentic_distributor_client::DistOptions;
use gtc::error::{GtcError, GtcResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const RELEASE_INDEX_SCHEMA: &str = "greentic.release-index.v1";
const RELEASE_CACHE_SCHEMA: &str = "greentic.release-cache.v1";

pub(super) fn run_release_cache(matches: &ArgMatches, _debug: bool, _locale: &str) -> i32 {
    match matches.subcommand() {
        Some(("export", sub_matches)) => match run_release_cache_export(sub_matches) {
            Ok(path) => {
                println!("Exported release cache to {}", path.display());
                0
            }
            Err(err) => {
                eprintln!("failed to export release cache: {err}");
                1
            }
        },
        Some(("import", sub_matches)) => match run_release_cache_import(sub_matches) {
            Ok(summary) => {
                println!(
                    "Imported release cache {} {} with {} artifacts",
                    summary.channel, summary.release, summary.artifacts
                );
                0
            }
            Err(err) => {
                eprintln!("failed to import release cache: {err}");
                1
            }
        },
        _ => {
            eprintln!("usage: gtc release-cache <export|import> ...");
            2
        }
    }
}

fn run_release_cache_export(matches: &ArgMatches) -> GtcResult<PathBuf> {
    let release = required_arg(matches, "release")?;
    let channel = required_arg(matches, "channel")?;
    let output = PathBuf::from(required_arg(matches, "output")?);
    let cache_root = DistOptions::default().cache_dir;
    let index_rel = release_index_rel_path(&channel, &release)?;
    let index_path = cache_root.join(&index_rel);
    let index = read_release_index(&index_path)?;

    if index.schema != RELEASE_INDEX_SCHEMA {
        return Err(GtcError::message(format!(
            "unsupported release index schema '{}'",
            index.schema
        )));
    }
    if index.release != release || release_channel_str(&index.channel) != channel {
        return Err(GtcError::message(format!(
            "release index {} describes {}/{}",
            index_path.display(),
            release_channel_str(&index.channel),
            index.release
        )));
    }

    let mut payloads = BTreeMap::new();
    payloads.insert(
        index_rel,
        fs::read(&index_path)
            .map_err(|err| GtcError::io(format!("failed to read {}", index_path.display()), err))?,
    );

    for entry in index.refs.values() {
        let artifact_dir = artifact_rel_dir(&entry.digest)?;
        for filename in ["blob", "entry.json"] {
            let rel = artifact_dir.join(filename);
            let path = cache_root.join(&rel);
            let bytes = fs::read(&path)
                .map_err(|err| GtcError::io(format!("failed to read {}", path.display()), err))?;
            payloads.insert(rel, bytes);
        }
    }

    let manifest = ReleaseCacheManifest {
        schema: RELEASE_CACHE_SCHEMA.to_string(),
        format: "tar.gz".to_string(),
        release: release.clone(),
        channel: channel.clone(),
        created_at: unix_timestamp_string(),
        artifact_count: index.refs.len(),
    };
    payloads.insert(
        PathBuf::from("manifest.json"),
        serde_json::to_vec_pretty(&manifest)
            .map_err(|err| GtcError::json("failed to encode release cache manifest", err))?,
    );

    let checksums = checksums_for_payloads(&payloads);
    let checksums_bytes = serde_json::to_vec_pretty(&checksums)
        .map_err(|err| GtcError::json("failed to encode release cache checksums", err))?;

    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|err| GtcError::io(format!("failed to create {}", parent.display()), err))?;
    }
    let file = fs::File::create(&output)
        .map_err(|err| GtcError::io(format!("failed to create {}", output.display()), err))?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut archive = tar::Builder::new(encoder);
    append_payloads(&mut archive, &payloads)?;
    append_bytes(&mut archive, Path::new("checksums.json"), &checksums_bytes)?;
    let encoder = archive
        .into_inner()
        .map_err(|err| GtcError::io("failed to finish release cache tar", err))?;
    encoder
        .finish()
        .map_err(|err| GtcError::io("failed to finish release cache gzip", err))?;
    Ok(output)
}

fn run_release_cache_import(matches: &ArgMatches) -> GtcResult<ImportSummary> {
    let input = PathBuf::from(required_arg(matches, "input")?);
    let temp = tempfile::tempdir()
        .map_err(|err| GtcError::io("failed to create release cache import tempdir", err))?;
    unpack_archive(&input, temp.path())?;
    verify_import_checksums(temp.path())?;

    let manifest: ReleaseCacheManifest = read_json(temp.path().join("manifest.json"))?;
    if manifest.schema != RELEASE_CACHE_SCHEMA {
        return Err(GtcError::message(format!(
            "unsupported release cache schema '{}'",
            manifest.schema
        )));
    }
    if manifest.format != "tar.gz" {
        return Err(GtcError::message(format!(
            "unsupported release cache format '{}'",
            manifest.format
        )));
    }

    let index_rel = release_index_rel_path(&manifest.channel, &manifest.release)?;
    let index_path = temp.path().join(&index_rel);
    let index = read_release_index(&index_path)?;
    if index.schema != RELEASE_INDEX_SCHEMA {
        return Err(GtcError::message(format!(
            "unsupported release index schema '{}'",
            index.schema
        )));
    }
    if index.release != manifest.release || release_channel_str(&index.channel) != manifest.channel
    {
        return Err(GtcError::message(
            "release cache manifest and index disagree",
        ));
    }
    if index.refs.len() != manifest.artifact_count {
        return Err(GtcError::message(format!(
            "release cache manifest lists {} artifacts but index contains {}",
            manifest.artifact_count,
            index.refs.len()
        )));
    }

    let cache_root = DistOptions::default().cache_dir;
    for entry in index.refs.values() {
        let artifact_dir = artifact_rel_dir(&entry.digest)?;
        for filename in ["blob", "entry.json"] {
            let rel = artifact_dir.join(filename);
            let source = temp.path().join(&rel);
            if !source.is_file() {
                return Err(GtcError::message(format!(
                    "release cache missing {}",
                    rel.display()
                )));
            }
            copy_atomic(&source, &cache_root.join(&rel))?;
        }
    }
    copy_atomic(&index_path, &cache_root.join(&index_rel))?;

    Ok(ImportSummary {
        release: manifest.release,
        channel: manifest.channel,
        artifacts: index.refs.len(),
    })
}

fn required_arg(matches: &ArgMatches, name: &str) -> GtcResult<String> {
    matches
        .get_one::<String>(name)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| GtcError::message(format!("missing --{name}")))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReleaseCacheManifest {
    schema: String,
    format: String,
    release: String,
    channel: String,
    created_at: String,
    artifact_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ReleaseIndex {
    schema: String,
    release: String,
    channel: ReleaseChannel,
    refs: BTreeMap<String, ReleaseIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ReleaseIndexEntry {
    version: String,
    digest: String,
    canonical_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ReleaseChannel {
    Stable,
    Dev,
    Rnd,
}

#[derive(Debug)]
struct ImportSummary {
    release: String,
    channel: String,
    artifacts: usize,
}

fn read_release_index(path: &Path) -> GtcResult<ReleaseIndex> {
    read_json(path)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: impl AsRef<Path>) -> GtcResult<T> {
    let path = path.as_ref();
    let bytes = fs::read(path)
        .map_err(|err| GtcError::io(format!("failed to read {}", path.display()), err))?;
    serde_json::from_slice(&bytes)
        .map_err(|err| GtcError::json(format!("failed to parse {}", path.display()), err))
}

fn release_index_rel_path(channel: &str, release: &str) -> GtcResult<PathBuf> {
    validate_single_segment(channel, "channel")?;
    validate_single_segment(release, "release")?;
    Ok(PathBuf::from("release-index")
        .join("v1")
        .join(channel)
        .join(format!("{release}.json")))
}

fn artifact_rel_dir(digest: &str) -> GtcResult<PathBuf> {
    let hex = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| GtcError::message(format!("unsupported artifact digest '{digest}'")))?;
    if hex.len() != 64 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(GtcError::message(format!(
            "invalid artifact digest '{digest}'"
        )));
    }
    let (prefix, rest) = hex.split_at(2);
    Ok(PathBuf::from("artifacts")
        .join("sha256")
        .join(prefix)
        .join(rest))
}

fn validate_single_segment(value: &str, label: &str) -> GtcResult<()> {
    if value.trim().is_empty()
        || Path::new(value).components().count() != 1
        || value.contains(std::path::MAIN_SEPARATOR)
    {
        return Err(GtcError::message(format!(
            "{label} must be a single path segment"
        )));
    }
    Ok(())
}

fn checksums_for_payloads(payloads: &BTreeMap<PathBuf, Vec<u8>>) -> BTreeMap<String, String> {
    payloads
        .iter()
        .map(|(path, bytes)| (archive_path_string(path), sha256_bytes(bytes)))
        .collect()
}

fn verify_import_checksums(root: &Path) -> GtcResult<()> {
    let checksums: BTreeMap<String, String> = read_json(root.join("checksums.json"))?;
    let payloads = collect_import_payloads(root)?;

    for (rel, path) in &payloads {
        if rel == "checksums.json" {
            continue;
        }
        let expected = checksums
            .get(rel)
            .ok_or_else(|| GtcError::message(format!("missing checksum for {rel}")))?;
        let bytes = fs::read(path)
            .map_err(|err| GtcError::io(format!("failed to read {}", path.display()), err))?;
        let actual = sha256_bytes(&bytes);
        if &actual != expected {
            return Err(GtcError::message(format!(
                "checksum mismatch for {rel}: expected {expected}, got {actual}"
            )));
        }
    }

    for (rel, expected) in &checksums {
        let rel_path = validate_archive_path(Path::new(&rel))?;
        let path = root.join(&rel_path);
        if !path.is_file() {
            return Err(GtcError::message(format!(
                "checksum references missing file {rel}"
            )));
        }
        let bytes = fs::read(&path)
            .map_err(|err| GtcError::io(format!("failed to read {}", path.display()), err))?;
        let actual = sha256_bytes(&bytes);
        if actual != *expected {
            return Err(GtcError::message(format!(
                "checksum mismatch for {rel}: expected {expected}, got {actual}"
            )));
        }
    }
    Ok(())
}

fn collect_import_payloads(root: &Path) -> GtcResult<BTreeMap<String, PathBuf>> {
    let mut out = BTreeMap::new();
    collect_import_payloads_inner(root, root, &mut out)?;
    Ok(out)
}

fn collect_import_payloads_inner(
    root: &Path,
    current: &Path,
    out: &mut BTreeMap<String, PathBuf>,
) -> GtcResult<()> {
    let entries = fs::read_dir(current)
        .map_err(|err| GtcError::io(format!("failed to read {}", current.display()), err))?;
    for entry in entries {
        let entry = entry.map_err(|err| {
            GtcError::io(
                format!("failed to read entry in {}", current.display()),
                err,
            )
        })?;
        let path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|err| GtcError::io(format!("failed to stat {}", path.display()), err))?;
        if metadata.is_dir() {
            collect_import_payloads_inner(root, &path, out)?;
        } else if metadata.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|err| GtcError::path("failed to calculate archive path", err))?;
            out.insert(archive_path_string(rel), path);
        } else {
            return Err(GtcError::message(format!(
                "release cache archive may contain only regular files: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn append_payloads<W: io::Write>(
    archive: &mut tar::Builder<W>,
    payloads: &BTreeMap<PathBuf, Vec<u8>>,
) -> GtcResult<()> {
    for (path, bytes) in payloads {
        append_bytes(archive, path, bytes)?;
    }
    Ok(())
}

fn append_bytes<W: io::Write>(
    archive: &mut tar::Builder<W>,
    path: &Path,
    bytes: &[u8],
) -> GtcResult<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, path, bytes)
        .map_err(|err| GtcError::io(format!("failed to append {}", path.display()), err))
}

fn unpack_archive(input: &Path, out_dir: &Path) -> GtcResult<()> {
    let file = fs::File::open(input)
        .map_err(|err| GtcError::io(format!("failed to open {}", input.display()), err))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|err| GtcError::io("failed to read release cache archive", err))?;
    for entry in entries {
        let mut entry =
            entry.map_err(|err| GtcError::io("failed to read release cache entry", err))?;
        if !entry.header().entry_type().is_file() {
            return Err(GtcError::message(
                "release cache archive may contain only regular files",
            ));
        }
        let rel = entry
            .path()
            .map_err(|err| GtcError::io("failed to read release cache entry path", err))?;
        let rel = validate_archive_path(&rel)?;
        let target = out_dir.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                GtcError::io(format!("failed to create {}", parent.display()), err)
            })?;
        }
        entry
            .unpack(&target)
            .map_err(|err| GtcError::io(format!("failed to unpack {}", target.display()), err))?;
    }
    Ok(())
}

fn validate_archive_path(path: &Path) -> GtcResult<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => out.push(value),
            _ => {
                return Err(GtcError::message(format!(
                    "unsafe release cache path {}",
                    path.display()
                )));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(GtcError::message("empty release cache path"));
    }
    Ok(out)
}

fn copy_atomic(source: &Path, target: &Path) -> GtcResult<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| GtcError::io(format!("failed to create {}", parent.display()), err))?;
    }
    let tmp = target.with_extension(format!("tmp.{}", std::process::id()));
    fs::copy(source, &tmp).map_err(|err| {
        GtcError::io(
            format!("failed to copy {} to {}", source.display(), tmp.display()),
            err,
        )
    })?;
    fs::rename(&tmp, target).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        GtcError::io(
            format!(
                "failed to replace {} with {}",
                target.display(),
                tmp.display()
            ),
            err,
        )
    })
}

fn archive_path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn release_channel_str(channel: &ReleaseChannel) -> &'static str {
    match channel {
        ReleaseChannel::Stable => "stable",
        ReleaseChannel::Dev => "dev",
        ReleaseChannel::Rnd => "rnd",
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity("sha256:".len() + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn unix_timestamp_string() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{seconds}")
}
