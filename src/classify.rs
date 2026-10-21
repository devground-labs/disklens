//! Explains individual folders inside caches and app-data locations:
//! which app owns them, whether they're safe to clear, and whether the app is still installed.

use crate::targets::Safety;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

pub struct Info {
    pub app: String,
    pub safety: Safety,
    pub note: &'static str,
    pub cmd: Option<&'static str>,
    /// Belongs to an app that isn't installed any more.
    pub leftover: bool,
}

struct Rule {
    /// Lowercase pattern: `foo*` prefix, `*foo` suffix, `*foo*` contains, otherwise exact.
    pat: &'static str,
    app: &'static str,
    safety: Safety,
    note: &'static str,
    cmd: Option<&'static str>,
}

const fn r(pat: &'static str, app: &'static str, safety: Safety, note: &'static str, cmd: Option<&'static str>) -> Rule {
    Rule { pat, app, safety, note, cmd }
}

use Safety::*;

const UPDATER: &str = "Downloaded app updates that were already installed.";
const PKG: &str = "Package download cache; re-downloaded when needed.";
const EDITOR: &str = "Editor data. Inside, Cache, CachedData and CachedExtensionVSIXs are safe to clear; keep User (your settings).";

/// Checked in order; first match wins.
static RULES: &[Rule] = &[
    // Updaters (match before vendor rules)
    r("*.shipit", "App updater", Safe, UPDATER, None),
    r("*-updater", "App updater", Safe, UPDATER, None),
    r("*softwareupdate*", "App updater", Safe, UPDATER, None),
    r("*squirrel*", "App updater", Safe, UPDATER, None),
    // Browsers
    r("google", "Google Chrome & Google apps", Safe, "Chrome's disk cache and other Google app caches. Quit Chrome first; it rebuilds the cache.", None),
    r("com.google.chrome*", "Google Chrome", Safe, "Chrome's disk cache. Quit Chrome first.", None),
    r("firefox", "Firefox", Safe, "Firefox disk cache.", None),
    r("org.mozilla.firefox", "Firefox", Safe, "Firefox disk cache.", None),
    r("com.brave.browser*", "Brave", Safe, "Brave disk cache.", None),
    r("brave*", "Brave", Safe, "Brave disk cache.", None),
    r("company.thebrowser.browser", "Arc", Safe, "Arc disk cache.", None),
    r("com.microsoft.edgemac*", "Microsoft Edge", Safe, "Edge disk cache.", None),
    r("com.apple.safari*", "Safari", Safe, "Safari's web cache. Or use Safari > Settings > Advanced > Empty Caches.", None),
    // Package managers / dev tools
    r("homebrew", "Homebrew", Safe, "Downloaded bottles and source tarballs.", Some("brew cleanup -s")),
    r("yarn", "Yarn", Safe, PKG, Some("yarn cache clean")),
    r("pnpm", "pnpm", Safe, PKG, Some("pnpm store prune")),
    r("pip", "pip", Safe, PKG, Some("pip cache purge")),
    r("pypoetry", "Poetry", Safe, PKG, Some("poetry cache clear --all .")),
    r("uv", "uv", Safe, PKG, Some("uv cache clean")),
    r("cocoapods", "CocoaPods", Safe, "Pod specs and downloads.", Some("pod cache clean --all")),
    r("node-gyp", "node-gyp", Safe, "Node headers for building native modules; re-downloaded when needed.", None),
    r("typescript", "TypeScript", Safe, "Downloaded type definitions for editors; re-fetched when needed.", None),
    r("next-swc", "Next.js", Safe, "Next.js compiler binaries; re-downloaded on next build.", None),
    r("go-build", "Go", Safe, "Go build cache.", Some("go clean -cache")),
    r("goimports", "Go (goimports)", Safe, "Import index used by goimports; rebuilt automatically.", None),
    r("gopls", "Go (gopls)", Safe, "Go language server cache.", None),
    r("ms-playwright*", "Playwright", Review, "Browser binaries for Playwright tests. Safe to remove; reinstall with `npx playwright install`.", None),
    r("cypress", "Cypress", Review, "Cypress test runner binaries; reinstall with `npx cypress install`.", None),
    r("jetbrains", "JetBrains IDEs", Safe, "IDE indexes and caches; rebuilt when you reopen a project.", None),
    r("vscode-cpptools", "VS Code C/C++", Safe, "IntelliSense database; rebuilt.", None),
    r("com.microsoft.vscode*", "VS Code", Safe, "VS Code cache.", None),
    r("*copilot*", "GitHub Copilot", Safe, "Copilot runtime cache; re-downloaded.", None),
    r("electron*", "Electron", Safe, "Electron build/download cache.", None),
    r("deno", "Deno", Safe, PKG, None),
    r("bazel*", "Bazel", Safe, "Bazel build outputs.", Some("bazel clean --expunge")),
    r("pip-tools", "pip-tools", Safe, PKG, None),
    r("sourcekit*", "Xcode SourceKit", Safe, "Code completion index; rebuilt.", None),
    r("com.apple.dt.xcode", "Xcode", Safe, "Xcode cache.", None),
    r("rustup", "Rust", Review, "Rust toolchain downloads.", None),
    r("stable-*", "Rust toolchain", Review, "An installed Rust toolchain.", Some("rustup toolchain uninstall <name>")),
    r("nightly-*", "Rust toolchain", Review, "An installed nightly Rust toolchain; often not needed.", Some("rustup toolchain uninstall <name>")),
    r("<version>", "Toolchain version", Review, "An installed language version (nvm/pyenv). Remove if you no longer use it.", Some("nvm uninstall <version>   # or pyenv uninstall")),
    // Editors / chat apps (Application Support)
    r("code", "VS Code", Review, EDITOR, None),
    r("cursor", "Cursor", Review, EDITOR, None),
    r("antigravity*", "Antigravity", Review, EDITOR, None),
    r("com.google.antigravity", "Antigravity", Safe, "Antigravity cache.", None),
    r("windsurf", "Windsurf", Review, EDITOR, None),
    r("zed", "Zed", Review, "Zed editor data.", None),
    r("slack", "Slack", Safe, "Mostly cached media and service-worker caches. Quit Slack; the Cache and Service Worker folders inside can go.", None),
    r("com.tinyspeck.slackmacgap*", "Slack", Safe, "Slack cache.", None),
    r("discord", "Discord", Safe, "Cached media; Discord re-downloads it.", None),
    r("microsoft teams*", "Microsoft Teams", Safe, "Teams cache.", None),
    r("zoom.us", "Zoom", Review, "Zoom settings and cached data. Recordings are in ~/Documents/Zoom.", None),
    r("us.zoom.*", "Zoom", Safe, "Zoom cache.", None),
    r("com.spotify.client*", "Spotify", Safe, "Offline/streaming cache. Change the limit in Spotify > Settings > Storage.", None),
    r("spotify", "Spotify", Review, "Spotify data incl. downloaded songs (Settings > Storage).", None),
    r("claude", "Claude", Review, "Claude desktop data incl. session workspaces and VM bundles.", None),
    r("com.anthropic.*", "Claude", Safe, "Claude cache.", None),
    r("postman", "Postman", Review, "Local collections and history. Sync to your Postman account before clearing.", None),
    r("steam", "Steam", Careful, "Installed games live in steamapps. Uninstall games from Steam to free space.", None),
    r("epic", "Epic Games", Review, "Epic launcher data.", None),
    r("com.epicgames.*", "Epic Games", Safe, "Epic launcher cache.", None),
    r("obs-studio", "OBS", Review, "OBS settings and logs.", None),
    r("pgadmin*", "pgAdmin", Review, "pgAdmin settings and sessions.", None),
    r("gitkraken*", "GitKraken", Review, "GitKraken data.", None),
    r("docker*", "Docker", Review, "Docker settings.", None),
    r("com.docker.docker", "Docker", Review, "Docker's VM disk with all images and volumes.", Some("docker system prune -a --volumes")),
    r("caches", "Misc app caches", Safe, "A cache folder; apps rebuild it.", None),
    r("crashreporter", "Crash reports", Safe, "Crash reports.", None),
    r("diagnosticreports", "Crash reports", Safe, "Crash reports.", None),
    r("mobilesync", "iPhone/iPad backups", Careful, "Local device backups. Delete only if you have another backup.", None),
    r("com.apple.wallpaper", "Wallpapers", Review, "Downloaded aerial and dynamic wallpapers. Remove in System Settings > Wallpaper.", None),
    r("com.apple.mail", "Apple Mail", Careful, "Mail data. The Mail Downloads folder inside only holds copies of opened attachments.", None),
    r("com.apple.garageband10", "GarageBand", Review, "GarageBand sound library and projects. Remove sound packs from GarageBand > Sound Library.", None),
    r("*garageband*", "GarageBand", Review, "GarageBand sound library.", None),
    // Apple's own stuff
    r("geoservices", "Maps", Safe, "Map tile cache.", None),
    r("sharedimagecache", "macOS", Safety::System, "System image cache.", None),
    r("com.apple.*", "macOS", Safety::System, "Used by a macOS service; macOS manages and trims it.", None),
    r("group.com.apple.*", "macOS", Safety::System, "Shared data for Apple apps.", None),
];

