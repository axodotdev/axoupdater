use std::process::exit;

use axoupdater::{AxoUpdater, AxoupdateResult};

fn real_main() -> AxoupdateResult<()> {
    AxoUpdater::new_for_updater_executable()?
        .load_receipt()?
        .run()?;

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
