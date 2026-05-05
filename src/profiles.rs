use crate::types::SearchMode;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub source: Option<String>,
    pub ref_name: Option<String>,
    pub mode: Option<SearchMode>,
    pub limit: Option<usize>,
    pub encoder: Option<String>,
    pub model: Option<String>,
    pub offline: Option<bool>,
    pub no_download: Option<bool>,
    pub cache_dir: Option<PathBuf>,
    pub no_cache: Option<bool>,
    pub project_cache: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ProfileStore {
    profiles: Vec<Profile>,
}

pub fn profile_store_path(cache_root: &Path) -> PathBuf {
    cache_root.join("profiles.json")
}

pub fn load_profiles(cache_root: &Path) -> Result<Vec<Profile>> {
    let path = profile_store_path(cache_root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("read profiles {}", path.display()))?;
    let store: ProfileStore =
        serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
    Ok(store.profiles)
}

pub fn save_profile(cache_root: &Path, profile: Profile) -> Result<()> {
    validate_profile_name(&profile.name)?;
    let mut profiles = load_profiles(cache_root)?;
    profiles.retain(|existing| existing.name != profile.name);
    profiles.push(profile);
    profiles.sort_by(|left, right| left.name.cmp(&right.name));
    write_profiles(cache_root, profiles)
}

pub fn get_profile(cache_root: &Path, name: &str) -> Result<Profile> {
    validate_profile_name(name)?;
    load_profiles(cache_root)?
        .into_iter()
        .find(|profile| profile.name == name)
        .with_context(|| {
            let names = profile_names(cache_root).unwrap_or_default().join(", ");
            if names.is_empty() {
                format!("profile {name:?} does not exist; no profiles are saved")
            } else {
                format!("profile {name:?} does not exist; available profiles: {names}")
            }
        })
}

pub fn delete_profile(cache_root: &Path, name: &str) -> Result<bool> {
    validate_profile_name(name)?;
    let mut profiles = load_profiles(cache_root)?;
    let original_len = profiles.len();
    profiles.retain(|profile| profile.name != name);
    let removed = profiles.len() != original_len;
    write_profiles(cache_root, profiles)?;
    Ok(removed)
}

pub fn profile_names(cache_root: &Path) -> Result<Vec<String>> {
    Ok(load_profiles(cache_root)?
        .into_iter()
        .map(|profile| profile.name)
        .collect())
}

fn write_profiles(cache_root: &Path, profiles: Vec<Profile>) -> Result<()> {
    fs::create_dir_all(cache_root)
        .with_context(|| format!("create cache root {}", cache_root.display()))?;
    let path = profile_store_path(cache_root);
    let store = ProfileStore { profiles };
    fs::write(&path, serde_json::to_string_pretty(&store)? + "\n")
        .with_context(|| format!("write profiles {}", path.display()))
}

fn validate_profile_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("profile name must not be empty");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        bail!("profile name may contain only letters, numbers, '.', '_' and '-'");
    }
    Ok(())
}