fn matches(pat: &str, name: &str) -> bool {
    if pat == "<version>" {
        // nvm/pyenv version folders: "v20.11.1", "3.12.2"
        let v = name.strip_prefix('v').unwrap_or(name);
        return v.contains('.') && v.chars().all(|c| c.is_ascii_digit() || c == '.');
    }
    match (pat.strip_prefix('*'), pat.strip_suffix('*')) {
        (Some(_), Some(_)) => name.contains(&pat[1..pat.len() - 1]),
        (Some(suf), None) => name.ends_with(suf),
        (None, Some(pre)) => name.starts_with(pre),
        (None, None) => name == pat,
    }
}

/// Start reading installed apps' bundle IDs (call once at startup).
pub fn warm_up() {
    std::thread::spawn(|| {
        let home = dirs::home_dir().unwrap_or_default();
        let mut apps = vec![];
        for root in ["/Applications".into(), "/Applications/Utilities".into(), "/System/Applications".into(), "/System/Applications/Utilities".into(), home.join("Applications")] {
            collect_apps(&root, 1, &mut apps);
        }
        let ids: HashSet<String> = apps
            .par_iter()
            .filter_map(|app| {
                let out = Command::new("plutil").args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"]).arg(app.join("Contents/Info.plist")).output().ok()?;
                let id = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
                (!id.is_empty()).then_some(id)
            })
            .collect();
        // Vendor ("com.wacom") covers drivers and helpers that live outside /Applications.
        let vendors = ids.iter().filter_map(|id| id.splitn(3, '.').take(2).collect::<Vec<_>>().join(".").into()).filter(|v: &String| v.contains('.')).collect();
        let all_names: HashSet<String> = apps.iter().filter_map(|a| a.file_stem().map(|s| s.to_string_lossy().to_lowercase().replace([' ', '-', '.'], ""))).collect();
        let names = all_names.iter().filter(|n| n.len() >= 5).cloned().collect();
        let _ = installed_cell().set(Installed { ids, vendors, names, all_names });
    });
}

