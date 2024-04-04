use std::env::consts::EXE_SUFFIX;
use std::io::Read;

use axoasset::LocalAsset;
use axoprocess::Cmd;
use camino::{Utf8Path, Utf8PathBuf};
use temp_dir::TempDir;

static BIN: &str = env!("CARGO_BIN_EXE_axoupdater");
static RECEIPT_TEMPLATE: &str = r#"{"binaries":["axolotlsay"],"install_prefix":"INSTALL_PREFIX","provider":{"source":"cargo-dist","version":"0.11.1"},"source":{"app_name":"axolotlsay","name":"cargodisttest","owner":"mistydemeo","release_type":"github"},"version":"VERSION"}"#;

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

fn install_receipt(version: &str, prefix: &Utf8PathBuf) -> String {
    RECEIPT_TEMPLATE
        .replace("INSTALL_PREFIX", &prefix.to_string().replace('\\', "\\\\"))
        .replace("VERSION", version)
}

fn write_receipt(version: &str, prefix: &Utf8PathBuf) -> std::io::Result<()> {
    let contents = install_receipt(version, prefix);
    let receipt_name = prefix.join("axolotlsay-receipt.json");
    LocalAsset::write_new(&contents, receipt_name).unwrap();

    Ok(())
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
    let tempdir = TempDir::new()?;
    let bindir_path = &tempdir.path().join("bin");
    let bindir = Utf8Path::from_path(bindir_path).unwrap();
    std::fs::create_dir_all(bindir)?;

    let base_version = "0.2.115";

    let url = axolotlsay_tarball_path(base_version);

    let mut response = reqwest::blocking::get(url)
        .unwrap()
        .error_for_status()
        .unwrap();

    let compressed_path =
        Utf8PathBuf::from_path_buf(tempdir.path().join("axolotlsay.tar.gz")).unwrap();
    let mut contents = vec![];
    response.read_to_end(&mut contents)?;
    std::fs::write(&compressed_path, contents)?;

    // Write the receipt for the updater to use
    write_receipt(base_version, &bindir.to_path_buf())?;

    LocalAsset::untar_gz_all(&compressed_path, bindir).unwrap();

    // Now install our copy of the updater instead of the one axolotlsay came with
    let updater_path = bindir.join(format!("axolotlsay-update{EXE_SUFFIX}"));
    std::fs::copy(BIN, &updater_path)?;

    let mut updater = Cmd::new(&updater_path, "run updater");
    updater.env("AXOUPDATER_CONFIG_PATH", bindir);
    // We'll do that manually
    updater.check(false);
    let result = updater.output();
    assert!(result.is_ok());
    let res = result.unwrap();
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
    let bindir_path = &tempdir.path().join("bin");
    let bindir = Utf8Path::from_path(bindir_path).unwrap();
    std::fs::create_dir_all(bindir)?;

    let base_version = "0.2.115";
    let target_version = "0.2.116";

    let url = axolotlsay_tarball_path(base_version);

    let mut response = reqwest::blocking::get(url)
        .unwrap()
        .error_for_status()
        .unwrap();

    let compressed_path =
        Utf8PathBuf::from_path_buf(tempdir.path().join("axolotlsay.tar.gz")).unwrap();
    let mut contents = vec![];
    response.read_to_end(&mut contents)?;
    std::fs::write(&compressed_path, contents)?;

    // Write the receipt for the updater to use
    write_receipt(base_version, &bindir.to_path_buf())?;

    LocalAsset::untar_gz_all(&compressed_path, bindir).unwrap();

    // Now install our copy of the updater instead of the one axolotlsay came with
    let updater_path = bindir.join(format!("axolotlsay-update{EXE_SUFFIX}"));
    std::fs::copy(BIN, &updater_path)?;

    let mut updater = Cmd::new(&updater_path, "run updater");
    updater.arg("--version").arg(target_version);
    updater.env("AXOUPDATER_CONFIG_PATH", bindir);
    // We'll do that manually
    updater.check(false);
    let result = updater.output();
    assert!(result.is_ok());

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
    let tempdir = TempDir::new()?;
    let bindir_path = &tempdir.path().join("bin");
    let bindir = Utf8Path::from_path(bindir_path).unwrap();
    std::fs::create_dir_all(bindir)?;

    let base_version = "0.2.116";
    let target_version = "0.2.115";

    let url = axolotlsay_tarball_path(base_version);

    let mut response = reqwest::blocking::get(url)
        .unwrap()
        .error_for_status()
        .unwrap();

    let compressed_path =
        Utf8PathBuf::from_path_buf(tempdir.path().join("axolotlsay.tar.gz")).unwrap();
    let mut contents = vec![];
    response.read_to_end(&mut contents)?;
    std::fs::write(&compressed_path, contents)?;

    // Write the receipt for the updater to use
    write_receipt(base_version, &bindir.to_path_buf())?;

    LocalAsset::untar_gz_all(&compressed_path, bindir).unwrap();

    // Now install our copy of the updater instead of the one axolotlsay came with
    let updater_path = bindir.join(format!("axolotlsay-update{EXE_SUFFIX}"));
    std::fs::copy(BIN, &updater_path)?;

    let mut updater = Cmd::new(&updater_path, "run updater");
    updater.arg("--version").arg(target_version);
    updater.env("AXOUPDATER_CONFIG_PATH", bindir);
    // We'll do that manually
    updater.check(false);
    let result = updater.output();
    assert!(result.is_ok());

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
