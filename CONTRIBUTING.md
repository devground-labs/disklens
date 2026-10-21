# Contributing to DiskLens

Thanks for helping make DiskLens better. Useful contributions include new cleanup locations, clearer explanations, bug fixes and UI improvements. This guide covers how to build the app, check your changes, and send a pull request.

By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## The one rule: DiskLens is read-only

**DiskLens must never delete, move or modify user files.** It scans, explains and points the way: it shows a location in Finder, or gives a command that people can copy and run themselves. Pull requests that add a delete, "clean up" or "empty" action, or that write anywhere outside `~/Library/Application Support/DiskLens/`, won't be merged.

When you're unsure how safe something is, pick the more cautious label. A wrong "Safe to clear" can cost someone their data. A wrong "Check first" only costs them a look.

## Building and running

You need macOS (the app is built and tested on Apple silicon) and a recent stable Rust toolchain from [rustup.rs](https://rustup.rs).

```bash
git clone https://github.com/devground-labs/disklens.git
cd disklens
cargo build --release
./target/release/disklens
```

Run the tests:

```bash
cargo test
```

Closing the window keeps DiskLens running in the menu bar. Use **Quit DiskLens** from the menu-bar icon to exit. To see folders macOS hides by default, give the binary (or your terminal) Full Disk Access in **System Settings › Privacy & Security › Full Disk Access**.

## Checking UI changes with screenshot mode

DiskLens has a built-in screenshot mode. It renders the window, saves a PNG and exits, so you can check a UI change without clicking through the app:

```bash
./target/release/disklens --screenshot out.png
```

Options you can add:

| Flag | What it does |
| --- | --- |
| `--select <id>` | Selects a location by its catalog id (for example `user-caches`) so the inspector panel shows it. |
| `--group <0-4>` | Opens a category: 0 Caches & junk, 1 Developer, 2 Apps & app data, 3 Your files, 4 System. |
| `--light` / `--dark` | Forces light or dark mode. Check both. |
| `--size WxH` | Sets the window size, for example `--size 1280x800`. |
| `--history` | Opens the History page. |
| `--explorer` | Opens the Explorer page. |
| `--scan` | Starts a fresh scan on launch. |

For example:

```bash
./target/release/disklens --screenshot caches-light.png --group 0 --select user-caches --light --size 1440x900
```

Screenshots show real folder names and sizes from your own disk. Look them over before you attach them to an issue or pull request, and crop or blur anything you'd rather keep private.

## Adding a location to the catalog

Locations (the rows in each category) live in the `TARGETS` list in [`src/targets.rs`](src/targets.rs). Each entry is a `Target`, usually built with the `t(...)` helper:

```rust
t(Developer, "my-tool-cache", "My Tool cache", &["~/Library/Caches/my-tool"], Safe,
  "What it is, what happens when it's cleared, and anything to do first (for example, quit the app).",
  Some("my-tool cache clean")),
```

The fields:

- **group**: the sidebar category, one of `Caches`, `Developer`, `Apps`, `Personal`, `System`.
- **id**: a short, unique, kebab-case id. It's used by `--select` and by `blurb()`, and it's stored in scan history, so don't rename existing ids.
- **name**: the row title people see.
- **paths**: one or more folders. Paths may start with `~/`.
- **safety**: how safe the location is to clear (see below).
- **desc**: the longer explanation shown in the inspector panel.
- **cmd**: an optional command or Finder step that clears it, shown with a Copy button. DiskLens only displays it and never runs it. Use `None` when there's no safe one-liner.

Wrappers change how an entry is scanned:

- `bd(...)` sizes each immediate subfolder, so the inspector can explain them one by one (see the classification rules below).
- `nested(...)` marks a location that's already counted inside another row, so it's left out of the totals.
- `special(...)` is for locations found by searching rather than by a fixed path (`Kind::NodeModules`, `Kind::BuildDirs`, `Kind::Installers`, `Kind::Snapshots`).

### Safety levels

| Level | Label | Use it when |
| --- | --- | --- |
| `Safe` | Safe to clear | It rebuilds itself or isn't needed any more. Clearing it loses nothing. |
| `Review` | Check first | It's often clearable, but may hold something people want. |
| `Careful` | Your files | It's the person's own data. Nothing comes back on its own. |
| `System` | Leave to macOS | macOS manages it and frees it by itself. Don't delete it by hand. |

### The one-line blurb

Every location also needs a plain-language one-liner in `blurb()` at the bottom of `src/targets.rs`. This is the line shown under the row title, written for people who don't know macOS internals:

```rust
"my-tool-cache" => "Files My Tool downloaded. It fetches them again when needed.",
```

Keep it to one short sentence, with no jargon or paths.

## Adding a classification rule

Locations wrapped in `bd(...)` (such as User caches and Application Support) list their subfolders, and [`src/classify.rs`](src/classify.rs) explains each one: which app owns it, how safe it is, and what clearing it does. Rules live in the `RULES` list and are checked in order. The first match wins, so put specific patterns before general ones.

```rust
r("com.example.myapp*", "My App", Safe, "My App's download cache. Quit My App first.", None),
```

The fields are:

- **pattern**: matched against the lowercased folder name. `foo*` matches a prefix, `*foo` a suffix, `*foo*` anywhere in the name, and anything else must match exactly.
- **app**: the app or tool the folder belongs to.
- **safety**: one of the four safety levels.
- **note**: a short explanation shown on the folder's card.
- **cmd**: an optional command that clears it, for example `Some("yarn cache clean")`.

Folders that don't match any rule still get a sensible default, and DiskLens flags reverse-DNS folders (`com.vendor.app`) that no installed app owns as leftovers. If you add a pattern, consider adding a case to the `patterns` test in `src/classify.rs`.

## Pull requests

- **Keep each PR small and focused.** One location, one fix or one UI change per PR is easiest to review.
- **Build with no warnings.** `cargo build` should finish without any.
- **Tests pass.** Run `cargo test`.
- **Format your code.** The codebase isn't fully `cargo fmt`-formatted yet, so running it reformats whole files. Until it is, match the surrounding style and keep formatting changes out of your PR. Once the codebase is formatted, run `cargo fmt` before you submit.
- **Respect the read-only rule.** No code that deletes, moves or changes user files.
- **Add screenshots for UI changes.** Show light and dark mode, using screenshot mode above, and check them for private information first.
- **Explain the why.** For a new location or rule, say where it comes from and how you know clearing it is safe (for example, the tool's own documentation).

CI runs `cargo build` and `cargo test` on macOS for every pull request.

Not sure where to start? Open an issue first, or pick one from the issue tracker. Requests for new locations and apps are a good first contribution.

## License

DiskLens is licensed under the [Apache License 2.0](LICENSE). By submitting a contribution, you agree that it is licensed under the Apache License 2.0 as well (inbound = outbound, as described in section 5 of the License), and you confirm that you have the right to submit it.

The DiskLens name and logo are trademarks of devground-labs and are not licensed under Apache-2.0. See [NOTICE](NOTICE).
