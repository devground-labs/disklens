//! AI storage: local models, assistant data and ML caches, itemized per model or folder.

use crate::scan::{self, dir_size};
use crate::targets::{self, Safety};
use rayon::prelude::*;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Items untouched for this long are called out as stale.
pub const STALE_SECS: u64 = 90 * 86_400;
/// Smallest loose model file worth listing.
const MIN_STRAY_BYTES: u64 = 200_000_000;
const MODEL_EXT: [&str; 5] = ["gguf", "safetensors", "ckpt", "pth", "onnx"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiGroup {
    Models,
    Media,
    Assistants,
    Frameworks,
}

impl AiGroup {
    pub const ALL: [AiGroup; 4] = [AiGroup::Models, AiGroup::Media, AiGroup::Assistants, AiGroup::Frameworks];
    pub fn label(self) -> &'static str {
        match self {
            AiGroup::Models => "Local language models",
            AiGroup::Media => "Image & audio models",
            AiGroup::Assistants => "Coding assistants & chat apps",
            AiGroup::Frameworks => "ML frameworks & datasets",
        }
    }
}

pub struct Tool {
    pub id: &'static str,
    pub name: &'static str,
    pub group: AiGroup,
    pub paths: &'static [&'static str],
    pub safety: Safety,
    pub desc: &'static str,
    pub cmd: Option<&'static str>,
    /// Item per `publisher/model` (two levels) instead of one per folder.
    pub nest: bool,
}

const fn tool(group: AiGroup, id: &'static str, name: &'static str, paths: &'static [&'static str], safety: Safety, desc: &'static str, cmd: Option<&'static str>) -> Tool {
    Tool { id, name, group, paths, safety, desc, cmd, nest: false }
}

const fn nest(t: Tool) -> Tool {
    Tool { nest: true, ..t }
}

use AiGroup::*;
use Safety::*;

pub static TOOLS: &[Tool] = &[
    // ---- Local language models ----
    tool(Models, "ollama", "Ollama", &["~/.ollama"], Review, "Models you pulled with Ollama. Each model is a few large layers; layers shared between models are only freed once no model uses them.", Some("ollama list; ollama rm <model>")),
    nest(tool(Models, "lmstudio", "LM Studio", &["~/.lmstudio/models", "~/.cache/lm-studio/models"], Review, "Model files downloaded in LM Studio, one folder per model. Delete from LM Studio's My Models tab, or remove the folder.", None)),
    tool(Models, "hf", "Hugging Face cache", &["~/.cache/huggingface"], Review, "Models and datasets downloaded by transformers, diffusers and similar libraries. Fetched again the next time code asks for them.", Some("huggingface-cli delete-cache   # interactive picker")),
    tool(Models, "llamacpp", "llama.cpp", &["~/.cache/llama.cpp"], Review, "GGUF models downloaded by llama.cpp.", None),
    tool(Models, "gpt4all", "GPT4All", &["~/Library/Application Support/nomic.ai/GPT4All", "~/.cache/gpt4all"], Review, "Models and chat history of the GPT4All app.", None),
    tool(Models, "jan", "Jan", &["~/Library/Application Support/Jan", "~/jan"], Review, "Models, threads and extensions of the Jan app.", None),
    // ---- Image & audio ----
    tool(Media, "comfyui", "ComfyUI", &["~/ComfyUI", "~/Documents/ComfyUI"], Review, "Checkpoints, LoRAs and outputs. Generated images in output/ are your own files.", None),
    tool(Media, "sd-webui", "Stable Diffusion WebUI / InvokeAI", &["~/stable-diffusion-webui", "~/invokeai"], Review, "Checkpoints, embeddings and generated images.", None),
    tool(Media, "drawthings", "Draw Things / DiffusionBee", &["~/Library/Containers/com.liuliu.draw-things", "~/.diffusionbee"], Review, "Downloaded image models and generated projects.", None),
    tool(Media, "whisper", "Whisper", &["~/.cache/whisper"], Safe, "Speech-to-text model weights. Downloaded again on next use.", Some("rm -rf ~/.cache/whisper")),
    // ---- Coding assistants & chat apps ----
    tool(Assistants, "claude-code", "Claude Code", &["~/.claude"], Review, "Conversation transcripts, file-edit history, plugins and settings. Transcripts are what lets you resume old sessions.", None),
    tool(Assistants, "claude-desktop", "Claude desktop app", &["~/Library/Application Support/Claude"], Review, "App data including session workspaces and VM bundles.", None),
    tool(Assistants, "cursor", "Cursor", &["~/.cursor", "~/Library/Application Support/Cursor"], Review, "Extensions, indexed codebases, workspace storage and caches.", None),
    tool(Assistants, "codeium", "Windsurf / Codeium", &["~/.codeium", "~/Library/Application Support/Windsurf"], Review, "Language servers, embeddings and workspace storage.", None),
    tool(Assistants, "copilot", "GitHub Copilot", &["~/.config/github-copilot"], Safe, "Copilot settings and cache.", None),
    tool(Assistants, "gemini-codex", "Gemini CLI / Codex CLI", &["~/.gemini", "~/.codex"], Review, "Session history, logs and settings of the command-line agents.", None),
    tool(Assistants, "continue-aider", "Continue / Aider", &["~/.continue", "~/.aider"], Review, "Indexes, caches and chat history.", None),
    tool(Assistants, "chatgpt", "ChatGPT desktop app", &["~/Library/Application Support/com.openai.chat"], Review, "Conversations and cache of the ChatGPT app.", None),
    // ---- ML frameworks & datasets ----
    tool(Frameworks, "torch", "PyTorch hub", &["~/.cache/torch"], Safe, "Pretrained weights fetched by torch.hub and torchvision. Downloaded again when code asks for them.", Some("rm -rf ~/.cache/torch")),
    tool(Frameworks, "keras-tf", "Keras / TensorFlow", &["~/.keras", "~/tensorflow_datasets"], Safe, "Pretrained weights and datasets. Downloaded again when code asks for them.", Some("rm -rf ~/.keras ~/tensorflow_datasets")),
    tool(Frameworks, "kaggle-ms", "Kaggle / ModelScope", &["~/.cache/kagglehub", "~/.cache/modelscope"], Safe, "Datasets and models fetched by kagglehub and ModelScope.", None),
];

