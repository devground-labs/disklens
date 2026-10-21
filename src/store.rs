//! Persists scan history as JSON in ~/Library/Application Support/DiskLens.

use crate::scan::Snapshot;
use std::fs;
use std::path::PathBuf;

const MAX_SNAPSHOTS: usize = 400;

fn file() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("DiskLens");
    // Carry history over from before the rename.
    let old = base.join("SysDataMonitor");
    if !dir.exists() && old.exists() {
        let _ = fs::rename(&old, &dir);
    }
    let _ = fs::create_dir_all(&dir);
    dir.join("history.json")
}

pub fn load() -> Vec<Snapshot> {
    fs::read(file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(history: &[Snapshot]) {
    let start = history.len().saturating_sub(MAX_SNAPSHOTS);
    let f = file();
    let tmp = f.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_vec(&history[start..]) {
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(tmp, f);
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Settings {
    /// Alert when free space drops below this many GB (0 = off).
    pub threshold_gb: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { threshold_gb: 20 }
    }
}

fn settings_file() -> PathBuf {
    file().with_file_name("settings.json")
}

pub fn load_settings() -> Settings {
    fs::read(settings_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_settings(s: &Settings) {
    if let Ok(json) = serde_json::to_vec(s) {
        let _ = fs::write(settings_file(), json);
    }
}

/// Lightweight free-space samples taken by the menu-bar monitor between full scans.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
pub struct DiskSample {
    pub ts: u64,
    pub total: u64,
    pub free: u64,
}

const MAX_SAMPLES: usize = 20_000;

fn disk_log_file() -> PathBuf {
    file().with_file_name("disk_log.json")
}

pub fn load_disk_log() -> Vec<DiskSample> {
    fs::read(disk_log_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn append_disk_sample(s: DiskSample) {
    let mut log = load_disk_log();
    log.push(s);
    let start = log.len().saturating_sub(MAX_SAMPLES);
    let f = disk_log_file();
    let tmp = f.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_vec(&log[start..]) {
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(tmp, f);
        }
    }
}
