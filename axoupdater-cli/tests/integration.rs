use std::env::consts::EXE_SUFFIX;

use axoasset::LocalAsset;
use axoprocess::Cmd;
use camino::{Utf8Path, Utf8PathBuf};
use tempfile::TempDir;

static BIN: &str = env!("CARGO_BIN_EXE_axoupdater");
static RECEIPT_TEMPLATE: &str = r#"{"binaries":["axolotlsay"],"install_prefix":"INSTALL_PREFIX","provider":{"source":"cargo-dist","version":"CARGO_DIST_VERSION"},"source":{"app_name":"axolotlsay","name":"cargodisttest","owner":"mistydemeo","release_type":"github"},"version":"VERSION"}"#;

// Handle aarch64 later
fn triple() -> String {
    match std::env::consts::OS {
        "windows" => "x86_64-pc-windows-msvc".to_owned(),
        "macos" => {
            if std::env::consts::ARCH == "x86_64" {
                "x86_64-apple-darwin".to_owned()
            } else {
                "aarch64-apple-darwin".to_owned()
            }
        }
        "linux" => {
            if std::env::consts::ARCH == "x86_64" {
                "x86_64-unknown-linux-gnu".to_owned()
            } else {
                "aarch64-unknown-linux-gnu".to_owned()
            }
        }
        _ => unimplemented!(),
    }
}

fn axolotlsay_tarball_path(version: &str) -> String {
    let triple = triple();
    format!("https://github.com/mistydemeo/cargodisttest/releases/download/v{version}/axolotlsay-{triple}.tar.gz")
}

fn install_receipt(version: &str, cargo_dist_version: &str, prefix: &Utf8PathBuf) -> String {
    RECEIPT_TEMPLATE
        .replace("INSTALL_PREFIX", &prefix.to_string().replace('\\', "\\\\"))
        .replace("CARGO_DIST_VERSION", cargo_dist_version)
        .replace("VERSION", version)
}

fn write_receipt(
    version: &str,
    cargo_dist_version: &str,
    receipt_prefix: &Utf8PathBuf,
    install_prefix: &Utf8PathBuf,
) -> std::io::Result<()> {
    // Create the prefix in case it doesn't exist
    LocalAsset::create_dir_all(receipt_prefix).unwrap();

    let contents = install_receipt(version, cargo_dist_version, install_prefix);
    let receipt_name = receipt_prefix.join("axolotlsay-receipt.json");
    LocalAsset::write_new(&contents, receipt_name).unwrap();

    Ok(())
}

fn executable_tempdir() -> std::io::Result<TempDir> {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        tempfile::Builder::new()
            .prefix("axoupdater-exec")
            .tempdir_in(runtime_dir)
    } else {
        TempDir::new()
    }
}

#[test]
fn bails_out_with_default_name() {
    let mut command = Cmd::new(BIN, "execute axoupdater");
    command.check(false);
    let result = command.output().unwrap();
    assert!(!result.status.success());

    let stderr_string = String::from_utf8(result.stderr).unwrap();
    assert!(stderr_string.contains("App name calculated as `axoupdater'"));
}

