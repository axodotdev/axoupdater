//! Fetching and processing from GitHub Releases

use super::{Asset, Release};
use crate::errors::*;
use axoasset::reqwest::{
    self,
    header::{ACCEPT, USER_AGENT},
};
use axotag::{parse_tag, Version};
use serde::Deserialize;

const GITHUB_API: &str = "https://api.github.com";

/// A struct representing a specific GitHub Release
#[derive(Clone, Debug, Deserialize)]
pub struct GithubRelease {
    /// The tag this release represents
    pub tag_name: String,
    /// The name of the release
    pub name: String,
    /// The URL at which this release lists
    pub url: String,
    /// All assets associated with this release
    pub assets: Vec<GithubAsset>,
    /// Whether or not this release is a prerelease
    pub prerelease: bool,
}

/// Represents a specific asset inside a GitHub Release.
#[derive(Clone, Debug, Deserialize)]
pub struct GithubAsset {
    /// The URL at which this asset can be found
    pub url: String,
    /// The URL at which this asset can be downloaded
    pub browser_download_url: String,
    /// This asset's name
    pub name: String,
}

pub(crate) async fn get_latest_github_release(
    name: &str,
    owner: &str,
    app_name: &str,
    token: &Option<String>,
) -> AxoupdateResult<Option<Release>> {
    let client = reqwest::Client::new();
    let mut request = client
        .get(format!("{GITHUB_API}/repos/{owner}/{name}/releases/latest"))
        .header(ACCEPT, "application/json")
        .header(
            USER_AGENT,
            format!("axoupdate/{}", env!("CARGO_PKG_VERSION")),
        );
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let gh_release: GithubRelease = request
        .send()
        .await?
        .error_for_status()
        .map_err(|_| AxoupdateError::NoStableReleases {
            app_name: app_name.to_owned(),
        })?
        .json()
        .await?;

    // Ensure that this release contains an installer asset; if not, it may be
    // a mismarked "latest" release that's not installable by us.
    // Returning None here will let us fall back to iterating releases.
    if !gh_release
        .assets
        .iter()
        .any(|asset| asset.name.starts_with(&format!("{app_name}-installer")))
    {
        return Ok(None);
    }

    match Release::try_from_github(app_name, gh_release) {
        Ok(release) => Ok(Some(release)),
        Err(e) => Err(e),
    }
}

pub(crate) async fn get_specific_github_tag(
    name: &str,
    owner: &str,
    app_name: &str,
    tag: &str,
    token: &Option<String>,
) -> AxoupdateResult<Release> {
    let client = reqwest::Client::new();
    let mut request = client
        .get(format!(
            "{GITHUB_API}/repos/{owner}/{name}/releases/tags/{tag}"
        ))
        .header(ACCEPT, "application/json")
        .header(
            USER_AGENT,
            format!("axoupdate/{}", env!("CARGO_PKG_VERSION")),
        );
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let gh_release: GithubRelease = request
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

    Release::try_from_github(app_name, gh_release)
}

pub(crate) async fn get_specific_github_version(
    name: &str,
    owner: &str,
    app_name: &str,
    version: &Version,
    token: &Option<String>,
) -> AxoupdateResult<Release> {
    let releases = get_github_releases(name, owner, app_name, token).await?;
    let release = releases.into_iter().find(|r| &r.version == version);

    if let Some(release) = release {
        Ok(release)
    } else {
        Err(AxoupdateError::VersionNotFound {
            name: name.to_owned(),
            app_name: app_name.to_owned(),
            version: version.to_string(),
        })
    }
}

