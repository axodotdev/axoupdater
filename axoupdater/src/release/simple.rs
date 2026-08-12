//! Fetching and processing from GitHub Releases

use super::{Asset, Release};
use crate::errors::*;
use axoasset::reqwest::{
    self,
    header::{ACCEPT},
};
use axotag::{parse_tag, Version};
use serde::{Deserialize, Serialize};

/// A struct representing a specific GitHub Release
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SimpleRelease {
    /// The version this release represents
    pub version: String,
    /// The date of the release
    pub date: Option<String>,
    /// The artifacts in this release
    #[serde(alias = "artifacts")]
    pub assets: Vec<SimpleAsset>,
    /// Whether this is a prerelease
    pub prerelease: Option<bool>,
}

/// Represents a specific asset inside a GitHub Release.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SimpleAsset {
    /// Target triple like "aarch64-apple-darwin"
    platform: String,
    /// URL of the artifact like "https://static.myapp.com/0.10.4/myapp-aarch64-apple-darwin.tar.gz"
    url: String,
    /// File extension like "tar.gz" or "zip"
    archive_format: Option<String>,
    /// Hash like "a6852e4dc565c8fedcf5adcdf09fca7caf5347739bed512bd95b15dada36db51"
    sha256: Option<String>,
}

pub(crate) async fn get_latest_simple_release(
    _name: &str,
    app_name: &str,
    url: &str,
    client: &reqwest::Client,
    token: &Option<String>,
) -> AxoupdateResult<Option<Release>> {
    let releases = get_simple_releases(app_name, url, client, token).await?;
    
    // Assume the input is sorted so first == newest, but prefer stable over prerelease if any stables exist
    let allow_prereleases = !releases.iter().any(|release| !release.prerelease);
    Ok(releases.into_iter().filter(|release|  allow_prereleases || !release.prerelease).next())
}

pub(crate) async fn get_specific_simple_tag(
    name: &str,
    app_name: &str,
    url: &str,
    tag: &str,
    client: &reqwest::Client,
    token: &Option<String>,
) -> AxoupdateResult<Release> {
    let releases = get_simple_releases(app_name, url, client, token).await?;
    let release = releases.into_iter().find(|r| &r.tag_name == tag);

    if let Some(release) = release {
        Ok(release)
    } else {
        Err(AxoupdateError::VersionNotFound {
            name: name.to_owned(),
            app_name: app_name.to_owned(),
            version: tag.to_owned(),
        })
    }
}