struct Installed {
    ids: HashSet<String>,
    vendors: HashSet<String>,
    /// App names, lowercased without spaces ("camostudio").
    names: HashSet<String>,
    /// Every app name, including short ones like "obs" (used for installer matching).
    all_names: HashSet<String>,
}

fn installed_cell() -> &'static OnceLock<Installed> {
    static SET: OnceLock<Installed> = OnceLock::new();
    &SET
}

fn collect_apps(dir: &Path, depth: u32, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "app") {
            out.push(p);
        } else if depth > 0 && p.is_dir() {
            collect_apps(&p, depth - 1, out); // e.g. /Applications/Adobe Foo/Foo.app
        }
    }
}

/// For reverse-DNS folder names (com.vendor.app...), the bundle id it probably belongs to.
fn bundle_like(name: &str) -> Option<String> {
    let n = name.strip_prefix("group.").unwrap_or(name);
    // Team-ID prefixed group containers, e.g. "UBF8T346G9.com.microsoft.office".
    let n = match n.split_once('.') {
        Some((team, rest)) if team.len() == 10 && team.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) => rest,
        _ => n,
    };
    let tld = n.split('.').next()?;
    let parts = n.split('.').count();
    (parts >= 3 && matches!(tld, "com" | "org" | "net" | "io" | "co" | "ai" | "app" | "dev" | "us" | "me" | "de" | "jp" | "cc" | "tv")).then(|| n.to_lowercase())
}

/// True when we know the installed apps and none of them plausibly owns `id`.
/// Errs on the side of "installed": a false leftover could lead someone to delete live app data.
fn is_leftover(id: &str) -> bool {
    let Some(inst) = installed_cell().get() else { return false };
    if id.starts_with("com.apple.") || id.starts_with("org.swift.") {
        return false;
    }
    let vendor = id.splitn(3, '.').take(2).collect::<Vec<_>>().join(".");
    let flat = id.replace(['-', '.'], "");
    !(inst.vendors.contains(&vendor) || inst.ids.iter().any(|b| id.starts_with(b.as_str()) || b.starts_with(id)) || inst.names.iter().any(|n| flat.contains(n.as_str())))
}

/// Words in installer file names that never name the app itself.
const INSTALLER_NOISE: &[&str] = &["macos", "mac", "osx", "darwin", "apple", "arm64", "aarch64", "x86", "x64", "intel", "universal", "community", "installer", "install", "setup", "latest", "release", "stable", "dmg", "pkg"];