// Performs an in-place upgrade from an old version to a newer one.
// The process runs like so:
// * Simulate an install of axolotlsay into a temporary directory
// * Write an install receipt to that path
// * Copy this repo's copy of axoupdater into the temporary directory in place of the one that axolotlsay once came with
// * Run axoupdater
// * Confirm that the new binary exists and is a newer version than the one we had before
//
// NOTE: axolotlsay 0.2.115 is a good base version to use because it contains a
//       several noteworthy bugfixes in its installer.
#[test]
fn test_upgrade() -> std::io::Result<()> {
    let tempdir = executable_tempdir()?;
    let bindir_path = &tempdir.path().join("bin");
    let bindir = Utf8Path::from_path(bindir_path).unwrap();
    std::fs::create_dir_all(bindir)?;

    let base_version = "0.2.115";

    let url = axolotlsay_tarball_path(base_version);
    let compressed_path =
        Utf8PathBuf::from_path_buf(tempdir.path().join("axolotlsay.tar.gz")).unwrap();

    let client = axoasset::AxoClient::with_reqwest(axoasset::reqwest::Client::new());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(client.load_and_write_to_file(&url, &compressed_path))
        .unwrap();

    // Write the receipt for the updater to use
    write_receipt(
        base_version,
        "0.11.1",
        &bindir.to_path_buf(),
        &bindir.to_path_buf(),
    )?;

    LocalAsset::untar_gz_all(&compressed_path, bindir).unwrap();

    // Now install our copy of the updater instead of the one axolotlsay came with
    let updater_path = bindir.join(format!("axolotlsay-update{EXE_SUFFIX}"));
    std::fs::copy(BIN, &updater_path)?;

    let mut updater = Cmd::new(&updater_path, "run updater");
    updater.env("AXOUPDATER_CONFIG_PATH", bindir);
    // If we're not running in CI, try to avoid ruining the user's PATH.
    if std::env::var("CI").is_err() {
        updater.env("INSTALLER_NO_MODIFY_PATH", "1");
        updater.env("AXOLOTLSAY_NO_MODIFY_PATH", "1");
    }
    // We'll do that manually
    updater.check(false);
    let res = updater.output().unwrap();
    let output_stdout = String::from_utf8(res.stdout).unwrap();
    let output_stderr = String::from_utf8(res.stderr).unwrap();

    // Now let's check the version we just updated to
    let new_axolotlsay_path = &bindir.join(format!("axolotlsay{EXE_SUFFIX}"));
    assert!(
        new_axolotlsay_path.exists(),
        "update result was\nstdout\n{}\nstderr\n{}",
        output_stdout,
        output_stderr
    );
    let mut new_axolotlsay = Cmd::new(new_axolotlsay_path, "version test");
    new_axolotlsay.arg("--version");
    let output = new_axolotlsay.output().unwrap();
    let stderr_string = String::from_utf8(output.stdout).unwrap();
    assert!(stderr_string.starts_with("axolotlsay "));
    assert_ne!(stderr_string, format!("axolotlsay {}\n", base_version));

    Ok(())
}

#[test]
fn test_upgrade_xdg_config_home() -> std::io::Result<()> {
    let tempdir = executable_tempdir()?;
    let bindir_path = &tempdir.path().join("bin");
    let bindir = Utf8Path::from_path(bindir_path).unwrap();
    std::fs::create_dir_all(bindir)?;
    let xdg_config_home = tempdir.path().join("config");
    let xdg_config_home = Utf8Path::from_path(&xdg_config_home).unwrap();

    let base_version = "0.2.115";

    let url = axolotlsay_tarball_path(base_version);
    let compressed_path =
        Utf8PathBuf::from_path_buf(tempdir.path().join("axolotlsay.tar.gz")).unwrap();

    let client = axoasset::AxoClient::with_reqwest(axoasset::reqwest::Client::new());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(client.load_and_write_to_file(&url, &compressed_path))
        .unwrap();

    // Write the receipt for the updater to use
    write_receipt(
        base_version,
        "0.11.1",
        &xdg_config_home.join("axolotlsay"),
        &bindir.to_path_buf(),
    )?;

    LocalAsset::untar_gz_all(&compressed_path, bindir).unwrap();

    // Now install our copy of the updater instead of the one axolotlsay came with
    let updater_path = bindir.join(format!("axolotlsay-update{EXE_SUFFIX}"));
    std::fs::copy(BIN, &updater_path)?;

    let mut updater = Cmd::new(&updater_path, "run updater");
    updater.env("XDG_CONFIG_HOME", xdg_config_home);
    // If we're not running in CI, try to avoid ruining the user's PATH.
    if std::env::var("CI").is_err() {
        updater.env("INSTALLER_NO_MODIFY_PATH", "1");
        updater.env("AXOLOTLSAY_NO_MODIFY_PATH", "1");
    }
    // We'll do that manually
    updater.check(false);
    let res = updater.output().unwrap();
    let output_stdout = String::from_utf8(res.stdout).unwrap();
    let output_stderr = String::from_utf8(res.stderr).unwrap();

    // Now let's check the version we just updated to
    let new_axolotlsay_path = &bindir.join(format!("axolotlsay{EXE_SUFFIX}"));
    assert!(
        new_axolotlsay_path.exists(),
        "update result was\nstdout\n{}\nstderr\n{}",
        output_stdout,
        output_stderr
    );
    let mut new_axolotlsay = Cmd::new(new_axolotlsay_path, "version test");
    new_axolotlsay.arg("--version");
    let output = new_axolotlsay.output().unwrap();
    let stderr_string = String::from_utf8(output.stdout).unwrap();
    assert!(stderr_string.starts_with("axolotlsay "));
    assert_ne!(stderr_string, format!("axolotlsay {}\n", base_version));

    Ok(())
}

