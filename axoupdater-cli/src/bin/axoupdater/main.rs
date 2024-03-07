use axocli::{CliApp, CliAppBuilder};
use axoupdater::AxoUpdater;

struct CliArgs {}

fn real_main(_cli: &CliApp<CliArgs>) -> Result<(), miette::Report> {
    eprintln!("Checking for updates...");

    if let Some(result) = AxoUpdater::new_for_updater_executable()?
        .load_receipt()?
        .run_sync()?
    {
        eprintln!("New release {} installed!", result.new_version)
    } else {
        eprintln!("Already up to date; not upgrading");
    }

    Ok(())
}

fn main() {
    CliAppBuilder::new("axoupdater").start(CliArgs {}, real_main);
}
