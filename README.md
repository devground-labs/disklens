<p align="center">
  <img src="assets/icon.png" width="128" alt="DiskLens logo">
</p>

<h1 align="center">DiskLens</h1>

<p align="center">
  See what's filling your Mac's disk, and what's safe to reclaim.<br>
  A native macOS app by <a href="https://github.com/devground-labs">devground-labs</a>.
</p>

---

macOS lumps a lot of things into "System Data" and gives you no way to see what's inside or whether you can delete it. DiskLens scans the usual suspects (caches, logs, developer tools, app leftovers, installers, backups) and tells you, in plain language, what each one is and whether clearing it is safe.

**DiskLens never deletes anything.** It shows you what's there, explains it, and gives you the command or Finder location to clear it yourself.

## Features

- **Disk overview**: one bar for the whole disk, split into Caches & junk, Developer, Apps & app data, Your files, System, everything else, and free space.
- **40+ known locations**, each with a one-line explanation and a clear status:
  - **Safe to clear**: rebuilds itself, nothing lost
  - **Check first**: look inside before deleting
  - **Your files**: only you can decide
  - **Leave to macOS**: macOS frees it on its own
- **Folder-by-folder breakdown** of caches and app data, with each folder matched to the app that owns it.
- **Leftover detection**: flags data and caches from apps you've uninstalled.
- **Installer cleanup**: finds `.dmg` / `.pkg` / `.xip` files and tells you whether the app is already installed.
- **History**: a chart of used space over time, with each cleanup marked and explained.
- **Menu-bar monitor**: shows free space in the menu bar and sends a notification when it drops below a threshold you pick (5–50 GB).
- **Explorer**: browse any folder, sorted by size.
- Light and dark mode, native fonts, fast parallel scanning (a full scan takes about 10–15 seconds).

## Install

1. Download [**DiskLens.dmg**](https://github.com/devground-labs/disklens/releases/latest/download/DiskLens.dmg) (or pick a version on the [Releases page](https://github.com/devground-labs/disklens/releases)).
2. Open the dmg and drag **DiskLens** into **Applications**.
3. The first time you open it, macOS will say it can't verify the app, because DiskLens isn't notarized by Apple yet. To approve it once:
   - Open **System Settings › Privacy & Security**, scroll down and click **Open Anyway** next to DiskLens (on older macOS, right-click the app and choose **Open**), or
   - run `xattr -dr com.apple.quarantine /Applications/DiskLens.app` in Terminal.

You can verify the download against the source: the dmg is built by [this workflow](.github/workflows/release.yml) from the tagged commit.

## Requirements

- macOS 11 or later
- To build from source: a recent stable [Rust](https://rustup.rs) toolchain

## Build and run

```bash
git clone https://github.com/devground-labs/disklens.git
cd disklens
cargo build --release
./target/release/disklens
```

To make a dmg yourself, run `scripts/package.sh` (add `--universal` for Apple silicon + Intel). The result is in `dist/`.

Closing the window keeps DiskLens running in the menu bar. Use **Quit DiskLens** from the menu-bar icon to exit.

### Full Disk Access (recommended)

macOS hides some folders (Mail, Messages, other apps' containers) from apps by default, so their sizes are undercounted. To see everything, open **System Settings › Privacy & Security › Full Disk Access** and add **DiskLens** (or the terminal you launch it from), then scan again.

## Where data is stored

Scan history, free-space samples and settings live in `~/Library/Application Support/DiskLens/`. Nothing is sent anywhere.

## Feedback

Tried DiskLens? [Tell us how it went](https://github.com/devground-labs/disklens/issues/new/choose). A few words is plenty, and it helps to hear what worked, what confused you and what was missing.

## Contributing

Contributions are welcome: new cleanup locations, better explanations, bug fixes and UI improvements. See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test and submit changes. Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

DiskLens is open source under the [Apache License 2.0](LICENSE). See [NOTICE](NOTICE) for attribution.

The DiskLens name and logo are trademarks of devground-labs and are not covered by the Apache-2.0 license. If you distribute a modified version, please give it a different name and logo.

© 2026 devground-labs