#[test]
fn test_upgrade_allow_prerelease() -> std::io::Result<()> {
    let tempdir = executable_tempdir()?;
    let bindir_path = &tempdir.path().join("bin");
    let bindir = Utf8Path::from_path(bindir_path).unwrap();
    std::fs::create_dir_all(bindir)?;

    let base_version = "0.2.115";

    let url = axolotlsay_tarball_path(base_version);
    let compressed_path =
        Utf8PathBuf::from_path_buf(tempdir.path().join("axolotlsay.tar.gz")).unwrap();

    let client = axoasset::AxoClient::with_reqwest(axoasset::reqwest::Client::new());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(client.load_and_write_to_file(&url, &compressed_path))
        .unwrap();

    // Write the receipt for the updater to use
    write_receipt(
        base_version,
        "0.11.1",
        &bindir.to_path_buf(),
        &bindir.to_path_buf(),
    )?;

    LocalAsset::untar_gz_all(&compressed_path, bindir).unwrap();

    // Now install our copy of the updater instead of the one axolotlsay came with
    let updater_path = bindir.join(format!("axolotlsay-update{EXE_SUFFIX}"));
    std::fs::copy(BIN, &updater_path)?;

    let mut updater = Cmd::new(&updater_path, "run updater");
    // If we're not running in CI, try to avoid ruining the user's PATH.
    if std::env::var("CI").is_err() {
        updater.env("INSTALLER_NO_MODIFY_PATH", "1");
        updater.env("AXOLOTLSAY_NO_MODIFY_PATH", "1");
    }
    updater.env("AXOUPDATER_CONFIG_PATH", bindir);
    updater.arg("--prerelease");
    // We'll do that manually
    updater.check(false);
    let res = updater.output().unwrap();
    let output_stdout = String::from_utf8(res.stdout).unwrap();
    let output_stderr = String::from_utf8(res.stderr).unwrap();

    // Now let's check the version we just updated to
    let new_axolotlsay_path = &bindir.join(format!("axolotlsay{EXE_SUFFIX}"));
    assert!(
        new_axolotlsay_path.exists(),
        "update result was\nstdout\n{}\nstderr\n{}",
        output_stdout,
        output_stderr
    );
    let mut new_axolotlsay = Cmd::new(new_axolotlsay_path, "version test");
    new_axolotlsay.arg("--version");
    let output = new_axolotlsay.output().unwrap();
    let stderr_string = String::from_utf8(output.stdout).unwrap();
    assert!(stderr_string.starts_with("axolotlsay "));
    assert_ne!(stderr_string, format!("axolotlsay {}\n", base_version));

    Ok(())
}

// A similar test to the one above, but it upgrades to a specific version
// instead of whatever's latest.
#[test]
fn test_upgrade_to_specific_version() -> std::io::Result<()> {
    let tempdir = TempDir::new()?;
    let executable_tmp = executable_tempdir()?;
    let bindir_path = &executable_tmp.path().join("bin");
    let bindir = Utf8Path::from_path(bindir_path).unwrap();
    std::fs::create_dir_all(bindir)?;

    let base_version = "0.2.115";
    let target_version = "0.2.116";

    let url = axolotlsay_tarball_path(base_version);
    let compressed_path =
        Utf8PathBuf::from_path_buf(tempdir.path().join("axolotlsay.tar.gz")).unwrap();

    let client = axoasset::AxoClient::with_reqwest(axoasset::reqwest::Client::new());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(client.load_and_write_to_file(&url, &compressed_path))
        .unwrap();

    // Write the receipt for the updater to use
    write_receipt(
        base_version,
        "0.11.1",
        &bindir.to_path_buf(),
        &bindir.to_path_buf(),
    )?;

    LocalAsset::untar_gz_all(&compressed_path, bindir).unwrap();

    // Now install our copy of the updater instead of the one axolotlsay came with
    let updater_path = bindir.join(format!("axolotlsay-update{EXE_SUFFIX}"));
    std::fs::copy(BIN, &updater_path)?;

    let mut updater = Cmd::new(&updater_path, "run updater");
    updater.arg("--version").arg(target_version);
    updater.env("AXOUPDATER_CONFIG_PATH", bindir);
    // If we're not running in CI, try to avoid ruining the user's PATH.
    if std::env::var("CI").is_err() {
        updater.env("INSTALLER_NO_MODIFY_PATH", "1");
        updater.env("AXOLOTLSAY_NO_MODIFY_PATH", "1");
    }
    // We'll do that manually
    updater.check(false);
    let _res = updater.output().unwrap();

    // Now let's check the version we just updated to
    let new_axolotlsay_path = &bindir.join(format!("axolotlsay{EXE_SUFFIX}"));
    assert!(new_axolotlsay_path.exists());
    let mut new_axolotlsay = Cmd::new(new_axolotlsay_path, "version test");
    new_axolotlsay.arg("--version");
    let output = new_axolotlsay.output().unwrap();
    let stderr_string = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stderr_string, format!("axolotlsay {}\n", target_version));

    Ok(())
}