#[derive(Clone, Debug)]
pub struct AiItem {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub safety: Safety,
    pub note: String,
    pub cmd: Option<String>,
    /// Last change in seconds since the epoch, 0 if unknown.
    pub modified: u64,
}

impl AiItem {
    pub fn stale(&self, now: u64) -> bool {
        self.modified > 0 && now.saturating_sub(self.modified) > STALE_SECS && self.safety != Safety::Careful
    }
}

#[derive(Clone, Debug)]
pub struct AiTool {
    pub id: &'static str,
    pub bytes: u64,
    pub denied: bool,
    pub modified: u64,
    pub items: Vec<AiItem>,
}

#[derive(Clone, Debug, Default)]
pub struct AiScan {
    pub ts: u64,
    pub secs: u64,
    pub tools: Vec<AiTool>,
    /// Model files outside every known location.
    pub stray: Vec<AiItem>,
}

pub fn find(id: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.id == id)
}

fn shorten(p: &Path) -> String {
    let home = format!("{}/", dirs::home_dir().unwrap_or_default().display());
    let s = p.to_string_lossy();
    s.strip_prefix(&home).map(|r| format!("~/{r}")).unwrap_or_else(|| s.into_owned())
}

/// Newest modification time of `p` and what's inside it, two levels down. A folder's own time only
/// moves when entries are added or removed, so it alone would call busy folders stale.
fn mtime(p: &Path) -> u64 {
    fn rec(p: &Path, depth: usize) -> u64 {
        let Ok(md) = fs::symlink_metadata(p) else { return 0 };
        let own = md.mtime().max(0) as u64;
        if !md.is_dir() || depth == 0 {
            return own;
        }
        let Ok(rd) = fs::read_dir(p) else { return own };
        rd.flatten().map(|e| rec(&e.path(), depth - 1)).fold(own, u64::max)
    }
    rec(p, 2)
}

/// Safety and explanation for a well-known folder inside a tool, falling back to the tool's own.
fn item_info(tool: &Tool, name: &str) -> (Safety, &'static str) {
    let known = match (tool.id, name) {
        ("claude-code", "projects") => Some((Careful, "Conversation transcripts for every project. Removing them means old sessions can't be resumed.")),
        ("claude-code", "file-history") => Some((Review, "Copies of files Claude edited, used to rewind changes. Old ones can go.")),
        ("claude-code", "plugins") => Some((Review, "Installed plugins and marketplaces. Reinstalled from /plugin.")),
        ("claude-code", "skills") => Some((Careful, "Your own skills. Nothing recreates these.")),
        ("claude-code", "backups") => Some((Review, "Backups of your settings file.")),
        ("claude-code", "shell-snapshots" | "session-env" | "telemetry" | "cache" | "ide" | "statsig") => Some((Safe, "Temporary state that is recreated as needed.")),
        ("ollama", "history" | "id_ed25519" | "id_ed25519.pub") => Some((Careful, "Your Ollama prompt history and identity key.")),
        ("hf", "xet" | "assets") => Some((Safe, "Download helper data. Recreated on demand.")),
        (_, "Cache" | "CachedData" | "Code Cache" | "GPUCache" | "logs" | "Crashpad" | "blob_storage") => Some((Safe, "Rebuilds itself. Quit the app before clearing.")),
        _ => None,
    };
    known.unwrap_or((tool.safety, ""))
}

