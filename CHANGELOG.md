# Version 0.4.1 (2024-04-10)

This is a minor patch release to preserve more http error info in cases where GitHub is flaking out ([#80](https://github.com/axodotdev/axoupdater/pull/80)).

# Version 0.4.0 (2024-04-08)

This release contains a few new features and fixes:

- Pagination has been implemented for the GitHub API, making it possible to query for specific releases older than the 30 most recent versions. ([#70](https://github.com/axodotdev/axoupdater/pull/70)
- Improved version parsing and handling has been adding, ensuring that axoupdater will no longer try to pick an older stable version if the user is already running on a newer release. ([#72](https://github.com/axodotdev/axoupdater/pull/72))
- Added a test helper to simplify end-to-end self-updater tests for users of the axoupdater library. ([#76](https://github.com/axodotdev/axoupdater/pull/76))

# Version 0.3.6 (2024-04-05)

This is a minor bugfix release. It updates the ordering of axo releases queries to reflect changes to the deployed API.

# Version 0.3.5 (2024-04-05)

This is a minor bugfix release. It makes us try to temporarily rename the current executable on windows, in case we're about to overwrite it.

# Version 0.3.4 (2024-04-04)

This is a minor bugfix release. It fixes an issue which would cause Windows updates to fail if the parent process is PowerShell Core.

# Version 0.3.3 (2024-03-21)

This is a minor bugfix release. It relaxes the reqwest dependency, which had been bumped to 0.12.0 in the previous release. It will now accept either 0.11.0 or any later version.

# Version 0.3.2 (2024-03-21)

This is a minor bugfix release:

* more robust behaviour when paired with installers built with cargo-dist 0.12.0 (not yet released)
* fix for an issue on windows where the installer would never think the receipt matched the binary

# Version 0.3.1 (2024-03-18)

This is a minor bugfix release which fixes loading install receipts which contain UTF-8 byte order marks.

# Version 0.3.0 (2024-03-08)

This release contains several bugfixes and improvements:

- `axoupdater` now compares the path to which the running program was installed to the path it locates in the install receipt, and declines to upgrade if they're not equivalent. This fixes an issue where a user who had installed a copy with an installer which produced an install receipt and a second copy from a package manager would be prompted to upgrade even on the package manager-provided version.
- The `run()` and `run_sync()` methods now provide information on the upgrade that they performed. If the upgrade was performed, it returns the old and new versions and the tag that the new version was built from.
- It's now possible to silence stdout and stderr from the underlying installer when using `axoupdater` as a library.

# Version 0.2.0 (2024-03-06)

This release makes a breaking change to the library API. `run()` and `is_update_needed()` are now both async methods; new `run_sync()` and `is_update_needed_sync()` methods have been added which replicate the old behaviour. This should make it easier to incorporate the axoupdater library into asynchronous applications, especially applications which already use tokio.

To use the blocking methods, enable the `blocking` feature when importing this crate as a library.

# Version 0.1.0 (2024-03-01)

This is the initial release of axoupdater, including both the standalone binary and the library for embedding in other binaries.
