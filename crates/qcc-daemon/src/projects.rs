//! Checks on what a client asks a project to be.

use std::path::Path;

use qcc_domain::ProjectIcon;

/// A project needs a name and at least one folder; every folder must be an
/// existing directory given by an absolute path. Folders are kept in the order
/// given, without trailing slashes and without repeats.
pub(crate) fn check(name: &str, folders: &[String]) -> Result<(String, Vec<String>), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a project needs a name".into());
    }
    if folders.is_empty() {
        return Err("a project needs at least one folder".into());
    }
    let mut kept: Vec<String> = Vec::new();
    for folder in folders {
        let trimmed = folder.trim();
        let normal = if trimmed.len() > 1 { trimmed.trim_end_matches('/') } else { trimmed };
        let path = Path::new(normal);
        if !path.is_absolute() || !path.is_dir() {
            return Err(format!("folder {folder:?} is not an absolute path to a directory"));
        }
        if !kept.iter().any(|known| known == normal) {
            kept.push(normal.to_owned());
        }
    }
    Ok((name.to_owned(), kept))
}

/// An icon is optional; one that is given must be well formed.
pub(crate) fn check_icon(icon: Option<ProjectIcon>) -> Result<Option<ProjectIcon>, String> {
    icon.map(ProjectIcon::checked).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_checked_and_tidied() {
        let dir = std::env::temp_dir();
        let dir = dir.to_str().unwrap().trim_end_matches('/').to_owned();
        let (name, folders) =
            check(" App ", &[format!("{dir}/"), dir.clone()]).expect("a valid project");
        assert_eq!(name, "App");
        assert_eq!(folders, std::slice::from_ref(&dir));

        assert!(check("", std::slice::from_ref(&dir)).is_err());
        assert!(check("App", &[]).is_err());
        assert!(check("App", &["relative".into()]).is_err());
        assert!(check("App", &[format!("{dir}/no-such-dir-qcc")]).is_err());
    }
}