fn make_item(tool: &Tool, name: String, path: &Path, bytes: u64) -> AiItem {
    let base = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (safety, note) = item_info(tool, &base);
    AiItem { name, path: path.to_string_lossy().into_owned(), bytes, safety, note: note.into(), cmd: None, modified: mtime(path) }
}

/// `models--org--name` becomes `org/name`; datasets and spaces are labelled by kind.
pub fn hf_name(dir: &str) -> Option<(String, &'static str)> {
    for (prefix, kind) in [("models--", "model"), ("datasets--", "dataset"), ("spaces--", "space")] {
        if let Some(rest) = dir.strip_prefix(prefix) {
            return Some((rest.replace("--", "/"), kind));
        }
    }
    None
}

/// Models listed by Ollama's manifests, sized from the layers each one references.
pub fn ollama_models(manifests: &Path) -> Vec<AiItem> {
    fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = vec![];
    collect(manifests, &mut files);
    let mut items: Vec<AiItem> = files
        .iter()
        .filter_map(|f| {
            let v: serde_json::Value = serde_json::from_slice(&fs::read(f).ok()?).ok()?;
            let layers = v.get("layers")?.as_array()?;
            let bytes = layers.iter().chain(v.get("config")).filter_map(|l| l.get("size")?.as_u64()).sum();
            let rel: Vec<String> = f.strip_prefix(manifests).ok()?.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            let name = match rel.as_slice() {
                [host, ns, model, tag] if host == "registry.ollama.ai" && ns == "library" => format!("{model}:{tag}"),
                [host, ns, model, tag] if host == "registry.ollama.ai" => format!("{ns}/{model}:{tag}"),
                [host, ns, model, tag] => format!("{host}/{ns}/{model}:{tag}"),
                _ => rel.join("/"),
            };
            Some(AiItem {
                path: f.to_string_lossy().into_owned(),
                bytes,
                safety: Review,
                note: "Pull it again with `ollama pull` any time. Layers shared with other models stay until none use them.".into(),
                cmd: Some(format!("ollama rm {name}")),
                modified: mtime(f),
                name,
            })
        })
        .collect();
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    items
}

/// Immediate children of `dir` as items; with `nest`, the children of those (publisher/model).
fn child_items(tool: &Tool, dir: &Path, denied: &mut bool) -> Vec<AiItem> {
    let kids = match scan::explore(dir) {
        Ok(k) => k,
        Err(e) => {
            *denied |= e.starts_with("Permission denied");
            return vec![];
        }
    };
    let mut out = vec![];
    for k in kids.into_iter().filter(|k| k.bytes > 0) {
        *denied |= k.denied;
        if tool.nest && k.is_dir {
            match scan::explore(&k.path) {
                Ok(inner) => out.extend(inner.into_iter().filter(|i| i.bytes > 0).map(|i| make_item(tool, format!("{}/{}", k.name, i.name), &i.path, i.bytes))),
                Err(_) => out.push(make_item(tool, k.name.clone(), &k.path, k.bytes)),
            }
        } else if let (Some((name, kind)), "hf") = (hf_name(&k.name), tool.id) {
            let mut it = make_item(tool, name, &k.path, k.bytes);
            it.safety = if kind == "model" { Review } else { Safe };
            it.note = format!("A {kind} downloaded from Hugging Face. It is fetched again the next time code asks for it.");
            it.cmd = Some(format!("rm -rf {}", shorten(&k.path)));
            out.push(it);
        } else {
            out.push(make_item(tool, k.name.clone(), &k.path, k.bytes));
        }
    }
    out
}

