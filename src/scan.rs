//! Disk scanning: parallel directory sizing, special finders, snapshots.

use crate::targets::{self, Kind, TARGETS};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Item {
    pub path: String,
    pub bytes: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct TargetResult {
    pub id: String,
    pub bytes: u64,
    pub denied: bool,
    pub items: Vec<Item>,
    pub note: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Disk {
    pub total: u64,
    pub free: u64,
}

impl Disk {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Snapshot {
    pub ts: u64,
    pub secs: u64,
    pub disk: Disk,
    pub targets: Vec<TargetResult>,
}

#[derive(Default)]
pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub current: Mutex<String>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

type Seen = Mutex<HashSet<(u64, u64)>>;

/// Allocated size of `path` (like `du -skx`): stays on one filesystem, counts hard links once.
fn walk(path: &Path, dev: u64, seen: &Seen, denied: &AtomicBool) -> u64 {
    let md = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                denied.store(true, Ordering::Relaxed);
            }
            return 0;
        }
    };
    let mut total = md.blocks() * 512;
    if md.is_dir() {
        if md.dev() != dev {
            return 0;
        }
        match fs::read_dir(path) {
            Ok(rd) => {
                let kids: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                total += kids.par_iter().map(|p| walk(p, dev, seen, denied)).sum::<u64>();
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    denied.store(true, Ordering::Relaxed);
                }
            }
        }
    } else if md.nlink() > 1 && !seen.lock().unwrap().insert((md.dev(), md.ino())) {
        return 0;
    }
    total
}

