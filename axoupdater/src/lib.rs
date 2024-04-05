#![deny(missing_docs)]

//! axoupdater crate

use std::{
    env::{self, args, current_dir, current_exe},
    path::PathBuf,
    process::Stdio,
};

#[cfg(unix)]
use std::{fs::File, os::unix::fs::PermissionsExt};

use axoasset::{AxoassetError, LocalAsset, SourceFile};
use axoprocess::{AxoprocessError, Cmd};
use camino::Utf8PathBuf;
#[cfg(feature = "axo_releases")]
use gazenot::{error::GazenotError, Gazenot};
use miette::Diagnostic;
#[cfg(feature = "github_releases")]
use reqwest::{
    self,
    header::{ACCEPT, USER_AGENT},
};
use serde::Deserialize;
use temp_dir::TempDir;
use thiserror::Error;

/// Provides information about the result of the upgrade operation
pub struct UpdateResult {
    /// The old version (pre-upgrade)
    pub old_version: String,
    /// The new version (post-upgrade)
    pub new_version: String,
    /// The tag the new version was created from
    pub new_version_tag: String,
    /// The root that the new version was installed to
    /// NOTE: This is a prediction, and the underlying installer may ignore it
    /// if it's out of date. Installers built with cargo-dist 0.12.0 or later
    /// will definitively use this value.
    pub install_prefix: Utf8PathBuf,
}

/// Used to specify what version to upgrade to
#[derive(Clone)]
pub enum UpdateRequest {
    /// Always update to the latest
    Latest,
    /// Upgrade (or downgrade) to this specific version
    SpecificVersion(String),
    /// Upgrade (or downgrade) to this specific tag
    SpecificTag(String),
}

/// Struct representing an updater process
pub struct AxoUpdater {
    /// The name of the program to update, if specified
    pub name: Option<String>,
    /// Information about where updates should be fetched from
    pub source: Option<ReleaseSource>,
    /// What version should be updated to
    version_specifier: UpdateRequest,
    /// Information about the latest release; used to determine if an update is needed
    requested_release: Option<Release>,
    /// The current version number
    current_version: Option<String>,
    /// Information about the install prefix of the previous version
    install_prefix: Option<Utf8PathBuf>,
    /// Whether to display the underlying installer's stdout
    print_installer_stdout: bool,
    /// Whether to display the underlying installer's stderr
    print_installer_stderr: bool,
}

impl Default for AxoUpdater {
    fn default() -> Self {
        Self::new()
    }
}

impl AxoUpdater {
    /// Creates a new, empty AxoUpdater struct. This struct lacks information
    /// necessary to perform the update, so at least the name and source fields
    /// will need to be filled in before the update can run.
    pub fn new() -> AxoUpdater {
        AxoUpdater {
            name: None,
            source: None,
            version_specifier: UpdateRequest::Latest,
            requested_release: None,
            current_version: None,
            install_prefix: None,
            print_installer_stdout: true,
            print_installer_stderr: true,
        }
    }

    /// Creates a new AxoUpdater struct with an explicitly-specified name.
    pub fn new_for(app_name: &str) -> AxoUpdater {
        AxoUpdater {
            name: Some(app_name.to_owned()),
            source: None,
            version_specifier: UpdateRequest::Latest,
            requested_release: None,
            current_version: None,
            install_prefix: None,
            print_installer_stdout: true,
            print_installer_stderr: true,
        }
    }

    /// Creates a new AxoUpdater struct by attempting to autodetect the name
    /// of the current executable. This is only meant to be used by standalone
    /// updaters, not when this crate is used as a library in another program.
    pub fn new_for_updater_executable() -> AxoupdateResult<AxoUpdater> {
        let Some(app_name) = get_app_name() else {
            return Err(AxoupdateError::NoAppName {});
        };

        // Happens if the binary didn't get renamed properly
        if app_name == "axoupdater" {
            return Err(AxoupdateError::UpdateSelf {});
        };

        Ok(AxoUpdater {
            name: Some(app_name.to_owned()),
            source: None,
            version_specifier: UpdateRequest::Latest,
            requested_release: None,
            current_version: None,
            install_prefix: None,
            print_installer_stdout: true,
            print_installer_stderr: true,
        })
    }

