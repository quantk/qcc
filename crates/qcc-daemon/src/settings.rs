//! The user's settings: read at start, checked and written on every change.

use std::path::Path;

use qcc_protocol::methods::Settings;

/// Missing or unreadable settings are the defaults; a broken file is reported and
/// left for the user to look at, not overwritten until they save again.
pub(crate) fn load(path: &Path) -> Settings {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
            tracing::warn!(path = %path.display(), %error, "settings are not valid; using the defaults");
            Settings::default()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Settings::default(),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "cannot read the settings");
            Settings::default()
        }
    }
}

/// Written next to the old file and renamed over it, so a crash never leaves half
/// a file. Only the user may read it: agents' variables are often API keys.
pub(crate) fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let text = serde_json::to_string_pretty(settings).expect("settings serialize");
    let partial = path.with_extension("json.partial");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&partial)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&partial, path)
}

/// Settings that could not work are refused rather than saved.
pub(crate) fn check(settings: &Settings) -> Result<(), String> {
    for (name, path) in &settings.programs {
        if name.is_empty() || name.contains('/') {
            return Err(format!("{name:?} is not a program name"));
        }
        if !Path::new(path).is_absolute() {
            return Err(format!("the path of {name} must be absolute, not {path:?}"));
        }
    }
    for (provider, provider_settings) in &settings.providers {
        for name in provider_settings.env.keys() {
            let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !valid {
                return Err(format!("{name:?} is not a variable name ({provider})"));
            }
            // The loader's variables would let a setting change what runs.
            if name.starts_with("LD_") || name.starts_with("DYLD_") || name.starts_with("QCC_") {
                return Err(format!("{name} cannot be set for an agent ({provider})"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use qcc_domain::ProviderId;
    use qcc_protocol::methods::ProviderSettings;

    use super::*;

    #[test]
    fn settings_round_trip_and_survive_a_broken_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(load(&path), Settings::default());

        let mut settings = Settings::default();
        settings.programs.insert("claude".into(), "/opt/claude".into());
        save(&path, &settings).unwrap();
        assert_eq!(load(&path), settings);
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), Settings::default());
    }

    #[test]
    fn what_cannot_work_is_refused() {
        let with_env = |name: &str| {
            let mut settings = Settings::default();
            let mut provider = ProviderSettings::default();
            provider.env.insert(name.into(), "x".into());
            settings.providers.insert(ProviderId::new("codex"), provider);
            check(&settings)
        };
        assert!(with_env("OPENAI_API_KEY").is_ok());
        assert!(with_env("LD_PRELOAD").is_err());
        assert!(with_env("QCC_SOCKET").is_err());
        assert!(with_env("1BAD").is_err());

        let mut settings = Settings::default();
        settings.programs.insert("claude".into(), "relative/claude".into());
        assert!(check(&settings).is_err());
    }
}
