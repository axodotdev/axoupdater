#![deny(missing_docs)]

//! axoupdater crate

use std::{
    env::{self, args, current_dir},
    path::PathBuf,
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
}

/// Struct representing an updater process
pub struct AxoUpdater {
    /// The name of the program to update, if specified
    pub name: Option<String>,
    /// Information about where updates should be fetched from
    pub source: Option<ReleaseSource>,
    /// Information about the latest release; used to determine if an update is needed
    latest_release: Option<Release>,
    /// The current version number
    current_version: Option<String>,
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
            latest_release: None,
            current_version: None,
        }
    }

    /// Creates a new AxoUpdater struct with an explicitly-specified name.
    pub fn new_for(app_name: &str) -> AxoUpdater {
        AxoUpdater {
            name: Some(app_name.to_owned()),
            source: None,
            latest_release: None,
            current_version: None,
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
            latest_release: None,
            current_version: None,
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

        Ok(self)
    }

    /// Explicitly specifies the current version.
    pub fn set_current_version(&mut self, version: &str) -> AxoupdateResult<&mut AxoUpdater> {
        self.current_version = Some(version.to_owned());

        Ok(self)
    }

    /// Determines if an update is needed by querying the newest version from
    /// the location specified in `source`.
    /// This includes a blocking network call, so it may be slow.
    /// This can only be performed if the `current_version` field has been
    /// set, either by loading the install receipt or by specifying it using
    /// `set_current_version`.
    pub async fn is_update_needed(&mut self) -> AxoupdateResult<bool> {
        let Some(current_version) = self.current_version.to_owned() else {
            return Err(AxoupdateError::NotConfigured {
                missing_field: "current_version".to_owned(),
            });
        };

        let release = match &self.latest_release {
            Some(r) => r,
            None => {
                self.fetch_latest_release().await?;
                self.latest_release.as_ref().unwrap()
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

    /// Attempts to perform an update. The return value specifies whether an
    /// update was actually performed or not; false indicates "no update was
    /// needed", while an error indicates that an update couldn't be performed
    /// due to an error.
    pub async fn run(&mut self) -> AxoupdateResult<Option<UpdateResult>> {
        if !self.is_update_needed().await? {
            return Ok(None);
        }

        let release = match &self.latest_release {
            Some(r) => r,
            None => {
                self.fetch_latest_release().await?;
                self.latest_release.as_ref().unwrap()
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

        let path = if cfg!(windows) {
            "powershell"
        } else {
            installer_path.as_str()
        };
        let mut command = Cmd::new(path, "installer");
        if cfg!(windows) {
            command.arg(&installer_path);
        }
        command.run()?;

        let result = UpdateResult {
            old_version: self
                .current_version
                .to_owned()
                .unwrap_or("unable to determine".to_owned()),
            new_version: release.version(),
            new_version_tag: release.tag_name.to_owned(),
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

    async fn fetch_latest_release(&mut self) -> AxoupdateResult<()> {
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

        let Some(release) = get_latest_stable_release(
            &source.name,
            &source.owner,
            &source.app_name,
            &source.release_type,
        )
        .await?
        else {
            return Err(AxoupdateError::NoStableReleases {
                app_name: app_name.to_owned(),
            });
        };

        self.latest_release = Some(release);

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

    let mut releases: Vec<Release> = our_release
        .releases
        .iter()
        .map(Release::from_gazenot)
        .collect();
    // GitHub releases are sorted newest to oldest; Axo sorts oldest to newest
    releases.reverse();

    Ok(releases)
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
