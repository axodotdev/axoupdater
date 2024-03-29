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