// A similar test to the one above, but it actually downgrades to an older
// version on request instead of upgrading.
#[test]
fn test_downgrade_to_specific_version() -> std::io::Result<()> {
    let tempdir = executable_tempdir()?;
    let bindir_path = &tempdir.path().join("bin");
    let bindir = Utf8Path::from_path(bindir_path).unwrap();
    std::fs::create_dir_all(bindir)?;

    let base_version = "0.2.116";
    let target_version = "0.2.115";

    let url = axolotlsay_tarball_path(base_version);
    let compressed_path =
        Utf8PathBuf::from_path_buf(tempdir.path().join("axolotlsay.tar.gz")).unwrap();

    let client = axoasset::AxoClient::with_reqwest(axoasset::reqwest::Client::new());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(client.load_and_write_to_file(&url, &compressed_path))
        .unwrap();

    // Write the receipt for the updater to use
    write_receipt(
        base_version,
        "0.11.1",
        &bindir.to_path_buf(),
        &bindir.to_path_buf(),
    )?;

    LocalAsset::untar_gz_all(&compressed_path, bindir).unwrap();

    // Now install our copy of the updater instead of the one axolotlsay came with
    let updater_path = bindir.join(format!("axolotlsay-update{EXE_SUFFIX}"));
    std::fs::copy(BIN, &updater_path)?;

    let mut updater = Cmd::new(&updater_path, "run updater");
    updater.arg("--version").arg(target_version);
    updater.env("AXOUPDATER_CONFIG_PATH", bindir);
    // If we're not running in CI, try to avoid ruining the user's PATH.
    if std::env::var("CI").is_err() {
        updater.env("INSTALLER_NO_MODIFY_PATH", "1");
        updater.env("AXOLOTLSAY_NO_MODIFY_PATH", "1");
    }
    // We'll do that manually
    updater.check(false);
    let _res = updater.output().unwrap();

    // Now let's check the version we just updated to
    let new_axolotlsay_path = &bindir.join(format!("axolotlsay{EXE_SUFFIX}"));
    assert!(new_axolotlsay_path.exists());
    let mut new_axolotlsay = Cmd::new(new_axolotlsay_path, "version test");
    new_axolotlsay.arg("--version");
    let output = new_axolotlsay.output().unwrap();
    let stderr_string = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stderr_string, format!("axolotlsay {}\n", target_version));

    Ok(())
}

