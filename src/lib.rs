use std::env::{self, args, current_dir};

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

pub struct AxoUpdater {
    pub name: Option<String>,
    pub source: Option<ReleaseSource>,
    latest_release: Option<Release>,
    current_version: Option<String>,
}

impl Default for AxoUpdater {
    fn default() -> Self {
        Self::new()
    }
}

impl AxoUpdater {
    pub fn new() -> AxoUpdater {
        AxoUpdater {
            name: None,
            source: None,
            latest_release: None,
            current_version: None,
        }
    }

    pub fn new_for(app_name: &String) -> AxoUpdater {
        AxoUpdater {
            name: Some(app_name.to_owned()),
            source: None,
            latest_release: None,
            current_version: None,
        }
    }

    pub fn new_for_updater_executable() -> AxoupdateResult<AxoUpdater> {
        let Some(app_name) = get_app_name() else {
            return Err(AxoupdateError::NoAppName {});
        };

        // Happens if the binary didn't get renamed properly
        if app_name == "axoupdate" {
            return Err(AxoupdateError::UpdateSelf {});
        };

        Ok(AxoUpdater {
            name: Some(app_name.to_owned()),
            source: None,
            latest_release: None,
            current_version: None,
        })
    }

    pub fn load_receipt(&mut self) -> AxoupdateResult<&mut AxoUpdater> {
        let Some(app_name) = &self.name else {
            return Err(AxoupdateError::NoAppNamePassed {});
        };

        let receipt = load_receipt_for(app_name)?;

        self.source = Some(receipt.source.clone());
        self.current_version = Some(receipt.version.to_owned());

        Ok(self)
    }

    pub fn set_current_version(&mut self, version: &String) -> AxoupdateResult<&mut AxoUpdater> {
        self.current_version = Some(version.to_owned());

        Ok(self)
    }

    pub fn is_update_needed(&mut self) -> AxoupdateResult<bool> {
        let Some(current_version) = self.current_version.to_owned() else {
            return Err(AxoupdateError::NotConfigured {
                missing_field: "current_version".to_owned(),
            });
        };

        let release = match &self.latest_release {
            Some(r) => r,
            None => {
                self.fetch_latest_release()?;
                self.latest_release.as_ref().unwrap()
            }
        };

        Ok(current_version != release.version())
    }

