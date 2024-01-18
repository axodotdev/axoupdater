use std::process::exit;

use axoupdater::{AxoUpdater, AxoupdateResult};

fn real_main() -> AxoupdateResult<()> {
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

fn main() -> std::io::Result<()> {
    match real_main() {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("{e}");
            exit(1)
        }
    }
}