    /// Attempts to load an install receipt in order to prepare for an update.
    /// If present and valid, the install receipt is used to populate the
    /// `source` and `current_version` fields.
    /// Shell and Powershell installers produced by cargo-dist since 0.9.0
    /// will have created an install receipt.
    pub fn load_receipt(&mut self) -> AxoupdateResult<&mut AxoUpdater> {
        let Some(app_name) = &self.name else {
            return Err(AxoupdateError::NoAppNamePassed {});
        };

        let receipt = load_receipt_for(app_name)?;

        self.source = Some(receipt.source.clone());
        self.current_version = Some(receipt.version.to_owned());
        self.install_prefix = Some(receipt.install_prefix.to_owned());

        Ok(self)
    }

    /// Explicitly specifies the current version.
    pub fn set_current_version(&mut self, version: &str) -> AxoupdateResult<&mut AxoUpdater> {
        self.current_version = Some(version.to_owned());

        Ok(self)
    }

    /// Enables printing the underlying installer's stdout.
    pub fn enable_installer_stdout(&mut self) -> &mut AxoUpdater {
        self.print_installer_stdout = true;

        self
    }

    /// Disables printing the underlying installer's stdout.
    pub fn disable_installer_stdout(&mut self) -> &mut AxoUpdater {
        self.print_installer_stdout = false;

        self
    }

    /// Enables printing the underlying installer's stderr.
    pub fn enable_installer_stderr(&mut self) -> &mut AxoUpdater {
        self.print_installer_stderr = true;

        self
    }

    /// Disables printing the underlying installer's stderr.
    pub fn disable_installer_stderr(&mut self) -> &mut AxoUpdater {
        self.print_installer_stderr = false;

        self
    }

    /// Enables all output for the underlying installer.
    pub fn enable_installer_output(&mut self) -> &mut AxoUpdater {
        self.print_installer_stdout = true;
        self.print_installer_stderr = true;

        self
    }

    /// Disables all output for the underlying installer.
    pub fn disable_installer_output(&mut self) -> &mut AxoUpdater {
        self.print_installer_stdout = false;
        self.print_installer_stderr = false;

        self
    }

    /// Configures axoupdater's update strategy, replacing whatever was
    /// previously configured with the strategy in `version_specifier`.
    pub fn configure_version_specifier(
        &mut self,
        version_specifier: UpdateRequest,
    ) -> &mut AxoUpdater {
        self.version_specifier = version_specifier;

        self
    }

    /// Checks to see if the loaded install receipt is for this executable.
    /// Used to guard against cases where the running EXE is from a package
    /// manager, but a receipt from a shell installed-copy is present on the
    /// system.
    /// Returns an error if the receipt hasn't been loaded yet.
    pub fn check_receipt_is_for_this_executable(&self) -> AxoupdateResult<bool> {
        let current_exe_path = Utf8PathBuf::from_path_buf(current_exe()?.canonicalize()?)
            .map_err(|path| AxoupdateError::CaminoConversionFailed { path })?;
        // First determine the parent dir
        let mut current_exe_root = if let Some(parent) = current_exe_path.parent() {
            parent.to_path_buf()
        } else {
            current_exe_path
        };
        // If the parent dir is a "bin" dir, strip it to get the true root
        if current_exe_root.file_name() == Some("bin") {
            if let Some(parent) = current_exe_root.parent() {
                current_exe_root = parent.to_path_buf();
            }
        }

        // Looks like this EXE comes from a different source than the install
        // receipt
        if current_exe_root != self.install_prefix_root_normalized()? {
            return Ok(false);
        }

        Ok(true)
    }

