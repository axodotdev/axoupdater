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

    if let Some(version) = &cli.config.tag {
        updater.configure_version_specifier(axoupdater::UpdateRequest::SpecificTag(
            version.to_owned(),
        ));
    }
    if let Some(version) = &cli.config.version {
        updater.configure_version_specifier(axoupdater::UpdateRequest::SpecificVersion(
            version.to_owned(),
        ));
    }

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
