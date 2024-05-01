use axocli::{CliApp, CliAppBuilder};
use axoupdater::AxoUpdater;
use clap::Parser;
use miette::miette;

#[derive(Parser)]
struct CliArgs {
    /// Installs the specified tag instead of the latest version
    #[clap(long)]
    tag: Option<String>,

    /// Installs the specified version instead of the latest version
    #[clap(long)]
    version: Option<String>,

    /// Allows prereleases when just updating to "latest"
    #[clap(long)]
    prerelease: bool,
}

fn real_main(cli: &CliApp<CliArgs>) -> Result<(), miette::Report> {
    if cli.config.tag.is_some() && cli.config.version.is_some() {
        return Err(miette!(
            "Both `tag` and `version` are specified; these options are mutually exclusive!"
        ));
    }

    eprintln!("Checking for updates...");

    let mut updater = AxoUpdater::new_for_updater_executable()?;
    updater.load_receipt()?;

    if let Ok(token) = std::env::var("AXOUPDATER_GITHUB_TOKEN") {
        updater.set_github_token(&token);
    }

    let specifier = if let Some(tag) = &cli.config.tag {
        axoupdater::UpdateRequest::SpecificTag(tag.clone())
    } else if let Some(version) = &cli.config.version {
        axoupdater::UpdateRequest::SpecificVersion(version.clone())
    } else if cli.config.prerelease {
        axoupdater::UpdateRequest::LatestMaybePrerelease
    } else {
        axoupdater::UpdateRequest::Latest
    };
    updater.configure_version_specifier(specifier);

    if let Some(result) = updater.run_sync()? {
        eprintln!("New release {} installed!", result.new_version)
    } else {
        eprintln!("Already up to date; not upgrading");
    }

    Ok(())
}

fn main() {
    CliAppBuilder::new("axoupdater").start(CliArgs::parse(), real_main);
}
