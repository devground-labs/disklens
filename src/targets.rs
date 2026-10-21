//! Catalog of known macOS space hogs and how safe each is to reclaim.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Safety {
    /// Regenerates automatically, no data loss.
    Safe,
    /// Check before removing.
    Review,
    /// Your own data.
    Careful,
    /// Managed by macOS.
    System,
}

impl Safety {
    pub const ALL: [Safety; 4] = [Safety::Safe, Safety::Review, Safety::Careful, Safety::System];
    pub fn label(self) -> &'static str {
        match self {
            Safety::Safe => "Safe to clear",
            Safety::Review => "Check first",
            Safety::Careful => "Your files",
            Safety::System => "Leave to macOS",
        }
    }
    /// What the label means, in plain words.
    pub fn explain(self) -> &'static str {
        match self {
            Safety::Safe => "Rebuilds itself or isn't needed any more. Clearing it loses nothing.",
            Safety::Review => "Often clearable, but may hold something you want. Look inside before deleting.",
            Safety::Careful => "Your own files. Nothing here comes back on its own, so only you can decide.",
            Safety::System => "macOS manages this and frees it by itself, often after a restart. Don't delete by hand.",
        }
    }
}

/// Sidebar categories.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Caches,
    Developer,
    Apps,
    Personal,
    System,
}

impl Group {
    pub const ALL: [Group; 5] = [Group::Caches, Group::Developer, Group::Apps, Group::Personal, Group::System];
    pub fn label(self) -> &'static str {
        match self {
            Group::Caches => "Caches & junk",
            Group::Developer => "Developer",
            Group::Apps => "Apps & app data",
            Group::Personal => "Your files",
            Group::System => "System",
        }
    }
    pub fn blurb(self) -> &'static str {
        match self {
            Group::Caches => "Caches, logs, leftovers and downloaded updates. Mostly regenerates on its own.",
            Group::Developer => "Package caches, toolchains, build output and simulators. Usually re-downloadable.",
            Group::Apps => "Per-app data. Folders left behind by uninstalled apps are flagged as leftovers.",
            Group::Personal => "Your own files and media. Nothing here regenerates, so review by hand.",
            Group::System => "Managed by macOS. Usually reclaimed by restarting or by macOS itself.",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Paths,
    NodeModules,
    BuildDirs,
    Snapshots,
    /// .dmg/.pkg/.xip/.iso files in Downloads, Desktop and Documents, plus "Install macOS" apps.
    Installers,
}

pub struct Target {
    pub id: &'static str,
    pub name: &'static str,
    /// Paths may start with `~/`.
    pub paths: &'static [&'static str],
    pub safety: Safety,
    pub desc: &'static str,
    pub cmd: Option<&'static str>,
    pub kind: Kind,
    pub group: Group,
    /// Already counted inside another row; excluded from totals.
    pub nested: bool,
    /// Size each immediate subfolder so the inspector can classify them one by one.
    pub breakdown: bool,
}

const fn t(
    group: Group,
    id: &'static str,
    name: &'static str,
    paths: &'static [&'static str],
    safety: Safety,
    desc: &'static str,
    cmd: Option<&'static str>,
) -> Target {
    Target { id, name, paths, safety, desc, cmd, kind: Kind::Paths, group, nested: false, breakdown: false }
}

const fn special(group: Group, id: &'static str, name: &'static str, safety: Safety, kind: Kind, desc: &'static str, cmd: Option<&'static str>) -> Target {
    Target { id, name, paths: &[], safety, desc, cmd, kind, group, nested: false, breakdown: false }
}

/// Mark a target as broken down per subfolder.
const fn bd(t: Target) -> Target {
    Target { breakdown: true, ..t }
}

/// Mark a target as overlapping another (excluded from totals).
const fn nested(t: Target) -> Target {
    Target { nested: true, ..t }
}

use Group::*;
use Safety::*;

