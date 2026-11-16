//! Finding and loading plugins.
//!
//! Bundled plugins take the same road as everyone else: they are installed into the
//! plugin directory and loaded from there. The one difference is trust — the user
//! chose the daemon, so its own plugins get what they request. Any other plugin gets
//! only what a grant file gives it.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use qcc_domain::{Delta, SkillRef, ThreadId};
use qcc_plugin_host::services::DirKv;
use qcc_plugin_host::{
    Contribution, Grant, MANIFEST_FILE, Permissions, PluginHost, ProgramPaths, Services,
    ThreadsCall,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::Config;
use crate::core::Command;

pub(crate) fn is_bundled(id: &str) -> bool {
    qcc_plugins_bundle::plugin(id).is_some()
}

/// What the user granted a third-party plugin: `<data>/grants/<id>.toml`, in the
/// format of a manifest's `[permissions]` table. No file means no permissions.
fn grant_for(config: &Config, id: &str) -> Grant {
    if is_bundled(id) {
        return Grant::AllRequested;
    }
    let path = config.grants_dir().join(format!("{id}.toml"));
    let granted = match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str::<Permissions>(&text).unwrap_or_else(|error| {
            tracing::error!(plugin = id, %error, "ignoring an invalid grant file");
            Permissions::default()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(plugin = id, grant = %path.display(), "plugin has no granted permissions");
            Permissions::default()
        }
        Err(error) => {
            tracing::error!(plugin = id, grant = %path.display(), %error, "cannot read the grant file; granting nothing");
            Permissions::default()
        }
    };
    Grant::Only(granted)
}

pub(crate) fn load_all(
    config: &Config,
    commands: &UnboundedSender<Command>,
    programs: &ProgramPaths,
) -> anyhow::Result<PluginHost> {
    let plugins_dir = config.plugins_dir();
    qcc_plugins_bundle::install(&plugins_dir)?;

    let mut host = PluginHost::with_cache(Some(&config.cache_dir.join("wasm")))?;
    let mut dirs: Vec<_> = std::fs::read_dir(&plugins_dir)?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.join(MANIFEST_FILE).is_file())
        .collect();
    dirs.sort();

    for dir in dirs {
        // The directory name is only a location; the manifest names the plugin.
        let id = match plugin_id(&dir) {
            Some(id) => id,
            None => continue,
        };
        let grant = grant_for(config, &id);
        let loaded = host.load_dir(&dir, &grant, |manifest| {
            let deltas = commands.clone();
            let crashes = commands.clone();
            let threads = commands.clone();
            Services {
                deltas: Arc::new(move |plugin: &str, thread: ThreadId, deltas_: Vec<Delta>| {
                    let _ = deltas.send(Command::Deltas {
                        plugin: plugin.to_owned(),
                        thread,
                        deltas: deltas_,
                    });
                }),
                kv: Box::new(DirKv::new(config.plugin_data_dir(&manifest.id).join("kv"))),
                processes: Arc::new(qcc_plugin_host::services::OsProcesses::with_paths(
                    programs.clone(),
                )),
                threads: Arc::new(move |plugin: &str, call: ThreadsCall| {
                    let (reply, answer) = std::sync::mpsc::channel();
                    // Past the deadline the core refuses the call instead of running
                    // it: a call reported as failed must not happen after all.
                    let deadline = Instant::now() + THREADS_TIMEOUT;
                    threads
                        .send(Command::Threads { plugin: plugin.to_owned(), call, deadline, reply })
                        .map_err(|_| "the daemon is shutting down".to_owned())?;
                    // The core answers at once; it never waits on a plugin. It is not
                    // running yet while plugins are being loaded. The wait outlasts the
                    // deadline, so whatever the core did is what the plugin hears.
                    answer.recv_timeout(THREADS_TIMEOUT + THREADS_GRACE).map_err(|_| {
                        "the daemon did not answer; threads are not available during init"
                            .to_owned()
                    })?
                }),
                on_crash: Arc::new(move |plugin: &str, reason: &str| {
                    let _ = crashes.send(Command::PluginCrashed {
                        plugin: plugin.to_owned(),
                        reason: reason.to_owned(),
                    });
                }),
            }
        });
        match loaded {
            Ok(plugin) => tracing::info!(
                plugin = plugin.id(),
                version = %plugin.manifest().version,
                "plugin loaded"
            ),
            // One broken plugin must not take the daemon down.
            Err(error) => tracing::error!(dir = %dir.display(), %error, "plugin failed to load"),
        }
    }
    Ok(host)
}

