//! Local JSON store: read → normalize → diff → atomic write (no-op when unchanged).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::model::{Account, AccountInput, SCHEMA_VERSION, Store};

pub fn app_dir() -> AppResult<PathBuf> {
    let base =
        dirs::config_dir().ok_or_else(|| AppError::new("no_appdata", "APPDATA introuvable"))?;
    Ok(base.join("claude-account-manager"))
}

/// MSIX apps such as Claude Desktop silently redirect writes under %APPDATA% made by their child
/// processes to `%LOCALAPPDATA%\Packages\<pkg>\LocalCache\Roaming`. Terminals started from there
/// cannot see those files, so this returns the redirected copies of `dir`, if any.
pub fn package_copies(dir: &Path) -> Vec<PathBuf> {
    let (Some(local), Some(name)) = (dirs::data_local_dir(), dir.file_name()) else {
        return Vec::new();
    };
    fs::read_dir(local.join("Packages"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("LocalCache").join("Roaming").join(name))
        .filter(|p| p.is_dir())
        .collect()
}

/// True when a probe written to `dir` lands in a package's redirected copy.
pub fn is_virtualized(dir: &Path) -> bool {
    let probe = format!(".probe-{}", uuid::Uuid::new_v4());
    if fs::create_dir_all(dir)
        .and_then(|_| fs::write(dir.join(&probe), b""))
        .is_err()
    {
        return false;
    }
    let hit = package_copies(dir).iter().any(|c| c.join(&probe).exists());
    let _ = fs::remove_file(dir.join(&probe));
    hit
}

/// One-time import of data written while redirected. No-op once `dir` has its own store.
/// Junctions are skipped; `ensure_profile` recreates them on the next launch.
pub fn import_from(dir: &Path, candidates: &[PathBuf]) -> AppResult<Option<PathBuf>> {
    if store_path(dir).exists() {
        return Ok(None);
    }
    let Some(src) = candidates.iter().find(|c| store_path(c).exists()) else {
        return Ok(None);
    };
    let walk = walkdir::WalkDir::new(src)
        .into_iter()
        .filter_entry(|e| !e.path_is_symlink() && !junction::exists(e.path()).unwrap_or(false));
    for entry in walk {
        let entry = entry.map_err(|e| AppError::new("import_failed", e.to_string()))?;
        let rel = entry
            .path()
            .strip_prefix(src)
            .map_err(|e| AppError::new("import_failed", e.to_string()))?;
        let target = dir.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if !target.exists() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(Some(src.clone()))
}

pub fn store_path(dir: &Path) -> PathBuf {
    dir.join("accounts.json")
}

pub fn load(dir: &Path) -> AppResult<Store> {
    let path = store_path(dir);
    if !path.exists() {
        return Ok(Store::default());
    }
    let raw = fs::read_to_string(&path)?;
    let store: Store = serde_json::from_str(&raw).map_err(|e| {
        AppError::new(
            "store_corrupt",
            format!(
                "{} est illisible ({e}). Restaurez {}.bak ou corrigez le fichier.",
                path.display(),
                path.display()
            ),
        )
    })?;
    if store.schema_version > SCHEMA_VERSION {
        return Err(AppError::new(
            "store_too_new",
            format!(
                "Schéma v{} plus récent que l'app (v{SCHEMA_VERSION}). Mettez l'app à jour.",
                store.schema_version
            ),
        ));
    }
    Ok(store)
}

/// Returns `true` when the file was written, `false` when already converged.
pub fn save(dir: &Path, store: &Store) -> AppResult<bool> {
    validate(store)?;
    let path = store_path(dir);
    let mut content = serde_json::to_string_pretty(store)?;
    content.push('\n');
    write_if_changed(&path, content.as_bytes(), true)
}

/// Atomic write (temp file in the same dir + rename). Keeps a single `.bak` of the previous version.
pub fn write_if_changed(path: &Path, content: &[u8], backup: bool) -> AppResult<bool> {
    if let Ok(existing) = fs::read(path) {
        if existing == content {
            return Ok(false);
        }
        if backup {
            fs::write(path.with_extension("json.bak"), &existing)?;
        }
    }
    let dir = path
        .parent()
        .ok_or_else(|| AppError::new("bad_path", path.display().to_string()))?;
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(content)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| AppError::from(e.error))?;
    Ok(true)
}

pub fn validate(store: &Store) -> AppResult<()> {
    let defaults = store
        .accounts
        .iter()
        .filter(|a| a.config_dir.is_none())
        .count();
    if defaults > 1 {
        return Err(AppError::new(
            "multiple_default",
            "Un seul compte peut utiliser le profil par défaut (~/.claude).",
        ));
    }
    let mut ids: Vec<&str> = store.accounts.iter().map(|a| a.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != store.accounts.len() {
        return Err(AppError::new(
            "duplicate_id",
            "Identifiants de compte en double.",
        ));
    }
    Ok(())
}

/// Pure upsert. New accounts get an isolated profile dir unless they claim the default profile.
pub fn upsert(
    store: &mut Store,
    input: AccountInput,
    profiles_dir: &Path,
    now: &str,
) -> AppResult<Account> {
    if input.label.trim().is_empty() {
        return Err(AppError::new(
            "label_required",
            "Le libellé est obligatoire.",
        ));
    }
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let existing = store.accounts.iter().position(|a| a.id == id);
    let config_dir = if input.is_default_profile {
        None
    } else {
        match existing.map(|i| store.accounts[i].config_dir.clone()) {
            Some(Some(dir)) => Some(dir),
            _ => Some(profiles_dir.join(&id).join("cli").display().to_string()),
        }
    };
    let desktop_dir = match existing.map(|i| store.accounts[i].desktop_user_data_dir.clone()) {
        Some(d) => d,
        None if input.is_default_profile => None,
        None => Some(profiles_dir.join(&id).join("desktop").display().to_string()),
    };
    let mut account = Account {
        id: id.clone(),
        label: input.label.trim().to_string(),
        email: input.email.trim().to_string(),
        plan: input.plan,
        price: input.price,
        renewal_date: input.renewal_date.filter(|d| !d.is_empty()),
        color: input.color,
        config_dir,
        desktop_user_data_dir: desktop_dir,
        five_hour_token_cap: input.five_hour_token_cap,
        weekly_token_cap: input.weekly_token_cap,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };
    match existing {
        Some(i) => {
            let prev = &store.accounts[i];
            account.created_at = prev.created_at.clone();
            account.updated_at = prev.updated_at.clone();
            if *prev != account {
                account.updated_at = now.to_string();
            }
            store.accounts[i] = account.clone();
        }
        None => store.accounts.push(account.clone()),
    }
    validate(store)?;
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Plan;

    #[test]
    fn imports_redirected_copy_once_without_junctions() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst, shared) = (
            tmp.path().join("pkg"),
            tmp.path().join("real"),
            tmp.path().join("shared"),
        );
        fs::create_dir_all(src.join("profiles/a/cli")).unwrap();
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("big.md"), "x").unwrap();
        junction::create(&shared, src.join("profiles/a/cli/skills")).unwrap();
        fs::write(store_path(&src), "{}").unwrap();
        fs::write(src.join("profiles/a/cli/.credentials.json"), "{}").unwrap();

        assert_eq!(
            import_from(&dst, std::slice::from_ref(&src)).unwrap(),
            Some(src.clone())
        );
        assert!(dst.join("profiles/a/cli/.credentials.json").exists());
        assert!(!dst.join("profiles/a/cli/skills").exists());
        fs::write(store_path(&dst), "{\"kept\":1}").unwrap();
        assert_eq!(import_from(&dst, &[src]).unwrap(), None);
        assert_eq!(
            fs::read_to_string(store_path(&dst)).unwrap(),
            "{\"kept\":1}"
        );
    }

    fn input(label: &str, default: bool) -> AccountInput {
        AccountInput {
            id: None,
            label: label.into(),
            email: "a@b.c".into(),
            plan: Plan::Pro,
            price: None,
            renewal_date: None,
            color: "#0A84FF".into(),
            is_default_profile: default,
            five_hour_token_cap: None,
            weekly_token_cap: None,
        }
    }

    #[test]
    fn second_identical_save_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::default();
        upsert(&mut s, input("Perso", true), dir.path(), "t0").unwrap();
        assert!(save(dir.path(), &s).unwrap());
        assert!(!save(dir.path(), &s).unwrap());
        assert_eq!(load(dir.path()).unwrap(), s);
    }

    #[test]
    fn identical_upsert_keeps_updated_at() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::default();
        let a = upsert(&mut s, input("Perso", false), dir.path(), "t0").unwrap();
        let mut again = input("Perso", false);
        again.id = Some(a.id.clone());
        let b = upsert(&mut s, again, dir.path(), "t1").unwrap();
        assert_eq!(b.updated_at, "t0");
        assert_eq!(a.config_dir, b.config_dir);
    }

    #[test]
    fn only_one_default_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::default();
        upsert(&mut s, input("A", true), dir.path(), "t").unwrap();
        let err = upsert(&mut s, input("B", true), dir.path(), "t").unwrap_err();
        assert_eq!(err.code, "multiple_default");
    }

    #[test]
    fn corrupt_store_is_actionable_and_backup_kept() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::default();
        upsert(&mut s, input("A", true), dir.path(), "t").unwrap();
        save(dir.path(), &s).unwrap();
        upsert(&mut s, input("B", false), dir.path(), "t").unwrap();
        save(dir.path(), &s).unwrap();
        fs::write(store_path(dir.path()), "{oops").unwrap();
        assert_eq!(load(dir.path()).unwrap_err().code, "store_corrupt");
        assert!(store_path(dir.path()).with_extension("json.bak").exists());
    }
}