    /// Determines if an update is needed by querying the newest version from
    /// the location specified in `source`.
    /// This includes a blocking network call, so it may be slow.
    /// This can only be performed if the `current_version` field has been
    /// set, either by loading the install receipt or by specifying it using
    /// `set_current_version`.
    /// Note that this also checks to see if the current executable is
    /// *eligible* for updates, by checking to see if it's the executable
    /// that the install receipt is for. In the case that the executable comes
    /// from a different source, it will return before the network call for a
    /// new version.
    pub async fn is_update_needed(&mut self) -> AxoupdateResult<bool> {
        if !self.check_receipt_is_for_this_executable()? {
            return Ok(false);
        }

        let Some(current_version) = self.current_version.to_owned() else {
            return Err(AxoupdateError::NotConfigured {
                missing_field: "current_version".to_owned(),
            });
        };

        let release = match &self.requested_release {
            Some(r) => r,
            None => {
                self.fetch_release().await?;
                self.requested_release.as_ref().unwrap()
            }
        };

        Ok(current_version != release.version())
    }

    #[cfg(feature = "blocking")]
    /// Identical to Axoupdater::is_update_needed(), but performed synchronously.
    pub fn is_update_needed_sync(&mut self) -> AxoupdateResult<bool> {
        tokio::runtime::Builder::new_current_thread()
            .worker_threads(1)
            .max_blocking_threads(128)
            .enable_all()
            .build()
            .expect("Initializing tokio runtime failed")
            .block_on(self.is_update_needed())
    }

    /// Returns the root of the install prefix, stripping the final `/bin`
    /// component if necessary. Works around a bug introduced in cargo-dist
    /// where this field was returned inconsistently in receipts for a few
    /// versions.
    fn install_prefix_root(&self) -> AxoupdateResult<Utf8PathBuf> {
        let Some(install_prefix) = &self.install_prefix else {
            return Err(AxoupdateError::NotConfigured {
                missing_field: "install_prefix".to_owned(),
            });
        };

        let mut install_root = install_prefix.to_owned();
        if install_root.file_name() == Some("bin") {
            if let Some(parent) = install_root.parent() {
                install_root = parent.to_path_buf();
            }
        }

        Ok(install_root)
    }

    /// Returns a normalized version of install_prefix_root, for comparison
    fn install_prefix_root_normalized(&self) -> AxoupdateResult<Utf8PathBuf> {
        let raw_root = self.install_prefix_root()?;
        let normalized = Utf8PathBuf::from_path_buf(raw_root.canonicalize()?)
            .map_err(|path| AxoupdateError::CaminoConversionFailed { path })?;
        Ok(normalized)
    }