// A similar test to the one above, but it upgrades to a significantly older
// version that's outside the GitHub API's 30 versions.
// This version isn't available for every target, so we only run it for
// certain target triples.
#[test]
fn test_downgrade_to_specific_old_version() -> std::io::Result<()> {
    // Only available for x86_64 Darwin and x86_64 Linux
    match std::env::consts::OS {
        "linux" | "macos" => {
            if std::env::consts::ARCH != "x86_64" {
                return Ok(());
            }
        }
        "windows" => return Ok(()),
        _ => return Ok(()),
    }

    let tempdir = executable_tempdir()?;
    let bindir_path = &tempdir.path().join("bin");
    let bindir = Utf8Path::from_path(bindir_path).unwrap();
    std::fs::create_dir_all(bindir)?;

    let base_version = "0.2.116";
    let target_version = "0.2.50";

    let url = axolotlsay_tarball_path(base_version);
    let compressed_path =
        Utf8PathBuf::from_path_buf(tempdir.path().join("axolotlsay.tar.gz")).unwrap();

    let client = axoasset::AxoClient::with_reqwest(axoasset::reqwest::Client::new());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(client.load_and_write_to_file(&url, &compressed_path))
        .unwrap();

    // Write the receipt for the updater to use
    write_receipt(
        base_version,
        "0.11.1",
        &bindir.to_path_buf(),
        &bindir.to_path_buf(),
    )?;

    LocalAsset::untar_gz_all(&compressed_path, bindir).unwrap();

    // Now install our copy of the updater instead of the one axolotlsay came with
    let updater_path = bindir.join(format!("axolotlsay-update{EXE_SUFFIX}"));
    std::fs::copy(BIN, &updater_path)?;

    let mut updater = Cmd::new(&updater_path, "run updater");
    updater.arg("--version").arg(target_version);
    updater.env("AXOUPDATER_CONFIG_PATH", bindir);
    // If we're not running in CI, try to avoid ruining the user's PATH.
    if std::env::var("CI").is_err() {
        updater.env("INSTALLER_NO_MODIFY_PATH", "1");
        updater.env("AXOLOTLSAY_NO_MODIFY_PATH", "1");
    }
    // This installer is so old it doesn't respect the install path, so we
    // have to set CARGO_HOME to force it.
    updater.env("CARGO_HOME", tempdir.path());
    // We'll do that manually
    updater.check(false);
    let _res = updater.output().unwrap();

    // Now let's check the version we just updated to
    let new_axolotlsay_path = &bindir.join(format!("axolotlsay{EXE_SUFFIX}"));
    assert!(new_axolotlsay_path.exists());
    let mut new_axolotlsay = Cmd::new(new_axolotlsay_path, "version test");
    new_axolotlsay.arg("--version");
    let output = new_axolotlsay.output().unwrap();
    let stderr_string = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stderr_string, format!("axolotlsay {}\n", target_version));

    Ok(())
}

// Similar to `test_upgrade` but tests releases created after the
// cargo-dist receipt prefix bug was fixed; see:
// https://github.com/axodotdev/cargo-dist/pull/1037
#[test]
fn test_upgrade_from_prefix_with_no_bin() -> std::io::Result<()> {
    let tempdir = executable_tempdir()?;
    let prefix = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
    let bindir_path = &tempdir.path().join("bin");
    let bindir = Utf8Path::from_path(bindir_path).unwrap();
    std::fs::create_dir_all(bindir)?;

    // The first cargodisttest release with the "/bin" bug fixed
    let base_version = "0.2.133";

    let url = axolotlsay_tarball_path(base_version);
    let compressed_path =
        Utf8PathBuf::from_path_buf(tempdir.path().join("axolotlsay.tar.gz")).unwrap();

    let client = axoasset::AxoClient::with_reqwest(axoasset::reqwest::Client::new());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(client.load_and_write_to_file(&url, &compressed_path))
        .unwrap();

    // Write the receipt for the updater to use
    // 0.15.0 is the first cargo-dist that published fixed installers for the
    // /bin bug mentioned above
    write_receipt(base_version, "0.15.0", &prefix, &prefix)?;

    LocalAsset::untar_gz_all(&compressed_path, bindir).unwrap();

    // Now install our copy of the updater instead of the one axolotlsay came with
    let updater_path = bindir.join(format!("axolotlsay-update{EXE_SUFFIX}"));
    std::fs::copy(BIN, &updater_path)?;

    let mut updater = Cmd::new(&updater_path, "run updater");
    updater.env("AXOUPDATER_CONFIG_PATH", prefix);
    // If we're not running in CI, try to avoid ruining the user's PATH.
    if std::env::var("CI").is_err() {
        updater.env("INSTALLER_NO_MODIFY_PATH", "1");
        updater.env("AXOLOTLSAY_NO_MODIFY_PATH", "1");
    }
    // We'll do that manually
    updater.check(false);
    let res = updater.output().unwrap();

    let output_stdout = String::from_utf8(res.stdout).unwrap();
    let output_stderr = String::from_utf8(res.stderr).unwrap();

    // Now let's check the version we just updated to
    let new_axolotlsay_path = &bindir.join(format!("axolotlsay{EXE_SUFFIX}"));
    assert!(
        new_axolotlsay_path.exists(),
        "update result was\nstdout\n{}\nstderr\n{}",
        output_stdout,
        output_stderr
    );
    let mut new_axolotlsay = Cmd::new(new_axolotlsay_path, "version test");
    new_axolotlsay.arg("--version");
    let output = new_axolotlsay.output().unwrap();
    let stderr_string = String::from_utf8(output.stdout).unwrap();
    assert!(stderr_string.starts_with("axolotlsay "));
    assert_ne!(stderr_string, format!("axolotlsay {}\n", base_version));

    Ok(())
}