    pub fn run(&mut self) -> AxoupdateResult<bool> {
        if !self.is_update_needed()? {
            return Ok(false);
        }

        let release = match &self.latest_release {
            Some(r) => r,
            None => {
                self.fetch_latest_release()?;
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

        let installer_path = Utf8PathBuf::try_from(tempdir.path().join("installer"))?;

        #[cfg(unix)]
        {
            let installer_file = File::create(&installer_path)?;
            let mut perms = installer_file.metadata()?.permissions();
            perms.set_mode(0o744);
            installer_file.set_permissions(perms)?;
        }

        let client = reqwest::blocking::Client::new();
        let download = client
            .get(&installer_url.browser_download_url)
            .header(ACCEPT, "application/octet-stream")
            .send()?
            .text()?;

        LocalAsset::write_new_all(&download, &installer_path)?;

        Cmd::new(&installer_path, "installer").run()?;

        Ok(true)
    }

    fn fetch_latest_release(&mut self) -> AxoupdateResult<()> {
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
        )?
        else {
            return Err(AxoupdateError::NoStableReleases {
                app_name: app_name.to_owned(),
            });
        };

        self.latest_release = Some(release);

        Ok(())
    }
}

pub type AxoupdateResult<T> = std::result::Result<T, AxoupdateError>;

#[derive(Debug, Error, Diagnostic)]
pub enum AxoupdateError {
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    CaminoPathBuf(#[from] camino::FromPathBufError),

    #[error(transparent)]
    Homedir(#[from] homedir::GetHomeError),

    #[error(transparent)]
    Axoasset(#[from] AxoassetError),

    #[error(transparent)]
    Axoprocess(#[from] AxoprocessError),

    #[cfg(feature = "axo_releases")]
    #[error(transparent)]
    Gazenot(#[from] GazenotError),

    #[error("Release is located on backend {backend}, but it's not enabled")]
    #[diagnostic(help("This probably isn't your fault; please open an issue!"))]
    BackendDisabled { backend: String },

    #[error("Unable to determine config file path for app {app_name}!")]
    #[diagnostic(help("This probably isn't your fault; please open an issue!"))]
    ConfigFetchFailed { app_name: String },

    #[error("Unable to determine the name of the app to update")]
    #[diagnostic(help("This probably isn't your fault; please open an issue!"))]
    NoAppName {},

    #[error("No app name was configured for this updater")]
    #[diagnostic(help("This isn't your fault; please open an issue!"))]
    NoAppNamePassed {},

    #[error("Unable to fetch your home directory")]
    #[diagnostic(help("This may not be your fault; please open an issue!"))]
    NoHome {},

    #[error("Unable to find an installer for your OS")]
    NoInstallerForPackage {},

    #[error("There are no stable releases available for {app_name}")]
    NoStableReleases { app_name: String },

    #[error("No releases were found for the app {app_name} in workspace {name}")]
    ReleaseNotFound { name: String, app_name: String },

    #[error("App name calculated as `axoupdate'")]
    #[diagnostic(help(
        "This probably isn't what you meant to update; was the updater installed correctly?"
    ))]
    UpdateSelf {},

    #[error("The updater isn't properly configured")]
    #[diagnostic(help("Missing configuration value for {}", missing_field))]
    NotConfigured { missing_field: String },
}

const GITHUB_API: &str = "https://api.github.com";

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub name: String,
    pub url: String,
    pub assets: Vec<Asset>,
    pub prerelease: bool,
}

impl Release {
    pub fn version(&self) -> String {
        if let Some(stripped) = self.tag_name.strip_prefix('v') {
            stripped.to_owned()
        } else {
            self.tag_name.to_owned()
        }
    }

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

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub url: String,
    pub browser_download_url: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseSourceType {
    GitHub,
    Axo,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReleaseSource {
    pub release_type: ReleaseSourceType,
    pub owner: String,
    pub name: String,
    pub app_name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct InstallReceipt {
    pub install_prefix: Utf8PathBuf,
    pub binaries: Vec<String>,
    pub source: ReleaseSource,
    pub version: String,
}

#[cfg(feature = "github_releases")]
pub fn get_github_releases(
    name: &String,
    owner: &String,
    app_name: &String,
) -> AxoupdateResult<Vec<Release>> {
    let client = reqwest::blocking::Client::new();
    let resp: Vec<Release> = client
        .get(format!("{GITHUB_API}/repos/{owner}/{name}/releases"))
        .header(ACCEPT, "application/json")
        .header(
            USER_AGENT,
            format!("axoupdate/{}", env!("CARGO_PKG_VERSION")),
        )
        .send()?
        .json()?;

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
pub fn get_axo_releases(
    name: &String,
    owner: &String,
    app_name: &String,
) -> AxoupdateResult<Vec<Release>> {
    let abyss = Gazenot::new_unauthed("github".to_string(), owner)?;
    let release_lists = tokio::runtime::Builder::new_current_thread()
        .worker_threads(1)
        .max_blocking_threads(128)
        .enable_all()
        .build()
        .expect("Initializing tokio runtime failed")
        .block_on(abyss.list_releases_many(vec![app_name.to_owned()]))?;
    let Some(our_release) = release_lists.iter().find(|rl| &rl.package_name == app_name) else {
        return Err(AxoupdateError::ReleaseNotFound {
            name: name.to_owned(),
            app_name: app_name.to_owned(),
        });
    };

    Ok(our_release
        .releases
        .iter()
        .map(Release::from_gazenot)
        .collect())
}

pub fn get_latest_stable_release(
    name: &String,
    owner: &String,
    app_name: &String,
    release_type: &ReleaseSourceType,
) -> AxoupdateResult<Option<Release>> {
    let releases = match release_type {
        #[cfg(feature = "github_releases")]
        ReleaseSourceType::GitHub => get_github_releases(name, owner, app_name)?,
        #[cfg(not(feature = "github_releases"))]
        ReleaseSourceType::GitHub => {
            return Err(AxoupdateError::BackendDisabled {
                backend: "github".to_owned(),
            })
        }
        #[cfg(feature = "axo_releases")]
        ReleaseSourceType::Axo => get_axo_releases(name, owner, app_name)?,
        #[cfg(not(feature = "axo_releases"))]
        ReleaseSourceType::Axo => {
            return Err(AxoupdateError::BackendDisabled {
                backend: "axodotdev".to_owned(),
            })
        }
    };

    Ok(releases.into_iter().find(|r| !r.prerelease))
}

pub fn get_app_name() -> Option<String> {
    if cfg!(debug_assertions) {
        Some("cargo-dist".to_owned())
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

pub fn get_config_path(app_name: &String) -> AxoupdateResult<Utf8PathBuf> {
    if cfg!(debug_assertions) {
        Ok(Utf8PathBuf::try_from(current_dir()?)?)
    } else {
        let Some(home) = homedir::get_my_home()? else {
            return Err(AxoupdateError::NoHome {});
        };

        Ok(Utf8PathBuf::try_from(home)?.join(".config").join(app_name))
    }
}

fn load_receipt_from_path(install_receipt_path: &Utf8PathBuf) -> AxoupdateResult<InstallReceipt> {
    Ok(SourceFile::load_local(install_receipt_path)?.deserialize_json()?)
}

fn load_receipt_for(app_name: &String) -> AxoupdateResult<InstallReceipt> {
    let Ok(receipt_prefix) = get_config_path(app_name) else {
        return Err(AxoupdateError::ConfigFetchFailed {
            app_name: app_name.to_owned(),
        });
    };

    let install_receipt_path = receipt_prefix.join(format!("{app_name}-receipt.json"));

    load_receipt_from_path(&install_receipt_path)
}