    /// Attempts to perform an update. The return value specifies whether an
    /// update was actually performed or not; false indicates "no update was
    /// needed", while an error indicates that an update couldn't be performed
    /// due to an error.
    pub async fn run(&mut self) -> AxoupdateResult<Option<UpdateResult>> {
        if !self.is_update_needed().await? {
            return Ok(None);
        }

        let release = match &self.requested_release {
            Some(r) => r,
            None => {
                self.fetch_release().await?;
                self.requested_release.as_ref().unwrap()
            }
        };
        let tempdir = TempDir::new()?;

        let installer_url = match env::consts::OS {
            "macos" | "linux" => release
                .assets
                .iter()
                .find(|asset| asset.name.ends_with("-installer.sh")),
            "windows" => release
                .assets
                .iter()
                .find(|asset| asset.name.ends_with("-installer.ps1")),
            _ => unreachable!(),
        };

        let installer_url = if let Some(installer_url) = installer_url {
            installer_url
        } else {
            return Err(AxoupdateError::NoInstallerForPackage {});
        };

        let extension = if cfg!(windows) { ".ps1" } else { ".sh" };
        let installer_path =
            Utf8PathBuf::try_from(tempdir.path().join(format!("installer{extension}")))?;

        #[cfg(unix)]
        {
            let installer_file = File::create(&installer_path)?;
            let mut perms = installer_file.metadata()?.permissions();
            perms.set_mode(0o744);
            installer_file.set_permissions(perms)?;
        }

        let client = reqwest::Client::new();
        let download = client
            .get(&installer_url.browser_download_url)
            .header(ACCEPT, "application/octet-stream")
            .send()
            .await?
            .text()
            .await?;

        LocalAsset::write_new_all(&download, &installer_path)?;

        // Before we update, move ourselves to a temporary directory.
        // This is necessary because Windows won't let an actively-running
        // executable be overwritten.
        // If the update fails, we'll move it back to where it was before
        // we began the update process.
        // NOTE: this TempDir needs to be held alive for the whole function.
        let temp_root;
        let to_restore = if cfg!(target_family = "windows") {
            temp_root = TempDir::new()?;
            let old_path = std::env::current_exe()?;
            let old_filename = old_path.file_name().expect("current binary has no name!?");
            let ourselves = temp_root.path().join(old_filename);
            std::fs::rename(&old_path, &ourselves)?;

            Some((ourselves, old_path))
        } else {
            None
        };

        let path = if cfg!(windows) {
            "powershell"
        } else {
            installer_path.as_str()
        };
        let mut command = Cmd::new(path, "execute installer");
        if cfg!(windows) {
            command.arg(&installer_path);
        }
        if !self.print_installer_stdout {
            command.stdout(Stdio::null());
        }
        if !self.print_installer_stderr {
            command.stderr(Stdio::null());
        }
        // On Windows, fixes a bug that occurs if the parent process is
        // PowerShell Core.
        // https://github.com/PowerShell/PowerShell/issues/18530
        command.env_remove("PSModulePath");
        let install_prefix = self.install_prefix_root()?;
        // Forces the generated installer to install to exactly this path,
        // regardless of how it's configured to install.
        command.env("CARGO_DIST_FORCE_INSTALL_DIR", &install_prefix);
        let result = command.run();

        if result.is_err() {
            if let Some((ourselves, old_path)) = to_restore {
                std::fs::rename(ourselves, old_path)?;
            }
        }

        result?;

        let result = UpdateResult {
            old_version: self
                .current_version
                .to_owned()
                .unwrap_or("unable to determine".to_owned()),
            new_version: release.version(),
            new_version_tag: release.tag_name.to_owned(),
            install_prefix,
        };

        Ok(Some(result))
    }

    #[cfg(feature = "blocking")]
    /// Identical to Axoupdater::run(), but performed synchronously.
    pub fn run_sync(&mut self) -> AxoupdateResult<Option<UpdateResult>> {
        tokio::runtime::Builder::new_current_thread()
            .worker_threads(1)
            .max_blocking_threads(128)
            .enable_all()
            .build()
            .expect("Initializing tokio runtime failed")
            .block_on(self.run())
    }

    async fn fetch_release(&mut self) -> AxoupdateResult<()> {
        let Some(app_name) = &self.name else {
            return Err(AxoupdateError::NotConfigured {
                missing_field: "app_name".to_owned(),
            });
        };
        let Some(source) = &self.source else {
            return Err(AxoupdateError::NotConfigured {
                missing_field: "source".to_owned(),
            });
        };

        let release = match self.version_specifier.to_owned() {
            UpdateRequest::Latest => {
                get_latest_stable_release(
                    &source.name,
                    &source.owner,
                    &source.app_name,
                    &source.release_type,
                )
                .await?
            }
            UpdateRequest::SpecificTag(version) => {
                get_specific_tag(
                    &source.name,
                    &source.owner,
                    &source.app_name,
                    &source.release_type,
                    &version,
                )
                .await?
            }
            UpdateRequest::SpecificVersion(version) => {
                get_specific_version(
                    &source.name,
                    &source.owner,
                    &source.app_name,
                    &source.release_type,
                    &version,
                )
                .await?
            }
        };

        let Some(release) = release else {
            return Err(AxoupdateError::NoStableReleases {
                app_name: app_name.to_owned(),
            });
        };

        self.requested_release = Some(release);

        Ok(())
    }
}