/// Returns (bytes, permission_denied_somewhere).
pub fn dir_size(path: &Path) -> (u64, bool) {
    let Ok(md) = fs::symlink_metadata(path) else { return (0, false) };
    let denied = AtomicBool::new(false);
    let seen = Mutex::new(HashSet::new());
    let b = walk(path, md.dev(), &seen, &denied);
    (b, denied.load(Ordering::Relaxed))
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// Find directories matching `pred` under $HOME (skipping Library, dotdirs, Trash), not descending into matches.
fn find_dirs(pred: &(dyn Fn(&Path) -> bool + Sync), max_depth: usize) -> Vec<PathBuf> {
    fn rec(dir: &Path, depth: usize, max: usize, pred: &(dyn Fn(&Path) -> bool + Sync), out: &Mutex<Vec<PathBuf>>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        let kids: Vec<_> = rd.flatten().collect();
        kids.par_iter().for_each(|e| {
            let Ok(ft) = e.file_type() else { return };
            if !ft.is_dir() {
                return;
            }
            let p = e.path();
            let name = e.file_name();
            let name = name.to_string_lossy();
            if depth == 0 && (name == "Library" || name == ".Trash") {
                return;
            }
            if depth == 0 && name.starts_with('.') {
                return;
            }
            if pred(&p) {
                out.lock().unwrap().push(p);
            } else if depth + 1 < max {
                rec(&p, depth + 1, max, pred, out);
            }
        });
    }
    let out = Mutex::new(vec![]);
    rec(&home(), 0, max_depth, pred, &out);
    out.into_inner().unwrap()
}

fn top_items(dirs: Vec<PathBuf>, limit: usize) -> (Vec<Item>, u64) {
    let mut items: Vec<Item> = dirs
        .par_iter()
        .map(|d| Item { path: d.to_string_lossy().into_owned(), bytes: dir_size(d).0 })
        .collect();
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    let total = items.iter().map(|i| i.bytes).sum();
    items.truncate(limit);
    (items, total)
}

const INSTALLER_EXT: [&str; 5] = ["dmg", "pkg", "mpkg", "xip", "iso"];

/// Installer files under Downloads/Desktop/Documents (4 levels deep), plus "Install macOS*.app".
fn find_installers() -> (Vec<Item>, u64) {
    fn rec(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            if INSTALLER_EXT.contains(&ext.as_str()) {
                out.push(p); // .pkg can be a directory bundle; dir_size handles both
            } else if ft.is_dir() && depth > 0 && ext != "app" {
                let name = e.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with('.') && name != "node_modules" {
                    rec(&p, depth - 1, out);
                }
            }
        }
    }
    let mut found = vec![];
    for d in ["Downloads", "Desktop", "Documents"] {
        rec(&home().join(d), 4, &mut found);
    }
    if let Ok(rd) = fs::read_dir("/Applications") {
        found.extend(rd.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Install macOS"))));
    }
    top_items(found, 200)
}

fn scan_target(t: &targets::Target) -> TargetResult {
    let mut r = TargetResult { id: t.id.into(), ..Default::default() };
    match t.kind {
        Kind::Paths if t.breakdown => {
            for p in t.paths {
                let Ok(kids) = explore(&targets::expand(p)) else { continue };
                for k in kids {
                    r.bytes += k.bytes;
                    r.denied |= k.denied;
                    r.items.push(Item { path: k.path.to_string_lossy().into_owned(), bytes: k.bytes });
                }
            }
            r.items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
            r.items.truncate(80);
        }
        Kind::Paths => {
            for p in t.paths {
                let (b, d) = dir_size(&targets::expand(p));
                r.bytes += b;
                r.denied |= d;
            }
        }
        Kind::NodeModules => {
            let (items, total) = top_items(find_dirs(&|p| p.file_name().is_some_and(|n| n == "node_modules"), 7), 25);
            r.items = items;
            r.bytes = total;
        }
        Kind::BuildDirs => {
            let dirs = find_dirs(
                &|p| {
                    let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    matches!(n.as_str(), ".venv" | "venv" | ".next" | "dist-newstyle")
                        || (n == "target" && p.parent().is_some_and(|pp| pp.join("Cargo.toml").exists()))
                },
                7,
            );
            let (items, total) = top_items(dirs, 25);
            r.items = items;
            r.bytes = total;
        }
        Kind::Installers => {
            let (items, total) = find_installers();
            r.items = items;
            r.bytes = total;
        }
        Kind::Snapshots => {
            if let Ok(out) = Command::new("tmutil").args(["listlocalsnapshots", "/"]).output() {
                let text = String::from_utf8_lossy(&out.stdout);
                let snaps: Vec<_> = text.lines().filter(|l| l.starts_with("com.apple")).collect();
                r.note = Some(format!("{} local snapshot(s)", snaps.len()));
                r.items = snaps.iter().map(|s| Item { path: s.to_string(), bytes: 0 }).collect();
            }
        }
    }
    r
}

pub fn disk_info() -> Disk {
    // `df -k` on the Data volume: 1K-blocks and available.
    let out = Command::new("df").args(["-k", "/System/Volumes/Data"]).output();
    if let Ok(out) = out {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(line) = text.lines().nth(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() >= 4 {
                if let (Ok(total), Ok(avail)) = (f[1].parse::<u64>(), f[3].parse::<u64>()) {
                    return Disk { total: total * 1024, free: avail * 1024 };
                }
            }
        }
    }
    Disk::default()
}

pub fn run_scan(progress: Arc<Progress>) -> Snapshot {
    let start = now();
    let active: Vec<&targets::Target> = TARGETS
        .iter()
        .filter(|t| t.kind != Kind::Paths || t.paths.iter().any(|p| targets::expand(p).exists()))
        .collect();
    progress.total.store(active.len(), Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let results: Vec<TargetResult> = active
        .par_iter()
        .map(|t| {
            *progress.current.lock().unwrap() = t.name.to_string();
            let r = scan_target(t);
            progress.done.fetch_add(1, Ordering::Relaxed);
            r
        })
        .collect();
    let mut results = results;
    let dl = home().join("Downloads").to_string_lossy().into_owned() + "/";
    let in_downloads: u64 = results.iter().filter(|r| r.id == "installers").flat_map(|r| &r.items).filter(|i| i.path.starts_with(&dl)).map(|i| i.bytes).sum();
    if let Some(d) = results.iter_mut().find(|r| r.id == "downloads") {
        d.bytes = d.bytes.saturating_sub(in_downloads);
    }
    Snapshot { ts: start, secs: now() - start, disk: disk_info(), targets: results }
}

/// Children of `dir` with sizes, largest first.
pub fn explore(dir: &Path) -> Result<Vec<ChildEntry>, String> {
    let rd = fs::read_dir(dir).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            "Permission denied. Grant Full Disk Access to this app in System Settings > Privacy & Security.".to_string()
        } else {
            e.to_string()
        }
    })?;
    let paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    let mut v: Vec<ChildEntry> = paths
        .par_iter()
        .filter(|p| fs::symlink_metadata(p).map(|m| !m.file_type().is_symlink()).unwrap_or(false))
        .map(|p| {
            let (bytes, denied) = dir_size(p);
            ChildEntry { name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), path: p.clone(), bytes, is_dir: p.is_dir(), denied }
        })
        .collect();
    v.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    Ok(v)
}

#[derive(Clone)]
pub struct ChildEntry {
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub is_dir: bool,
    pub denied: bool,
}