pub static TARGETS: &[Target] = &[
    // ---- Caches & junk ----
    bd(t(Caches, "user-caches", "User caches", &["~/Library/Caches"], Safe, "Caches written by your apps (browsers, package managers, updaters). Apps rebuild them; the first launch after clearing may be slower. Quit an app before clearing its folder. See the breakdown below for what each folder is.", Some("rm -rf ~/Library/Caches/<folder>   # one folder at a time, app quit"))),
    bd(t(Caches, "sys-caches", "System-wide caches", &["/Library/Caches"], Review, "Shared caches for all users. Mostly safe, but some belong to installers and updaters; clear per folder.", None)),
    bd(t(Caches, "user-logs", "User logs", &["~/Library/Logs"], Safe, "Application logs and crash reports. Only useful for debugging.", Some("rm -rf ~/Library/Logs/*"))),
    bd(special(Caches, "installers", "App installers (.dmg, .pkg)", Safe, Kind::Installers, "Disk images and installer packages in Downloads, Desktop and Documents, plus old \"Install macOS\" apps. Once an app is installed, its installer is only taking up space, and it can be downloaded again any time.", Some("rm ~/Downloads/<file>.dmg"))),
    t(Caches, "diag-reports", "Diagnostic reports", &["/Library/Logs/DiagnosticReports"], Safe, "Crash reports.", Some("sudo rm -rf /Library/Logs/DiagnosticReports/*")),
    t(Caches, "trash", "Trash", &["~/.Trash"], Safe, "Already deleted by you; space is only freed when emptied.", Some("Finder > Empty Trash")),
    t(Caches, "saved-state", "Saved app state", &["~/Library/Saved Application State"], Safe, "Window positions restored when apps reopen. Harmless to clear.", Some("rm -rf ~/Library/Saved\\ Application\\ State/*")),
    t(Caches, "sys-updates", "macOS update leftovers", &["/Library/Updates", "/macOS Install Data", "/private/var/db/softwareupdate"], Safe, "Downloaded updates and installers.", None),
    t(Caches, "ios-updates", "iPhone/iPad update files", &["~/Library/iTunes/iPhone Software Updates", "~/Library/iTunes/iPad Software Updates"], Safe, "Downloaded .ipsw firmware. Re-downloaded if ever needed.", Some("rm -rf ~/Library/iTunes/iP*\\ Software\\ Updates/*")),
    t(Caches, "mail-downloads", "Mail attachment copies", &["~/Library/Containers/com.apple.mail/Data/Library/Mail Downloads"], Safe, "Copies of attachments you opened from Mail. The originals stay in the messages.", Some("rm -rf ~/Library/Containers/com.apple.mail/Data/Library/Mail\\ Downloads/*")),
    // ---- Developer ----
    t(Developer, "xcode-derived", "Xcode DerivedData", &["~/Library/Developer/Xcode/DerivedData"], Safe, "Build products. Xcode regenerates on next build.", Some("rm -rf ~/Library/Developer/Xcode/DerivedData")),
    t(Developer, "xcode-archives", "Xcode Archives", &["~/Library/Developer/Xcode/Archives"], Review, "Archived app builds, needed to re-symbolicate or re-submit old releases.", None),
    t(Developer, "xcode-devsupport", "Xcode iOS DeviceSupport", &["~/Library/Developer/Xcode/iOS DeviceSupport"], Safe, "Debug symbols per iOS version you plugged in. Re-downloads on next connect.", Some("rm -rf ~/Library/Developer/Xcode/iOS\\ DeviceSupport/<old-version>")),
    t(Developer, "simulators", "iOS Simulators", &["~/Library/Developer/CoreSimulator"], Review, "Simulator runtimes and app data. Unavailable ones are safe to drop.", Some("xcrun simctl delete unavailable")),
    t(Developer, "android", "Android SDK & emulators", &["~/Library/Android/sdk", "~/.android/avd"], Review, "SDK platforms, system images and emulator disks. Remove old images in Android Studio's SDK Manager.", None),
    t(Developer, "docker", "Docker data", &["~/Library/Containers/com.docker.docker", "~/.docker"], Review, "Images, containers and volumes in one sparse disk image. Prune unused ones rather than deleting the file.", Some("docker system prune -a --volumes")),
    nested(t(Developer, "brew-cache", "Homebrew cache", &["~/Library/Caches/Homebrew"], Safe, "Downloaded bottles. (Already counted inside User caches.)", Some("brew cleanup -s"))),
    t(Developer, "npm", "npm cache", &["~/.npm"], Safe, "Package download cache.", Some("npm cache clean --force")),
    t(Developer, "pnpm-yarn", "pnpm / yarn / bun stores", &["~/Library/pnpm", "~/.pnpm-store", "~/.yarn", "~/.bun/install/cache"], Safe, "Package stores; re-downloaded on demand.", Some("pnpm store prune; bun pm cache rm")),
    t(Developer, "pip-cache", "pip / uv / conda caches", &["~/.cache/pip", "~/.cache/uv", "~/miniconda3/pkgs", "~/anaconda3/pkgs", "~/miniforge3/pkgs"], Safe, "Python package caches.", Some("pip cache purge; uv cache clean; conda clean -a")),
    t(Developer, "go-cache", "Go module cache", &["~/go/pkg"], Safe, "Downloaded Go dependencies; re-downloaded on demand. Use the Go command, rm can't delete its read-only files.", Some("go clean -modcache")),
    t(Developer, "cargo-gradle", "Cargo / Gradle / Maven", &["~/.cargo/registry", "~/.gradle/caches", "~/.gradle/wrapper/dists", "~/.m2/repository"], Safe, "Language toolchain caches; re-downloaded on demand.", Some("cargo cache -a   # needs cargo-cache")),
    bd(t(Developer, "toolchains", "Installed toolchains (rustup, nvm, pyenv)", &["~/.rustup/toolchains", "~/.nvm/versions/node", "~/.pyenv/versions"], Review, "Language versions you installed. Remove old ones you no longer use.", Some("rustup toolchain list; nvm ls; pyenv versions"))),
    t(Developer, "hf-ollama", "AI models (HuggingFace, Ollama, LM Studio)", &["~/.cache/huggingface", "~/.ollama", "~/.lmstudio", "~/.cache/torch"], Review, "Model weights, multi-GB each. Re-downloadable but slow; remove ones you no longer use.", Some("ollama list; ollama rm <model>")),
    special(Developer, "node_modules", "node_modules (all projects)", Safe, Kind::NodeModules, "Dependencies in your projects; Running npm install brings them back. Delete for projects you're not actively working on.", Some("rm -rf <project>/node_modules")),
    special(Developer, "build-dirs", "venvs / Rust target / build dirs", Safe, Kind::BuildDirs, "Rebuildable artifacts: .venv, venv, target (Rust), .next, dist-newstyle.", Some("rm -rf <project>/.venv  <project>/target")),
    // ---- Apps & app data ----
    nested(bd(t(Apps, "app-support", "Application Support (all apps)", &["~/Library/Application Support"], Review, "Where apps keep their data and settings. Folders for apps you've uninstalled are leftovers and can go. (Overlaps the app rows below.)", None))),
    nested(bd(t(Apps, "containers", "App Containers (sandboxed)", &["~/Library/Containers"], Review, "Per-app sandbox data. Containers for apps you've uninstalled are leftovers.", None))),
    bd(t(Apps, "group-containers", "Group Containers", &["~/Library/Group Containers"], Review, "Data shared between an app and its extensions (Office, Notes, WhatsApp, etc).", None)),
    t(Apps, "claude-app", "Claude desktop app data", &["~/Library/Application Support/Claude"], Review, "App data incl. session scratch workspaces and VM bundles.", None),
    t(Apps, "vscode", "VS Code / Cursor data", &["~/Library/Application Support/Code", "~/Library/Application Support/Cursor"], Review, "Extensions, workspace storage and caches. The Cache, CachedData and CachedExtensionVSIXs folders inside are safe; keep User.", None),
    t(Apps, "slack-discord", "Slack / Discord / Teams", &["~/Library/Application Support/Slack", "~/Library/Application Support/discord", "~/Library/Application Support/Microsoft/Teams"], Safe, "Mostly cached media; the apps re-download it.", None),
    t(Apps, "chrome", "Chrome profile", &["~/Library/Application Support/Google/Chrome"], Review, "Profile data, extensions, service-worker caches. Clear via browser settings, not by hand.", None),
    t(Apps, "games", "Games (Steam, Epic)", &["~/Library/Application Support/Steam/steamapps", "/Users/Shared/Epic Games"], Careful, "Installed games. Uninstall from the launcher to free space; re-downloadable.", None),
    t(Apps, "apps", "Applications", &["/Applications", "~/Applications"], Review, "Installed apps. Remove ones you never open.", None),
    // ---- Your files ----
    t(Personal, "downloads", "Downloads", &["~/Downloads"], Review, "Your downloads, not counting installers (those are under App installers). Zips and old files; only you know what matters.", None),
    t(Personal, "ios-backups", "iPhone/iPad backups", &["~/Library/Application Support/MobileSync/Backup"], Careful, "Local device backups. Often huge and forgotten; delete only if you have iCloud or another backup.", None),
    t(Personal, "mail", "Apple Mail", &["~/Library/Mail"], Careful, "Local mail store and attachments. Deleting loses offline mail.", None),
    t(Personal, "messages", "Messages attachments", &["~/Library/Messages"], Careful, "iMessage history and attachments. Manage via Messages > Settings.", None),
    t(Personal, "photos", "Photos library", &["~/Pictures/Photos Library.photoslibrary"], Careful, "Your photos. Use iCloud 'Optimize Mac Storage' to shrink.", None),
    t(Personal, "icloud", "iCloud Drive local copy", &["~/Library/Mobile Documents"], Careful, "Locally downloaded iCloud files. Right-click > Remove Download in Finder to evict.", None),
    t(Personal, "movies-music", "Movies / Music", &["~/Movies", "~/Music"], Careful, "Your media.", None),
    // ---- System ----
    t(Group::System, "tmp-folders", "Temp folders", &["/private/var/folders"], Safety::System, "Per-user temp and cache dirs. macOS cleans on reboot; a restart reclaims much of it.", Some("Restart your Mac")),
    t(Group::System, "swap", "Swap / VM", &["/private/var/vm"], Safety::System, "Virtual memory swap files. Grows under memory pressure; freed on restart.", Some("Restart your Mac")),
    t(Group::System, "sys-data-db", "System databases", &["/private/var/db"], Safety::System, "Spotlight, receipts, etc. Leave alone.", None),
    t(Group::System, "spotlight", "Spotlight index", &["/System/Volumes/Data/.Spotlight-V100"], Safety::System, "Search index. If abnormally large, `sudo mdutil -E /` rebuilds it.", Some("sudo mdutil -E /")),
    special(Group::System, "snapshots", "Local Time Machine snapshots", Safety::System, Kind::Snapshots, "Invisible APFS snapshots. macOS purges them when space is needed; their size isn't reported.", Some("tmutil listlocalsnapshots /   then   sudo tmutil deletelocalsnapshots <date>")),
];