/// An alias for Result<T, AxoupdateError>
pub type AxoupdateResult<T> = std::result::Result<T, AxoupdateError>;

/// An enum representing all of this crate's errors
#[derive(Debug, Error, Diagnostic)]
pub enum AxoupdateError {
    /// Passed through from Reqwest
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    /// Passed through from std::io::Error
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Passed through from Camino
    #[error(transparent)]
    CaminoPathBuf(#[from] camino::FromPathBufError),

    /// Passed through from homedir
    #[error(transparent)]
    Homedir(#[from] homedir::GetHomeError),

    /// Passed through from axoasset
    #[error(transparent)]
    Axoasset(#[from] AxoassetError),

    /// Passed through from axoprocess
    #[error(transparent)]
    Axoprocess(#[from] AxoprocessError),

    /// Passed through from gazenot
    #[cfg(feature = "axo_releases")]
    #[error(transparent)]
    Gazenot(#[from] GazenotError),

    /// Failure when converting a PathBuf to a Utf8PathBuf
    #[error("An internal error occurred when decoding path `{:?}' to utf8", path)]
    #[diagnostic(help("This probably isn't your fault; please open an issue!"))]
    CaminoConversionFailed {
        /// The path which Camino failed to convert
        path: PathBuf,
    },

    /// Indicates that the only updates available are located at a source
    /// this crate isn't configured to support. This is returned if the
    /// appropriate source is disabled via features.
    #[error("Release is located on backend {backend}, but it's not enabled")]
    #[diagnostic(help("This probably isn't your fault; please open an issue!"))]
    BackendDisabled {
        /// The name of the backend
        backend: String,
    },

    /// Indicates that axoupdater wasn't able to determine the config file path
    /// for this app. This path is where install receipts are located.
    #[error("Unable to determine config file path for app {app_name}!")]
    #[diagnostic(help("This probably isn't your fault; please open an issue!"))]
    ConfigFetchFailed {
        /// This app's name
        app_name: String,
    },

    /// Indicates that the install receipt for this app couldn't be read.
    #[error("Unable to read installation information for app {app_name}.")]
    #[diagnostic(help("This probably isn't your fault; please open an issue!"))]
    ReceiptLoadFailed {
        /// This app's name
        app_name: String,
    },

    /// Indicates that this app's name couldn't be determined when trying
    /// to autodetect it.
    #[error("Unable to determine the name of the app to update")]
    #[diagnostic(help("This probably isn't your fault; please open an issue!"))]
    NoAppName {},

    /// Indicates that no app name was specified before the updater process began.
    #[error("No app name was configured for this updater")]
    #[diagnostic(help("This isn't your fault; please open an issue!"))]
    NoAppNamePassed {},

    /// Indicates that the home directory couldn't be determined.
    #[error("Unable to fetch your home directory")]
    #[diagnostic(help("This may not be your fault; please open an issue!"))]
    NoHome {},

    /// Indicates that no installer is available for this OS when looking up
    /// the latest release.
    #[error("Unable to find an installer for your OS")]
    NoInstallerForPackage {},

    /// Indicates that no stable releases exist for the app being updated.
    #[error("There are no stable releases available for {app_name}")]
    NoStableReleases {
        /// This app's name
        app_name: String,
    },

    /// Indicates that no releases exist for this app at all.
    #[error("No releases were found for the app {app_name} in workspace {name}")]
    ReleaseNotFound {
        /// The workspace's name
        name: String,
        /// This app's name
        app_name: String,
    },

    /// Indicates that no releases exist for this app at all.
    #[error("The version {version} was not found for the app {app_name} in workspace {name}")]
    VersionNotFound {
        /// The workspace's name
        name: String,
        /// This app's name
        app_name: String,
        /// The version we failed to find
        version: String,
    },

    /// This error catches an edge case where the axoupdater executable was run
    /// under its default filename, "axoupdater", instead of being installed
    /// under an app-specific name.
    #[error("App name calculated as `axoupdater'")]
    #[diagnostic(help(
        "This probably isn't what you meant to update; was the updater installed correctly?"
    ))]
    UpdateSelf {},

    /// Indicates that a mandatory config field wasn't specified before the
    /// update process ran.
    #[error("The updater isn't properly configured")]
    #[diagnostic(help("Missing configuration value for {}", missing_field))]
    NotConfigured {
        /// The name of the missing field
        missing_field: String,
    },
}

const GITHUB_API: &str = "https://api.github.com";

/// A struct representing a specific release, either from GitHub or Axo Releases.
#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    /// The tag this release represents
    pub tag_name: String,
    /// The name of the release
    pub name: String,
    /// The URL at which this release lists
    pub url: String,
    /// All assets associated with this release
    pub assets: Vec<Asset>,
    /// Whether or not this release is a prerelease
    pub prerelease: bool,
}

impl Release {
    /// Returns the version, with leading `v` stripped if appropriate.
    pub fn version(&self) -> String {
        if let Some(stripped) = self.tag_name.strip_prefix('v') {
            stripped.to_owned()
        } else {
            self.tag_name.to_owned()
        }
    }

