# axoupdater

axoupdater provides an autoupdater program designed for use with [cargo-dist](https://opensource.axo.dev/cargo-dist/). It can be used either as a standalone program, or as a library within your own program. It supports releases hosted on either [GitHub Releases](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases) or [Axo Releases (in beta)](https://axo.dev).

In order to be able to check information about an installed program, it uses the install receipts produced by cargo-dist since version [0.10.0 or later](https://github.com/axodotdev/cargo-dist/releases/tag/v0.10.0). These install receipts are JSON files containing metadata about the currently-installed version of an app and the version of cargo-dist that produced it; they can be found in `~/.config/APP_NAME` (Linux, Mac) or `%LOCALAPPDATA%\APP_NAME` (Windows).

## Standalone use

When built as a standalone commandline app, axoupdater does exactly one thing: check if the user is using the latest version of the software it's built for, and perform an update if not. Rather than being hardcoded for a specific application, the updater's filename is used to determine what app to update. For example, if axoupdater is installed under the filename `axolotlsay-update`, then it will try to fetch updates for the app named `axolotlsay`. This means you only need to build axoupdater once, and can deploy it for many apps without rebuilding.

In an upcoming release, cargo-dist will support generating and installing the updater for your users as an optional feature.

## Library use

axoupdater can also be used as a library within your own applications in order to let you check for updates or perform an automatic update within your own apps. Here's a few examples of how that can be used.

To check for updates and notify the user:

```rust
if AxoUpdater::new_for("axolotlsay").load_receipt()?.is_update_needed()? {
    eprintln!("axolotlsay is outdated; please upgrade!");
}
```

To automatically perform an update if the program isn't up to date:

```rust
if AxoUpdater::new_for("axolotlsay").load_receipt()?.run()? {
    eprintln!("Update installed!");
} else {
    eprintln!("axolotlsay already up to date");
}
```

## Crate features

By default, axoupdater is built with support for both GitHub and Axo releases. If you're using it as a library in your program, and you know ahead of time which backend you're using to host your release assets, you can disable the other library in order to reduce the size of the dependency tree.

## Building

To build as a standalone binary, follow these steps:

* Run `cargo build --release --features=axocli`
* Rename `target/release/axoupdater` to `APPNAME-update`, where `APPNAME` is the name of the app you want it to upgrade.

## License

Licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [apache.org/licenses/LICENSE-2.0](https://www.apache.org/licenses/LICENSE-2.0))
* MIT license ([LICENSE-MIT](LICENSE-MIT) or [opensource.org/licenses/MIT](https://opensource.org/licenses/MIT))

at your option.