pub(crate) async fn get_specific_simple_version(
    name: &str,
    app_name: &str,
    url: &str,
    version: &Version,
    client: &reqwest::Client,
    token: &Option<String>,
) -> AxoupdateResult<Release> {
    let releases = get_simple_releases(app_name, url, client, token).await?;
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

pub(crate) async fn get_simple_releases(
    app_name: &str,
    url: &str,
    client: &reqwest::Client,
    token: &Option<String>,
) -> AxoupdateResult<Vec<Release>> {
    // fetch the info on the releases
    let ndjson = get_releases(client, url, token).await?.text()
        .await?;

    let releases = ndjson.lines()
        .filter_map(|line| serde_json::from_str::<SimpleRelease>(line).ok())
        .filter_map(|release| Release::try_from_simple(app_name, release).ok())
        .collect();

    Ok(releases)
}

pub(crate) async fn get_releases(
    client: &reqwest::Client,
    url: &str,
    token: &Option<String>,
) -> AxoupdateResult<reqwest::Response> {
    let mut request = client
        .get(url)
        .header(ACCEPT, "text/plain,application/ndjson");
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    Ok(request.send().await?.error_for_status()?)
}

impl Release {
    /// Constructs a release from GitHub Releases data.
    pub(crate) fn try_from_simple(
        package_name: &str,
        release: SimpleRelease,
    ) -> AxoupdateResult<Release> {
        // try to parse the release's version using axotag
        // (this is overkill if it's actually a version, but lets us handle this field as a tag)
        let announce = parse_tag(
            &[axotag::Package {
                name: package_name.to_owned(),
                version: None,
            }],
            &release.version,
        )?;
        let version = match announce.release {
            axotag::ReleaseType::None => unreachable!("parse_tag should never return None"),
            axotag::ReleaseType::Version(v) => v,
            axotag::ReleaseType::Package { version, .. } => version,
        };
        let prerelease = release.prerelease.unwrap_or(!version.pre.is_empty());
        Ok(Release {
            tag_name: release.version.clone(),
            version,
            prerelease,
            name: release.version,
            url: String::new(),
            assets: release
                .assets
                .into_iter()
                .map(|asset| Asset {
                    name: asset.url.rsplit_once('/').map(|(_, rhs)| rhs).unwrap_or(&asset.url).to_owned(),
                    url: asset.url.clone(),
                    browser_download_url: asset.url,
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod test {
    use super::{
        get_simple_releases, get_latest_simple_release, get_specific_simple_tag, get_specific_simple_version,
    };
    use axoasset::reqwest::{self, StatusCode};
    use httpmock::prelude::*;

    static SIMPLE_TEST_INPUT: &str = r#"
{"version":"v0.10.4","date":"2026-02-17T22:04:34.398448+00:00","artifacts":[{"platform":"aarch64-apple-darwin","variant":"default","url":"https://static.myapp.com/0.10.4/uv-aarch64-apple-darwin.tar.gz","archive_format":"tar.gz","sha256":"a6852e4dc565c8fedcf5adcdf09fca7caf5347739bed512bd95b15dada36db51"},{"platform":"aarch64-pc-windows-msvc","variant":"default","url":"https://static.myapp.com/0.10.4/uv-aarch64-pc-windows-msvc.zip","archive_format":"zip","sha256":"77f859cfc26181bdfb94087ce42336d9e2d9e0700bc42f6668445cde517198ce"}]}
{"version":"v0.10.3","date":"2026-02-16T11:29:11.266150+00:00","artifacts":[{"platform":"aarch64-apple-darwin","variant":"default","url":"https://static.myapp.com/0.10.3/uv-aarch64-apple-darwin.tar.gz","archive_format":"tar.gz","sha256":"ed2a08079527dafae4943fee80162ed750286657901e642eba4c9de928706df8"},{"platform":"aarch64-pc-windows-msvc","variant":"default","url":"https://static.myapp.com/0.10.3/uv-aarch64-pc-windows-msvc.zip","archive_format":"zip","sha256":"48243b8acbb31d0081e00878ee3b28535ed9f28ab8b27960b88aed8e1d6dd16a"}]}
{"version":"v0.10.2","date":"2026-02-10T19:20:56.636760+00:00","artifacts":[{"platform":"aarch64-apple-darwin","variant":"default","url":"https://static.myapp.com/0.10.2/uv-aarch64-apple-darwin.tar.gz","archive_format":"tar.gz","sha256":"3828b2de196687f60e9d199aea8b504299629300831eea0935ff3fe339903d0a"},{"platform":"aarch64-pc-windows-msvc","variant":"default","url":"https://static.myapp.com/0.10.2/uv-aarch64-pc-windows-msvc.zip","archive_format":"zip","sha256":"826e4ee3a03ec245e54c449e272fdf8aab749e039cc49c950ad43cc13702221f"}]}
    "#;

    static ALL_PRE_INPUT: &str = r#"
{"version":"v0.10.4-prerelease.1","date":"2026-02-17T22:04:34.398448+00:00","artifacts":[{"platform":"aarch64-apple-darwin","variant":"default","url":"https://static.myapp.com/0.10.4/uv-aarch64-apple-darwin.tar.gz","archive_format":"tar.gz","sha256":"a6852e4dc565c8fedcf5adcdf09fca7caf5347739bed512bd95b15dada36db51"},{"platform":"aarch64-pc-windows-msvc","variant":"default","url":"https://static.myapp.com/0.10.4/uv-aarch64-pc-windows-msvc.zip","archive_format":"zip","sha256":"77f859cfc26181bdfb94087ce42336d9e2d9e0700bc42f6668445cde517198ce"}]}
{"version":"v0.10.3-prerelease.1","date":"2026-02-16T11:29:11.266150+00:00","artifacts":[{"platform":"aarch64-apple-darwin","variant":"default","url":"https://static.myapp.com/0.10.3/uv-aarch64-apple-darwin.tar.gz","archive_format":"tar.gz","sha256":"ed2a08079527dafae4943fee80162ed750286657901e642eba4c9de928706df8"},{"platform":"aarch64-pc-windows-msvc","variant":"default","url":"https://static.myapp.com/0.10.3/uv-aarch64-pc-windows-msvc.zip","archive_format":"zip","sha256":"48243b8acbb31d0081e00878ee3b28535ed9f28ab8b27960b88aed8e1d6dd16a"}]}
{"version":"v0.10.2-prerelease.1","date":"2026-02-10T19:20:56.636760+00:00","artifacts":[{"platform":"aarch64-apple-darwin","variant":"default","url":"https://static.myapp.com/0.10.2/uv-aarch64-apple-darwin.tar.gz","archive_format":"tar.gz","sha256":"3828b2de196687f60e9d199aea8b504299629300831eea0935ff3fe339903d0a"},{"platform":"aarch64-pc-windows-msvc","variant":"default","url":"https://static.myapp.com/0.10.2/uv-aarch64-pc-windows-msvc.zip","archive_format":"zip","sha256":"826e4ee3a03ec245e54c449e272fdf8aab749e039cc49c950ad43cc13702221f"}]}
    "#;

    static MIXED_PRE_INPUT: &str = r#"
{"version":"v0.10.4-prerelease.1","date":"2026-02-17T22:04:34.398448+00:00","artifacts":[{"platform":"aarch64-apple-darwin","variant":"default","url":"https://static.myapp.com/0.10.4/uv-aarch64-apple-darwin.tar.gz","archive_format":"tar.gz","sha256":"a6852e4dc565c8fedcf5adcdf09fca7caf5347739bed512bd95b15dada36db51"},{"platform":"aarch64-pc-windows-msvc","variant":"default","url":"https://static.myapp.com/0.10.4/uv-aarch64-pc-windows-msvc.zip","archive_format":"zip","sha256":"77f859cfc26181bdfb94087ce42336d9e2d9e0700bc42f6668445cde517198ce"}]}
{"version":"v0.10.3","date":"2026-02-16T11:29:11.266150+00:00","artifacts":[{"platform":"aarch64-apple-darwin","variant":"default","url":"https://static.myapp.com/0.10.3/uv-aarch64-apple-darwin.tar.gz","archive_format":"tar.gz","sha256":"ed2a08079527dafae4943fee80162ed750286657901e642eba4c9de928706df8"},{"platform":"aarch64-pc-windows-msvc","variant":"default","url":"https://static.myapp.com/0.10.3/uv-aarch64-pc-windows-msvc.zip","archive_format":"zip","sha256":"48243b8acbb31d0081e00878ee3b28535ed9f28ab8b27960b88aed8e1d6dd16a"}]}
{"version":"v0.10.2-prerelease.1","date":"2026-02-10T19:20:56.636760+00:00","artifacts":[{"platform":"aarch64-apple-darwin","variant":"default","url":"https://static.myapp.com/0.10.2/uv-aarch64-apple-darwin.tar.gz","archive_format":"tar.gz","sha256":"3828b2de196687f60e9d199aea8b504299629300831eea0935ff3fe339903d0a"},{"platform":"aarch64-pc-windows-msvc","variant":"default","url":"https://static.myapp.com/0.10.2/uv-aarch64-pc-windows-msvc.zip","archive_format":"zip","sha256":"826e4ee3a03ec245e54c449e272fdf8aab749e039cc49c950ad43cc13702221f"}]}
    "#;

    #[tokio::test]
    async fn test_get_latest_simple_release() {
        let server = MockServer::start_async().await;

        let latest_release_http_call = server
            .mock_async(|when, then| {
                when.method("GET")
                    .path("/releases.ndjson");
                then.status(StatusCode::OK.as_u16())
                    .header("content-type", "application/ndjson")
                    .body(SIMPLE_TEST_INPUT);
            })
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/releases.ndjson", server.base_url());
        let result = get_latest_simple_release("name", "owner", &url, &client, &None).await;

        let release = result.expect("expected Ok result").expect("expected Some release");
        assert_eq!(release.version.to_string(), "0.10.4");

        latest_release_http_call.assert();
    }

    #[tokio::test]
    async fn test_get_latest_simple_release_all_pre() {
        let server = MockServer::start_async().await;

        let latest_release_http_call = server
            .mock_async(|when, then| {
                when.method("GET")
                    .path("/releases.ndjson");
                then.status(StatusCode::OK.as_u16())
                    .header("content-type", "application/ndjson")
                    .body(ALL_PRE_INPUT);
            })
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/releases.ndjson", server.base_url());
        let result = get_latest_simple_release("name", "owner", &url, &client, &None).await;

        let release = result.expect("expected Ok result").expect("expected Some release");
        assert_eq!(release.version.to_string(), "0.10.4-prerelease.1");

        latest_release_http_call.assert();
    }

    #[tokio::test]
    async fn test_get_latest_simple_release_mixed_pre() {
        let server = MockServer::start_async().await;

        let latest_release_http_call = server
            .mock_async(|when, then| {
                when.method("GET")
                    .path("/releases.ndjson");
                then.status(StatusCode::OK.as_u16())
                    .header("content-type", "application/ndjson")
                    .body(MIXED_PRE_INPUT);
            })
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/releases.ndjson", server.base_url());
        let result = get_latest_simple_release("name", "owner", &url, &client, &None).await;

        let release = result.expect("expected Ok result").expect("expected Some release");
        assert_eq!(release.version.to_string(), "0.10.3");

        latest_release_http_call.assert();
    }

    #[tokio::test]
    async fn test_get_specific_simple_tag() {
        let server = MockServer::start_async().await;

        let latest_release_http_call = server
            .mock_async(|when, then| {
                when.method("GET")
                    .path("/releases.ndjson");
                then.status(StatusCode::OK.as_u16())
                    .header("content-type", "application/ndjson")
                    .body(SIMPLE_TEST_INPUT);
            })
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/releases.ndjson", server.base_url());
        let result = get_specific_simple_tag("name", "owner", &url, "v0.10.2", &client, &None).await;

        let release = result.expect("expected Ok result");
        assert_eq!(release.version.to_string(), "0.10.2");

        latest_release_http_call.assert();
    }
    
    #[tokio::test]
    async fn test_get_specific_simple_version() {
        let server = MockServer::start_async().await;

        let latest_release_http_call = server
            .mock_async(|when, then| {
                when.method("GET")
                    .path("/releases.ndjson");
                then.status(StatusCode::OK.as_u16())
                    .header("content-type", "application/ndjson")
                    .body(SIMPLE_TEST_INPUT);
            })
            .await;

        
        let client = reqwest::Client::new();
        let url = format!("{}/releases.ndjson", server.base_url());
        let result = get_specific_simple_version("name", "owner", &url, &axotag::Version::new(0, 10, 3), &client, &None).await;

        let release = result.expect("expected Ok result");
        assert_eq!(release.version.to_string(), "0.10.3");

        latest_release_http_call.assert();
    }

        
    #[tokio::test]
    async fn test_get_simple_releases() {
        let server = MockServer::start_async().await;

        let latest_release_http_call = server
            .mock_async(|when, then| {
                when.method("GET")
                    .path("/releases.ndjson");
                then.status(StatusCode::OK.as_u16())
                    .header("content-type", "application/ndjson")
                    .body(SIMPLE_TEST_INPUT);
            })
            .await;

        
        let client = reqwest::Client::new();
        let url = format!("{}/releases.ndjson", server.base_url());
        let result = get_simple_releases("name", &url, &client, &None).await;

        let releases = result.expect("expected Ok result");
        assert_eq!(releases[0].version.to_string(), "0.10.4");
        assert_eq!(releases[1].version.to_string(), "0.10.3");
        assert_eq!(releases[2].version.to_string(), "0.10.2");

        latest_release_http_call.assert();
    }
}