    /// Constructs a release from Axo Releases data fetched via gazenot.
    #[cfg(feature = "axo_releases")]
    pub fn from_gazenot(release: &gazenot::PublicRelease) -> Release {
        Release {
            tag_name: release.tag_name.to_owned(),
            name: release.name.to_owned(),
            url: String::new(),
            assets: release
                .assets
                .iter()
                .map(|asset| Asset {
                    url: asset.browser_download_url.to_owned(),
                    browser_download_url: asset.browser_download_url.to_owned(),
                    name: asset.name.to_owned(),
                })
                .collect(),
            prerelease: release.prerelease,
        }
    }
}

/// Represents a specific asset inside a release.
#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    /// The URL at which this asset can be found
    pub url: String,
    /// The URL at which this asset can be downloaded
    pub browser_download_url: String,
    /// This asset's name
    pub name: String,
}

/// Where service this app's releases are hosted on
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseSourceType {
    /// GitHub Releases
    GitHub,
    /// Axo Releases
    Axo,
}

/// Information about the source of this app's releases
#[derive(Clone, Debug, Deserialize)]
pub struct ReleaseSource {
    /// Which hosting service to query for new releases
    pub release_type: ReleaseSourceType,
    /// Owner, in GitHub name-with-owner format
    pub owner: String,
    /// Name, in GitHub name-with-owner format
    pub name: String,
    /// The app's name; this can be distinct from the repository name above
    pub app_name: String,
}

/// Information parsed from a cargo-dist install receipt
#[derive(Clone, Debug, Deserialize)]
pub struct InstallReceipt {
    /// The path this app has been installed to
    pub install_prefix: Utf8PathBuf,
    /// A list of binaries installed by this app
    pub binaries: Vec<String>,
    /// Information about where this release was fetched from
    pub source: ReleaseSource,
    /// Installed version
    pub version: String,
}

