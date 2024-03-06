# Version 0.2.0 (2024-03-06)

This release makes a breaking change to the library API. `run()` and `is_update_needed()` are now both async methods; new `run_sync()` and `is_update_needed_sync()` methods have been added which replicate the old behaviour. This should make it easier to incorporate the axoupdater library into asynchronous applications, especially applications which already use tokio.

To use the blocking methods, enable the `blocking` feature when importing this crate as a library.

# Version 0.1.0 (2024-03-01)

This is the initial release of axoupdater, including both the standalone binary and the library for embedding in other binaries.