pub fn find(id: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.id == id)
}

pub fn expand(p: &str) -> std::path::PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => p.into(),
    }
}

/// One plain-language line per location, for people who don't know macOS internals.
pub fn blurb(id: &str) -> &'static str {
    match id {
        "user-caches" => "Temporary files apps keep so they load faster. Apps rebuild them.",
        "installers" => "Downloaded app installers. Not needed once the app is installed.",
        "sys-caches" => "Caches shared by every user account on this Mac.",
        "user-logs" => "Notes apps write about what they did. Only useful for debugging.",
        "diag-reports" => "Crash reports macOS saves when something crashes.",
        "trash" => "Files you deleted. The space comes back when you empty the Trash.",
        "saved-state" => "Remembers where app windows were, so they reopen in place.",
        "sys-updates" => "macOS update files left behind after an update installed.",
        "ios-updates" => "iPhone/iPad software updates downloaded by Finder.",
        "mail-downloads" => "Copies of attachments you opened in Mail.",
        "xcode-derived" => "Xcode build output. Rebuilt the next time you build.",
        "xcode-archives" => "App builds you archived in Xcode for release.",
        "xcode-devsupport" => "Debug files for each iPhone version you plugged in.",
        "simulators" => "Virtual iPhones and iPads used by Xcode.",
        "android" => "Android SDK and emulator disk images.",
        "docker" => "Docker images, containers and volumes.",
        "brew-cache" => "Downloads Homebrew keeps after installing packages.",
        "npm" => "Copies of JavaScript packages npm downloaded.",
        "pnpm-yarn" => "Package stores for pnpm, Yarn and Bun.",
        "pip-cache" => "Downloaded Python packages (pip, uv, conda).",
        "go-cache" => "Go libraries downloaded for your projects.",
        "cargo-gradle" => "Rust, Java and Kotlin libraries downloaded for your projects.",
        "toolchains" => "Versions of Rust, Node.js and Python you installed.",
        "hf-ollama" => "AI model files downloaded to run locally.",
        "node_modules" => "Libraries inside your JavaScript projects. Running npm install brings them back.",
        "build-dirs" => "Build output and Python environments inside your projects.",
        "app-support" => "Where apps store their data and settings.",
        "containers" => "Data for App Store and other sandboxed apps.",
        "group-containers" => "Data an app shares with its widgets and extensions.",
        "claude-app" => "The Claude desktop app's data and workspaces.",
        "vscode" => "VS Code and Cursor extensions, caches and settings.",
        "slack-discord" => "Images and files your chat apps cached.",
        "chrome" => "Your Chrome profile: history, extensions and cache.",
        "games" => "Games installed through Steam or Epic.",
        "apps" => "The apps installed on this Mac.",
        "downloads" => "Files you downloaded (installers are counted separately).",
        "ios-backups" => "Backups of your iPhone or iPad stored on this Mac.",
        "mail" => "Your email and attachments, stored for offline use.",
        "messages" => "Your iMessage history, photos and videos.",
        "photos" => "Your Photos library.",
        "icloud" => "iCloud Drive files downloaded to this Mac.",
        "movies-music" => "Your video and music files.",
        "tmp-folders" => "macOS temporary files. Most are cleared when you restart.",
        "swap" => "Overflow for memory. Shrinks when you restart.",
        "sys-data-db" => "macOS's internal databases. Leave these alone.",
        "spotlight" => "The index Spotlight uses to search your files.",
        "snapshots" => "Hidden Time Machine snapshots. macOS deletes them when space runs low.",
        _ => "",
    }
}