#[cfg(feature = "github_releases")]
async fn get_specific_github_tag(
    name: &str,
    owner: &str,
    app_name: &str,
    tag: &str,
) -> AxoupdateResult<Release> {
    let client = reqwest::Client::new();
    let resp: Release = client
        .get(format!(
            "{GITHUB_API}/repos/{owner}/{name}/releases/tags/{tag}"
        ))
        .header(ACCEPT, "application/json")
        .header(
            USER_AGENT,
            format!("axoupdate/{}", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await?
        .error_for_status()
        .map_err(|_| AxoupdateError::VersionNotFound {
            name: name.to_owned(),
            app_name: app_name.to_owned(),
            version: tag.to_owned(),
        })?
        .json()
        .await?;

    Ok(resp)
}

#[cfg(feature = "github_releases")]
async fn get_specific_github_version(
    name: &str,
    owner: &str,
    app_name: &str,
    version: &str,
) -> AxoupdateResult<Release> {
    let releases = get_github_releases(name, owner, app_name).await?;
    let release = releases.into_iter().find(|r| r.version() == version);

    if let Some(release) = release {
        Ok(release)
    } else {
        Err(AxoupdateError::VersionNotFound {
            name: name.to_owned(),
            app_name: app_name.to_owned(),
            version: version.to_owned(),
        })
    }
}

#[cfg(feature = "github_releases")]
async fn get_github_releases(
    name: &str,
    owner: &str,
    app_name: &str,
) -> AxoupdateResult<Vec<Release>> {
    let client = reqwest::Client::new();
    let resp: Vec<Release> = client
        .get(format!("{GITHUB_API}/repos/{owner}/{name}/releases"))
        .header(ACCEPT, "application/json")
        .header(
            USER_AGENT,
            format!("axoupdate/{}", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await?
        .error_for_status()
        .map_err(|_| AxoupdateError::ReleaseNotFound {
            name: name.to_owned(),
            app_name: app_name.to_owned(),
        })?
        .json()
        .await?;

    Ok(resp
        .into_iter()
        .filter(|r| {
            r.assets
                .iter()
                .any(|asset| asset.name.starts_with(&format!("{app_name}-installer")))
        })
        .collect())
}

#[cfg(feature = "axo_releases")]
async fn get_specific_axo_version(
    name: &str,
    owner: &str,
    app_name: &str,
    version: &str,
) -> AxoupdateResult<Release> {
    let releases = get_axo_releases(name, owner, app_name).await?;
    let release = releases.into_iter().find(|r| r.version() == version);

    if let Some(release) = release {
        Ok(release)
    } else {
        Err(AxoupdateError::ReleaseNotFound {
            name: name.to_owned(),
            app_name: app_name.to_owned(),
        })
    }
}

#[cfg(feature = "axo_releases")]
async fn get_specific_axo_tag(
    name: &str,
    owner: &str,
    app_name: &str,
    tag: &str,
) -> AxoupdateResult<Release> {
    let releases = get_axo_releases(name, owner, app_name).await?;
    let release = releases.into_iter().find(|r| r.tag_name == tag);

    if let Some(release) = release {
        Ok(release)
    } else {
        Err(AxoupdateError::ReleaseNotFound {
            name: name.to_owned(),
            app_name: app_name.to_owned(),
        })
    }
}

#[cfg(feature = "axo_releases")]
async fn get_axo_releases(
    name: &str,
    owner: &str,
    app_name: &str,
) -> AxoupdateResult<Vec<Release>> {
    let abyss = Gazenot::new_unauthed("github".to_string(), owner)?;
    let release_lists = abyss.list_releases_many(vec![app_name.to_owned()]).await?;
    let Some(our_release) = release_lists.iter().find(|rl| rl.package_name == app_name) else {
        return Err(AxoupdateError::ReleaseNotFound {
            name: name.to_owned(),
            app_name: app_name.to_owned(),
        });
    };

    let releases: Vec<Release> = our_release
        .releases
        .iter()
        .map(Release::from_gazenot)
        .collect();

    Ok(releases)
}

async fn get_specific_version(
    name: &str,
    owner: &str,
    app_name: &str,
    release_type: &ReleaseSourceType,
    version: &str,
) -> AxoupdateResult<Option<Release>> {
    let release = match release_type {
        #[cfg(feature = "github_releases")]
        ReleaseSourceType::GitHub => {
            get_specific_github_version(name, owner, app_name, version).await?
        }
        #[cfg(not(feature = "github_releases"))]
        ReleaseSourceType::GitHub => {
            return Err(AxoupdateError::BackendDisabled {
                backend: "github".to_owned(),
            })
        }
        #[cfg(feature = "axo_releases")]
        ReleaseSourceType::Axo => get_specific_axo_version(name, owner, app_name, version).await?,
        #[cfg(not(feature = "axo_releases"))]
        ReleaseSourceType::Axo => {
            return Err(AxoupdateError::BackendDisabled {
                backend: "axodotdev".to_owned(),
            })
        }
    };

    Ok(Some(release))
}

async fn get_specific_tag(
    name: &str,
    owner: &str,
    app_name: &str,
    release_type: &ReleaseSourceType,
    tag: &str,
) -> AxoupdateResult<Option<Release>> {
    let release = match release_type {
        #[cfg(feature = "github_releases")]
        ReleaseSourceType::GitHub => get_specific_github_tag(name, owner, app_name, tag).await?,
        #[cfg(not(feature = "github_releases"))]
        ReleaseSourceType::GitHub => {
            return Err(AxoupdateError::BackendDisabled {
                backend: "github".to_owned(),
            })
        }
        #[cfg(feature = "axo_releases")]
        ReleaseSourceType::Axo => get_specific_axo_tag(name, owner, app_name, tag).await?,
        #[cfg(not(feature = "axo_releases"))]
        ReleaseSourceType::Axo => {
            return Err(AxoupdateError::BackendDisabled {
                backend: "axodotdev".to_owned(),
            })
        }
    };

    Ok(Some(release))
}

async fn get_latest_stable_release(
    name: &str,
    owner: &str,
    app_name: &str,
    release_type: &ReleaseSourceType,
) -> AxoupdateResult<Option<Release>> {
    let releases = match release_type {
        #[cfg(feature = "github_releases")]
        ReleaseSourceType::GitHub => get_github_releases(name, owner, app_name).await?,
        #[cfg(not(feature = "github_releases"))]
        ReleaseSourceType::GitHub => {
            return Err(AxoupdateError::BackendDisabled {
                backend: "github".to_owned(),
            })
        }
        #[cfg(feature = "axo_releases")]
        ReleaseSourceType::Axo => get_axo_releases(name, owner, app_name).await?,
        #[cfg(not(feature = "axo_releases"))]
        ReleaseSourceType::Axo => {
            return Err(AxoupdateError::BackendDisabled {
                backend: "axodotdev".to_owned(),
            })
        }
    };

    Ok(releases.into_iter().find(|r| !r.prerelease))
}

fn get_app_name() -> Option<String> {
    if let Ok(name) = env::var("AXOUPDATER_APP_NAME") {
        Some(name)
    } else if let Some(path) = args().next() {
        Utf8PathBuf::from(&path)
            .file_name()
            .map(|s| s.strip_suffix(".exe").unwrap_or(s))
            .map(|s| s.strip_suffix("-update").unwrap_or(s))
            .map(|s| s.to_owned())
    } else {
        None
    }
}

fn get_config_path(app_name: &str) -> AxoupdateResult<Utf8PathBuf> {
    if env::var("AXOUPDATER_CONFIG_WORKING_DIR").is_ok() {
        Ok(Utf8PathBuf::try_from(current_dir()?)?)
    } else if let Ok(path) = env::var("AXOUPDATER_CONFIG_PATH") {
        Ok(Utf8PathBuf::from(path))
    } else {
        let home = if cfg!(windows) {
            env::var("LOCALAPPDATA").map(PathBuf::from).ok()
        } else {
            homedir::get_my_home()?.map(|path| path.join(".config"))
        };
        let Some(home) = home else {
            return Err(AxoupdateError::NoHome {});
        };

        Ok(Utf8PathBuf::try_from(home)?.join(app_name))
    }
}

fn load_receipt_from_path(install_receipt_path: &Utf8PathBuf) -> AxoupdateResult<InstallReceipt> {
    Ok(SourceFile::load_local(install_receipt_path)?.deserialize_json()?)
}

fn load_receipt_for(app_name: &str) -> AxoupdateResult<InstallReceipt> {
    let Ok(receipt_prefix) = get_config_path(app_name) else {
        return Err(AxoupdateError::ConfigFetchFailed {
            app_name: app_name.to_owned(),
        });
    };

    let install_receipt_path = receipt_prefix.join(format!("{app_name}-receipt.json"));

    load_receipt_from_path(&install_receipt_path).map_err(|_| AxoupdateError::ReceiptLoadFailed {
        app_name: app_name.to_owned(),
    })
}