/// Some(true) if the app an installer file belongs to is installed, Some(false) if not,
/// None while the installed-app list is still loading or the name has no usable words.
fn installer_app_installed(file: &str) -> Option<bool> {
    let inst = installed_cell().get()?;
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s).to_lowercase();
    // "OBS-Studio-30.0.2-macOS-Apple" -> ["obs", "studio"]; "EpicInstaller" keeps "epicinstaller" and also tries "epic".
    let words: Vec<String> = stem
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty() && !w.chars().any(|c| c.is_ascii_digit()) && !INSTALLER_NOISE.contains(w))
        .map(|w| w.strip_suffix("installer").filter(|s| !s.is_empty()).unwrap_or(w).to_string())
        .collect();
    if words.is_empty() {
        return None;
    }
    Some((1..=words.len()).any(|n| {
        let joined = words[..n].concat();
        let joined = match joined.as_str() {
            "vscode" | "code" => "visualstudiocode".to_string(),
            "chrome" => "googlechrome".to_string(),
            "teams" => "microsoftteams".to_string(),
            _ => joined,
        };
        inst.all_names.contains(&joined) || (joined.len() >= 5 && inst.names.iter().any(|a| a.starts_with(&joined)))
    }))
}

fn installer_info(name: &str) -> Info {
    let base = Info { app: String::new(), safety: Safe, note: "", cmd: None, leftover: false };
    if name.starts_with("Install macOS") {
        return Info { app: "macOS installer".into(), note: "A full macOS installer app. Re-downloadable from the App Store whenever you need it.", ..base };
    }
    match installer_app_installed(name) {
        Some(true) => Info { app: "App already installed".into(), note: "The app is in /Applications, so this installer is no longer needed.", ..base },
        Some(false) => Info { app: "App not installed".into(), safety: Review, note: "No matching app is installed. Delete it unless you still plan to install it.", ..base },
        None => Info { app: String::new(), note: "An installer. It can be downloaded again from the vendor.", ..base },
    }
}

/// Explain a folder (by name) found inside one of the breakdown locations.
pub fn classify(parent_id: &str, name: &str) -> Info {
    if parent_id == "installers" {
        return installer_info(name);
    }
    let lower = name.to_lowercase();
    let in_cache = matches!(parent_id, "user-caches" | "sys-caches" | "user-logs");
    let mut info = match RULES.iter().find(|r| matches(r.pat, &lower)) {
        Some(r) => Info { app: r.app.to_string(), safety: r.safety, note: r.note, cmd: r.cmd, leftover: false },
        None if in_cache => Info { app: String::new(), safety: Safe, note: "An app's cache; the app rebuilds it. Quit the app before clearing.", cmd: None, leftover: false },
        None if parent_id == "toolchains" => Info { app: String::new(), safety: Review, note: "An installed toolchain version.", cmd: None, leftover: false },
        None => Info { app: String::new(), safety: Review, note: "App data and settings. Removing it resets the app.", cmd: None, leftover: false },
    };
    if let Some(id) = bundle_like(name) {
        if is_leftover(&id) {
            info.leftover = true;
            if info.safety == Review {
                info.note = "No installed app matches this ID, so it's probably left over from an app you removed. Check, then delete.";
            }
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patterns() {
        assert!(matches("*.shipit", "com.tinyspeck.slackmacgap.shipit"));
        assert!(matches("ms-playwright*", "ms-playwright-go"));
        assert!(matches("<version>", "v20.11.1"));
        assert!(matches("<version>", "3.12.2"));
        assert!(!matches("<version>", "vscode"));
        assert_eq!(bundle_like("group.com.frontrow.app").as_deref(), Some("com.frontrow.app"));
        assert_eq!(bundle_like("UBF8T346G9.com.microsoft.office").as_deref(), Some("com.microsoft.office"));
        assert_eq!(bundle_like("google"), None);
    }

    #[test]
    #[ignore]
    fn list_installers() {
        warm_up();
        while installed_cell().get().is_none() {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        for f in ["OBS-Studio-30.0.2-macOS-Apple.dmg", "Slack-4.26.1-macOS.dmg", "EpicInstaller-13.3.0.dmg", "steam.dmg", "GitHub-Copilot-darwin-arm64.dmg", "mysql-workbench-community-8.0.36-macos-x86_64.dmg", "googlechrome.dmg", "VSCode-darwin-universal.zip"] {
            println!("{f}: {:?}", installer_app_installed(f));
        }
    }

    /// Lists folders flagged as leftovers on this Mac: `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn list_leftovers() {
        warm_up();
        while installed_cell().get().is_none() {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        for (id, dir) in [("user-caches", "~/Library/Caches"), ("app-support", "~/Library/Application Support"), ("containers", "~/Library/Containers"), ("group-containers", "~/Library/Group Containers")] {
            for e in std::fs::read_dir(crate::targets::expand(dir)).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if classify(id, &name).leftover {
                    println!("{id}: {name}");
                }
            }
        }
    }
}
