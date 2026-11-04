//! Claude Desktop (MSIX): the default account opens the packaged app; other accounts run the
//! packaged executable with their own Electron `--user-data-dir` (own login + single-instance lock).

use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::model::Account;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopInfo {
    pub exe: String,
    pub aumid: String,
    pub version: String,
}

fn hidden_powershell(script: &str) -> AppResult<String> {
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Re-resolved on every launch: the WindowsApps path changes with each app update.
pub fn locate() -> AppResult<DesktopInfo> {
    let out = hidden_powershell(
        "$p = Get-AppxPackage -Name Claude | Sort-Object Version -Descending | Select-Object -First 1; \
         if ($p) { $id = (Get-AppxPackageManifest $p).Package.Applications.Application | Select-Object -First 1 -ExpandProperty Id; \
         \"$($p.InstallLocation)|$($p.PackageFamilyName)!$id|$($p.Version)\" }",
    )?;
    let mut parts = out.split('|');
    let (Some(loc), Some(aumid), Some(version)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(AppError::new(
            "desktop_not_found",
            "Claude Desktop n'est pas installé (paquet MSIX « Claude » introuvable).",
        ));
    };
    let exe = PathBuf::from(loc).join("app").join("claude.exe");
    if !exe.is_file() {
        return Err(AppError::new(
            "desktop_exe_missing",
            format!("Exécutable introuvable : {}", exe.display()),
        ));
    }
    Ok(DesktopInfo {
        exe: exe.display().to_string(),
        aumid: aumid.to_string(),
        version: version.to_string(),
    })
}

pub fn launch(account: &Account) -> AppResult<()> {
    let info = locate()?;
    let spawned = match &account.desktop_user_data_dir {
        None => Command::new("explorer")
            .arg(format!("shell:AppsFolder\\{}", info.aumid))
            .spawn(),
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            Command::new(&info.exe)
                .arg(format!("--user-data-dir={dir}"))
                .arg(format!("--disk-cache-dir={dir}\\cache"))
                .spawn()
        }
    };
    spawned.map(|_| ()).map_err(|e| {
        AppError::new(
            "launch_failed",
            format!("Impossible de lancer Claude Desktop : {e}"),
        )
    })
}
