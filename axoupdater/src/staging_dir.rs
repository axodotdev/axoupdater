//! Installer staging directory selection and lifecycle helpers.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Stdio,
};

#[cfg(unix)]
use std::{os::unix::fs::PermissionsExt, process::Command};

use directories::BaseDirs;
use tempfile::TempDir;

use crate::errors::AxoupdateResult;

/// Selects an executable staging tempdir for installer downloads.
///
/// Tries runtime/cache/data-local roots first, then falls back to the
/// process-global temporary directory.
pub(crate) fn select_installer_tempdir(parent_name: &str) -> AxoupdateResult<TempDir> {
    let name_prefix = tempdir_name_prefix(parent_name);
    for candidate in prioritized_staging_roots() {
        if let Ok(tempdir) = tempfile::Builder::new()
            .prefix(&name_prefix)
            .tempdir_in(candidate)
        {
            if can_execute_from_dir(tempdir.path()) {
                return Ok(tempdir);
            }
        }
    }

    let tempdir = TempDir::new()?;
    if can_execute_from_dir(tempdir.path()) {
        Ok(tempdir)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "unable to find an executable temp directory",
        )
        .into())
    }
}

fn prioritized_staging_roots() -> Vec<PathBuf> {
    let Some(base_dirs) = BaseDirs::new() else {
        return Vec::new();
    };

    let mut dirs = Vec::new();

    if let Some(runtime_dir) = runtime_dir_with_unix_fallback(&base_dirs) {
        // On Linux, XDG says runtime dir "must" exist, be user owned, user-session scoped,
        // and be 700 (i.e. executable). Generally seems most appropriate.
        dirs.push(runtime_dir);
    }
    dirs.push(base_dirs.cache_dir().to_path_buf());
    dirs.push(base_dirs.data_local_dir().to_path_buf());

    dirs
}

fn tempdir_name_prefix(parent_name: &str) -> String {
    format!("{}-axoupdate-", sanitized_parent_component(parent_name))
}

fn sanitized_parent_component(parent_name: &str) -> String {
    let mut safe = parent_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if safe.is_empty() || safe.chars().all(|ch| ch == '.') {
        safe = "axoupdater".to_owned();
    }
    safe
}

fn runtime_dir_with_unix_fallback(base_dirs: &BaseDirs) -> Option<PathBuf> {
    if let Some(runtime_dir) = base_dirs.runtime_dir() {
        return Some(runtime_dir.to_path_buf());
    }

    #[cfg(unix)]
    {
        if env::var_os("XDG_RUNTIME_DIR").is_none() {
            let uid = unsafe { libc::geteuid() };
            let fallback = PathBuf::from(format!("/run/user/{uid}"));
            if fallback.is_dir() {
                return Some(fallback);
            }
        }
    }

    None
}

#[cfg(unix)]
fn can_execute_from_dir(dir: &Path) -> bool {
    let script_path = dir.join(format!("exec-probe-{}.sh", std::process::id()));
    let script_contents = "#!/bin/sh\nexit 0\n";

    if fs::write(&script_path, script_contents).is_err() {
        return false;
    }

    let chmod_result = fs::set_permissions(&script_path, fs::Permissions::from_mode(0o700));
    if chmod_result.is_err() {
        let _ = fs::remove_file(&script_path);
        return false;
    }

    let ran = Command::new(&script_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());

    let _ = fs::remove_file(&script_path);
    ran
}

#[cfg(not(unix))]
fn can_execute_from_dir(dir: &Path) -> bool {
    dir.is_dir()
}

#[cfg(test)]
mod tests {
    use directories::BaseDirs;

    #[cfg(unix)]
    use std::{fs, os::unix::fs::PermissionsExt};

    use super::{
        prioritized_staging_roots, runtime_dir_with_unix_fallback, sanitized_parent_component,
        tempdir_name_prefix,
    };

    #[test]
    fn test_staging_dir_priorities_follow_base_dirs_order() {
        let Some(base_dirs) = BaseDirs::new() else {
            return;
        };
        let mut expected = Vec::new();
        if let Some(runtime_dir) = runtime_dir_with_unix_fallback(&base_dirs) {
            expected.push(runtime_dir);
        }
        expected.push(base_dirs.cache_dir().to_path_buf());
        expected.push(base_dirs.data_local_dir().to_path_buf());

        let actual = prioritized_staging_roots();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parent_component_is_sanitized() {
        assert_eq!(
            sanitized_parent_component("../parent/workspace"),
            ".._parent_workspace"
        );
        assert_eq!(sanitized_parent_component(""), "axoupdater");
    }

    #[test]
    fn test_tempdir_name_prefix_shape() {
        assert_eq!(
            tempdir_name_prefix("parent-workspace"),
            "parent-workspace-axoupdate-".to_owned()
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_can_execute_from_dir_detects_unusable_permissions() {
        let parent = tempfile::tempdir().expect("parent tempdir");
        let no_exec = parent.path().join("noexec");
        fs::create_dir(&no_exec).expect("create test directory");
        fs::set_permissions(&no_exec, fs::Permissions::from_mode(0o600)).expect("chmod test dir");

        assert!(!super::can_execute_from_dir(&no_exec));
    }
}