fn scan_tool(tool: &'static Tool) -> Option<AiTool> {
    let found: Vec<PathBuf> = tool.paths.iter().map(|p| targets::expand(p)).filter(|p| p.exists()).collect();
    if found.is_empty() {
        return None;
    }
    let mut denied = false;
    let mut bytes = 0;
    let mut items = vec![];
    for p in &found {
        let (b, d) = dir_size(p);
        bytes += b;
        denied |= d;
        if tool.id == "ollama" {
            items.extend(ollama_models(&p.join("models/manifests")));
            // Everything that isn't model data, such as history and keys.
            for k in scan::explore(p).unwrap_or_default().into_iter().filter(|k| k.name != "models" && k.bytes > 0) {
                items.push(make_item(tool, k.name.clone(), &k.path, k.bytes));
            }
        } else {
            items.extend(child_items(tool, p, &mut denied));
        }
    }
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    items.truncate(80);
    let modified = items.iter().map(|i| i.modified).max().unwrap_or(0);
    (bytes > 0).then_some(AiTool { id: tool.id, bytes, denied, modified, items })
}

/// Model-weight files outside every known location, found by extension and size.
fn find_stray(skip: &[PathBuf]) -> Vec<AiItem> {
    fn rec(dir: &Path, depth: usize, skip: &[PathBuf], out: &Mutex<Vec<AiItem>>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        let kids: Vec<_> = rd.flatten().collect();
        kids.par_iter().for_each(|e| {
            let Ok(ft) = e.file_type() else { return };
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || ft.is_symlink() {
                return;
            }
            if ft.is_dir() {
                let apps = name.ends_with(".app") || name == "node_modules" || name == "Library";
                if depth > 0 && !apps && !skip.iter().any(|s| p.starts_with(s)) {
                    rec(&p, depth - 1, skip, out);
                }
                return;
            }
            let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            if !MODEL_EXT.contains(&ext.as_str()) {
                return;
            }
            let Ok(md) = e.metadata() else { return };
            let bytes = md.blocks() * 512;
            if bytes >= MIN_STRAY_BYTES {
                let note = "A model file outside the usual AI folders. If a project still loads it, deleting it breaks that project.".to_string();
                out.lock().unwrap().push(AiItem { name, path: p.to_string_lossy().into_owned(), bytes, safety: Review, note, cmd: None, modified: md.mtime().max(0) as u64 });
            }
        });
    }
    let out = Mutex::new(vec![]);
    rec(&dirs::home_dir().unwrap_or_default(), 8, skip, &out);
    let mut v = out.into_inner().unwrap();
    v.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    v.truncate(60);
    v
}

pub fn run_scan() -> AiScan {
    let start = scan::now();
    let mut tools: Vec<AiTool> = TOOLS.par_iter().filter_map(scan_tool).collect();
    tools.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    let skip: Vec<PathBuf> = TOOLS.iter().flat_map(|t| t.paths.iter().map(|p| targets::expand(p))).collect();
    AiScan { ts: start, secs: scan::now() - start, tools, stray: find_stray(&skip) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hf_names() {
        assert_eq!(hf_name("models--meta-llama--Llama-3-8B"), Some(("meta-llama/Llama-3-8B".into(), "model")));
        assert_eq!(hf_name("datasets--squad"), Some(("squad".into(), "dataset")));
        assert_eq!(hf_name("blobs"), None);
    }

    #[test]
    fn ollama_manifest_sizes_and_names() {
        let root = std::env::temp_dir().join(format!("disklens-ollama-{}", std::process::id()));
        let lib = root.join("registry.ollama.ai/library/llama3");
        let user = root.join("registry.ollama.ai/someone/tuned");
        fs::create_dir_all(&lib).unwrap();
        fs::create_dir_all(&user).unwrap();
        fs::write(lib.join("8b"), r#"{"config":{"size":10},"layers":[{"size":100},{"size":5}]}"#).unwrap();
        fs::write(user.join("latest"), r#"{"layers":[{"size":7}]}"#).unwrap();
        fs::write(user.join("broken"), "not json").unwrap();
        let items = ollama_models(&root);
        fs::remove_dir_all(&root).unwrap();
        let by: Vec<(&str, u64)> = items.iter().map(|i| (i.name.as_str(), i.bytes)).collect();
        assert_eq!(by, vec![("llama3:8b", 115), ("someone/tuned:latest", 7)]);
        assert_eq!(items[0].cmd.as_deref(), Some("ollama rm llama3:8b"));
    }

    #[test]
    fn catalog_ids_are_unique() {
        let mut ids: Vec<_> = TOOLS.iter().map(|t| t.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), TOOLS.len());
    }
}
