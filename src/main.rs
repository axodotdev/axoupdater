use axocli::{CliApp, CliAppBuilder};
use axoupdater::AxoUpdater;

struct CliArgs {}

fn real_main(_cli: &CliApp<CliArgs>) -> Result<(), miette::Report> {
    if AxoUpdater::new_for_updater_executable()?
        .load_receipt()?
        .run()?
    {
        eprintln!("New release installed!")
    } else {
        eprintln!("Already up to date; not upgrading");
    }

    Ok(())
}

fn main() {
    CliAppBuilder::new("axoupdater").start(CliArgs {}, real_main);
}