/// How long a plugin's `threads` call may wait for the core.
const THREADS_TIMEOUT: Duration = Duration::from_secs(5);
/// How much longer a plugin waits than the core may take to start its call.
const THREADS_GRACE: Duration = Duration::from_secs(1);

/// A skill a plugin offers, written out where agents read it.
#[derive(Debug, Clone)]
pub(crate) struct OfferedSkill {
    pub reference: SkillRef,
    pub plugin: String,
    /// Given only to threads that ask for it.
    pub on_request: bool,
}

/// Write the skills of plugins allowed to offer them where agents can read them,
/// replacing what an earlier start wrote. A name is the skill's handle, so the
/// first plugin to offer one keeps it.
pub(crate) fn write_skills(config: &Config, host: &PluginHost) -> Vec<OfferedSkill> {
    let root = config.skills_dir();
    if root.exists()
        && let Err(error) = std::fs::remove_dir_all(&root)
    {
        tracing::error!(dir = %root.display(), %error, "cannot clear old skills");
    }
    let mut skills = Vec::new();
    for plugin in host.plugins() {
        for contribution in plugin.contributions() {
            let Contribution::Skill(skill) = contribution else { continue };
            if !plugin.permissions().skills {
                tracing::warn!(
                    plugin = plugin.id(),
                    skill = skill.name,
                    "skill ignored: the plugin may not offer skills"
                );
                continue;
            }
            if !qcc_plugin_host::valid_name(&skill.name) {
                tracing::warn!(
                    plugin = plugin.id(),
                    skill = skill.name,
                    "skill ignored: invalid name"
                );
                continue;
            }
            if skills.iter().any(|offered: &OfferedSkill| offered.reference.name == skill.name) {
                tracing::warn!(
                    plugin = plugin.id(),
                    skill = skill.name,
                    "skill ignored: another plugin offers one of that name"
                );
                continue;
            }
            let dir = root.join(plugin.id()).join(&skill.name);
            let path = dir.join("SKILL.md");
            // A JSON string is a valid YAML scalar, whatever the description says.
            let description = serde_json::to_string(&skill.description).expect("strings serialize");
            let text = format!(
                "---\nname: {}\ndescription: {description}\n---\n\n{}\n",
                skill.name, skill.body
            );
            if let Err(error) =
                std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, text))
            {
                tracing::error!(path = %path.display(), %error, "cannot write a skill");
                continue;
            }
            skills.push(OfferedSkill {
                reference: SkillRef {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    path: path.display().to_string(),
                },
                plugin: plugin.id().to_owned(),
                on_request: skill.on_request,
            });
        }
    }
    skills
}

fn plugin_id(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    match qcc_plugin_host::Manifest::parse(&text) {
        Ok(manifest) => Some(manifest.id),
        Err(error) => {
            tracing::error!(dir = %dir.display(), %error, "invalid plugin manifest");
            None
        }
    }
}

/// The largest file of a web bundle the daemon will serve.
const MAX_ASSET_BYTES: u64 = 16 * 1024 * 1024;

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()).unwrap_or_default() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Read one file of a plugin's web bundle. The path may not leave the bundle, by
/// its spelling or through a symlink.
pub(crate) fn read_asset(
    config: &Config,
    manifest: &qcc_plugin_host::Manifest,
    path: &str,
) -> Result<(Vec<u8>, &'static str), String> {
    let web = manifest.web.as_ref().ok_or("the plugin has no web bundle")?;
    let relative = Path::new(path);
    let inside = !path.is_empty()
        && relative.components().all(|part| matches!(part, std::path::Component::Normal(_)));
    if !inside {
        return Err(format!("{path:?} is not a path inside the bundle"));
    }
    let plugins_dir = config.plugins_dir();
    let dir = std::fs::read_dir(&plugins_dir)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .find(|dir| plugin_id(dir).as_deref() == Some(manifest.id.as_str()))
        .ok_or("the plugin's directory is gone")?;
    let root = dir.join(web).canonicalize().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => "the plugin's web bundle is missing".to_owned(),
        _ => format!("cannot open the plugin's web bundle: {error}"),
    })?;
    let file = root.join(relative).canonicalize().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => format!("no {path:?} in the bundle"),
        _ => format!("cannot open {path:?}: {error}"),
    })?;
    if !file.starts_with(&root) || !file.is_file() {
        return Err(format!("no {path:?} in the bundle"));
    }
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    if size > MAX_ASSET_BYTES {
        return Err(format!("{path:?} is too large to serve"));
    }
    let data = std::fs::read(&file).map_err(|error| error.to_string())?;
    Ok((data, content_type(&file)))
}
