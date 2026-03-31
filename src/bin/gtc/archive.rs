use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Command as ProcessCommand;

use gtc::error::{GtcError, GtcResult};

pub(super) fn looks_like_zip(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"PK\x03\x04"
}

pub(super) fn looks_like_gzip(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0x1F && data[1] == 0x8B
}

pub(super) fn looks_like_squashfs(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"hsqs"
}

pub(super) fn extract_squashfs_file(path: &Path, out_dir: &Path) -> GtcResult<()> {
    let output = ProcessCommand::new("unsquashfs")
        .arg("-no-progress")
        .arg("-dest")
        .arg(out_dir)
        .arg(path)
        .output()
        .map_err(|e| {
            GtcError::io(
                format!("failed to run unsquashfs for {}", path.display()),
                e,
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(GtcError::message(format!(
        "failed to extract squashfs bundle {}: {}{}{}",
        path.display(),
        stdout.trim(),
        if !stdout.trim().is_empty() && !stderr.trim().is_empty() {
            " "
        } else {
            ""
        },
        stderr.trim()
    )))
}

pub(super) fn extract_zip_bytes(data: &[u8], out_dir: &Path) -> GtcResult<()> {
    let cursor = std::io::Cursor::new(data);
    let mut zip = zip::ZipArchive::new(cursor)
        .map_err(|e| GtcError::message(format!("failed to read zip archive: {e}")))?;

    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| GtcError::message(format!("failed to read zip entry #{i}: {e}")))?;
        let Some(path) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let target = safe_join(out_dir, &path)?;
        if entry.is_dir() {
            ensure_no_symlink_ancestors(out_dir, &target)?;
            fs::create_dir_all(&target)
                .map_err(|e| GtcError::io(format!("failed to create {}", target.display()), e))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            ensure_no_symlink_ancestors(out_dir, parent)?;
            fs::create_dir_all(parent)
                .map_err(|e| GtcError::io(format!("failed to create {}", parent.display()), e))?;
        }
        ensure_no_symlink_ancestors(out_dir, &target)?;
        let mut out = fs::File::create(&target)
            .map_err(|e| GtcError::io(format!("failed to create {}", target.display()), e))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| GtcError::io(format!("failed to write {}", target.display()), e))?;
    }

    Ok(())
}

pub(super) fn extract_targz_bytes(data: &[u8], out_dir: &Path) -> GtcResult<()> {
    let cursor = std::io::Cursor::new(data);
    let decoder = flate2::read::GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(decoder);
    extract_tar_archive(&mut archive, out_dir)
}

pub(super) fn extract_tar_bytes(data: &[u8], out_dir: &Path) -> GtcResult<()> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = tar::Archive::new(cursor);
    extract_tar_archive(&mut archive, out_dir)
}

pub(super) fn extract_tar_archive<R: Read>(
    archive: &mut tar::Archive<R>,
    out_dir: &Path,
) -> GtcResult<()> {
    for entry in archive
        .entries()
        .map_err(|e| GtcError::message(format!("failed to read tar archive: {e}")))?
    {
        let mut entry =
            entry.map_err(|e| GtcError::message(format!("failed to read tar entry: {e}")))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(GtcError::invalid_data(
                "archive entry",
                "uses unsupported link type",
            ));
        }
        let path = entry
            .path()
            .map_err(|e| GtcError::message(format!("failed to resolve tar entry path: {e}")))?
            .to_path_buf();
        let target = safe_join(out_dir, &path)?;

        if let Some(parent) = target.parent() {
            ensure_no_symlink_ancestors(out_dir, parent)?;
            fs::create_dir_all(parent)
                .map_err(|e| GtcError::io(format!("failed to create {}", parent.display()), e))?;
        }
        ensure_no_symlink_ancestors(out_dir, &target)?;
        entry
            .unpack(&target)
            .map_err(|e| GtcError::io(format!("failed to unpack {}", target.display()), e))?;
    }

    Ok(())
}

pub(super) fn safe_join(base: &Path, rel: &Path) -> GtcResult<PathBuf> {
    let mut clean = PathBuf::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(v) => clean.push(v),
            Component::CurDir => {}
            _ => return Err(GtcError::invalid_data("archive entry", "has unsafe path")),
        }
    }
    Ok(base.join(clean))
}

fn ensure_no_symlink_ancestors(base: &Path, candidate: &Path) -> GtcResult<()> {
    let relative = candidate.strip_prefix(base).map_err(|_| {
        GtcError::invalid_data(
            "archive entry path",
            format!("{} escapes extraction root", candidate.display()),
        )
    })?;
    let mut current = base.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(GtcError::invalid_data(
                    "archive entry path",
                    format!(
                        "archive entry traverses symlinked path {}",
                        current.display()
                    ),
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(GtcError::io(
                    format!("failed to inspect {}", current.display()),
                    err,
                ));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn set_executable_if_unix(path: &Path) -> GtcResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path)
        .map_err(|e| GtcError::io(format!("failed to stat {}", path.display()), e))?;
    let mut perms = metadata.permissions();
    let mode = perms.mode();
    perms.set_mode(mode | 0o755);
    fs::set_permissions(path, perms)
        .map_err(|e| GtcError::io(format!("failed to chmod {}", path.display()), e))
}

#[cfg(not(unix))]
pub(super) fn set_executable_if_unix(_path: &Path) -> GtcResult<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::{ensure_no_symlink_ancestors, set_executable_if_unix};
    use super::{looks_like_gzip, looks_like_squashfs, looks_like_zip, safe_join};
    use gtc::error::GtcError;
    #[cfg(unix)]
    use std::fs;
    use std::path::Path;

    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn archive_magic_detection_matches_expected_formats() {
        assert!(looks_like_zip(b"PK\x03\x04rest"));
        assert!(looks_like_gzip(&[0x1F, 0x8B, 0x08]));
        assert!(looks_like_squashfs(b"hsqsrest"));
        assert!(!looks_like_zip(b"notzip"));
        assert!(!looks_like_gzip(b"gz"));
        assert!(!looks_like_squashfs(b"sqsh"));
    }

    #[test]
    fn safe_join_rejects_parent_components() {
        let err = safe_join(Path::new("/tmp"), Path::new("../etc/passwd")).unwrap_err();
        assert!(matches!(err, GtcError::InvalidData { .. }));
        assert!(err.to_string().contains("unsafe path"));
    }

    #[test]
    fn safe_join_ignores_current_dir_components() {
        let joined = safe_join(Path::new("/tmp/base"), Path::new("./demo/file.txt")).unwrap();
        assert_eq!(joined, Path::new("/tmp/base/demo/file.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_no_symlink_ancestors_rejects_symlinked_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = tempfile::tempdir().expect("target");
        symlink(target.path(), dir.path().join("linked")).expect("symlink");

        let err = ensure_no_symlink_ancestors(dir.path(), &dir.path().join("linked/file.txt"))
            .unwrap_err();
        assert!(matches!(err, GtcError::InvalidData { .. }));
        assert!(err.to_string().contains("symlinked path"));
    }

    #[cfg(unix)]
    #[test]
    fn set_executable_if_unix_adds_exec_bits() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("tool");
        fs::write(&file, "echo hi").expect("write");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).expect("chmod");

        set_executable_if_unix(&file).expect("set executable");

        let mode = fs::metadata(&file).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
    }
}