pub(crate) async fn get_github_releases(
    name: &str,
    owner: &str,
    app_name: &str,
    token: &Option<String>,
) -> AxoupdateResult<Vec<Release>> {
    let client = reqwest::Client::new();
    let mut url = format!("{GITHUB_API}/repos/{owner}/{name}/releases");
    let mut pages_remain = true;
    let mut data: Vec<Release> = vec![];

    while pages_remain {
        // fetch the releases
        let resp = get_releases(&client, &url, token).await?;

        // collect the response headers
        let headers = resp.headers();
        let link_header = &headers
            .get(reqwest::header::LINK)
            .as_ref()
            .map(|link_header_val| {
                link_header_val
                    .to_str()
                    .expect("header was not ascii")
                    .to_string()
            });

        // append the data
        let mut body: Vec<Release> = resp
            .json::<Vec<GithubRelease>>()
            .await?
            .into_iter()
            .filter_map(|gh| Release::try_from_github(app_name, gh).ok())
            .collect();
        data.append(&mut body);

        // check headers to see pages remain and if they do update the URL
        pages_remain = if let Some(link_header) = link_header {
            if link_header.contains("rel=\"next\"") {
                url = get_next_url(link_header).expect("detected a next but it was a lie");
                true
            } else {
                false
            }
        } else {
            false
        };
    }

    Ok(data
        .into_iter()
        .filter(|r| {
            r.assets
                .iter()
                .any(|asset| asset.name.starts_with(&format!("{app_name}-installer")))
        })
        .collect())
}

// The format of the header looks like so:
// ```
// <https://api.github.com/repositories/1300192/issues?page=2>; rel="prev", <https://api.github.com/repositories/1300192/issues?page=4>; rel="next", <https://api.github.com/repositories/1300192/issues?page=515>; rel="last", <https://api.github.com/repositories/1300192/issues?page=1>; rel="first"
// ```
fn get_next_url(link_header: &str) -> Option<String> {
    let links = link_header.split(',').collect::<Vec<_>>();
    for entry in links {
        if entry.contains("next") {
            let mut link = entry.split(';').collect::<Vec<_>>()[0]
                .to_string()
                .trim()
                .to_string();
            link.remove(0);
            link.pop();
            return Some(link);
        }
    }
    None
}

pub(crate) async fn get_releases(
    client: &reqwest::Client,
    url: &str,
    token: &Option<String>,
) -> AxoupdateResult<reqwest::Response> {
    let mut request = client
        .get(url)
        .header(ACCEPT, "application/json")
        .header(
            USER_AGENT,
            format!("axoupdate/{}", env!("CARGO_PKG_VERSION")),
        )
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    Ok(request.send().await?.error_for_status()?)
}

impl Release {
    /// Constructs a release from GitHub Releases data.
    pub(crate) fn try_from_github(
        package_name: &str,
        release: GithubRelease,
    ) -> AxoupdateResult<Release> {
        // try to parse the github release's tag using axotag
        let announce = parse_tag(
            &[axotag::Package {
                name: package_name.to_owned(),
                version: None,
            }],
            &release.tag_name,
        )?;
        let version = match announce.release {
            axotag::ReleaseType::None => unreachable!("parse_tag should never return None"),
            axotag::ReleaseType::Version(v) => v,
            axotag::ReleaseType::Package { version, .. } => version,
        };
        Ok(Release {
            tag_name: release.tag_name,
            version,
            name: release.name,
            url: String::new(),
            assets: release
                .assets
                .into_iter()
                .map(|asset| Asset {
                    url: asset.url,
                    browser_download_url: asset.browser_download_url,
                    name: asset.name,
                })
                .collect(),
            prerelease: release.prerelease,
        })
    }
}

#[cfg(test)]
mod test {
    use super::get_next_url;

    #[test]
    fn test_link_header_parse() {
        let sample = r#"
    <https://api.github.com/repositories/1300192/issues?page=2>; rel="prev", <https://api.github.com/repositories/1300192/issues?page=4>; rel="next", <https://api.github.com/repositories/1300192/issues?page=515>; rel="last", <https://api.github.com/repositories/1300192/issues?page=1>; rel="first"
    "#;

        let result = get_next_url(sample);
        assert!(result.is_some());
        assert_eq!(
            "https://api.github.com/repositories/1300192/issues?page=4",
            result.unwrap()
        );
    }

    #[test]
    fn test_link_header_parse_next_missing() {
        let sample = r#"
    <https://api.github.com/repositories/1300192/issues?page=2>; rel="prev", <https://api.github.com/repositories/1300192/issues?page=515>; rel="last", <https://api.github.com/repositories/1300192/issues?page=1>; rel="first"
    "#;

        let result = get_next_url(sample);
        assert!(result.is_none());
    }

    #[test]
    fn test_link_header_parse_empty_header() {
        let sample = "";

        let result = get_next_url(sample);
        assert!(result.is_none());
    }
}
