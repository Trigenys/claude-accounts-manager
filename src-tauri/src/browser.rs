//! Private-window URL opener. Claude Code runs `$BROWSER <url>` (single executable, no extra args),
//! so secondary profiles point BROWSER at this app: invoked with a lone http(s) URL, it opens the
//! default browser in private mode and exits, keeping `/login` away from the main claude.ai session.

use std::path::Path;
use std::process::Command;

/// Private-mode flag for a browser executable, by file name.
pub fn private_flag(exe: &Path) -> Option<&'static str> {
    let name = exe.file_stem()?.to_string_lossy().to_ascii_lowercase();
    match name.as_str() {
        "msedge" => Some("--inprivate"),
        "chrome" | "brave" | "vivaldi" | "chromium" | "thorium" => Some("--incognito"),
        "opera" => Some("--private"),
        "firefox" | "librewolf" | "waterfox" | "zen" => Some("-private-window"),
        _ => None,
    }
}

/// First token of a shell `open\command` value (`"C:\x\b.exe" --flag "%1"` → `C:\x\b.exe`).
pub fn command_exe(cmd: &str) -> Option<String> {
    let cmd = cmd.trim();
    let exe = match cmd.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => cmd.split_whitespace().next()?,
    };
    (!exe.is_empty()).then(|| exe.to_string())
}

#[cfg(windows)]
fn default_browser_exe() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER};
    let prog_id: String = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(
            r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
        )
        .ok()?
        .get_value("ProgId")
        .ok()?;
    let cmd: String = RegKey::predef(HKEY_CLASSES_ROOT)
        .open_subkey(format!(r"{prog_id}\shell\open\command"))
        .ok()?
        .get_value("")
        .ok()?;
    command_exe(&cmd)
}

#[cfg(not(windows))]
fn default_browser_exe() -> Option<String> {
    None
}

/// Returns the URL when the process was started as a BROWSER handler (exactly one http(s) argument).
pub fn url_from_args(args: &[String]) -> Option<&str> {
    match args {
        [_, url] if url.starts_with("https://") || url.starts_with("http://") => Some(url),
        _ => None,
    }
}

/// Opens `url` in a private window of the default browser, falling back to a normal window
/// when the browser is unknown (the user still gets the page, just not isolated).
pub fn open_private(url: &str) -> std::io::Result<()> {
    if let Some(exe) = default_browser_exe() {
        if let Some(flag) = private_flag(Path::new(&exe)) {
            return Command::new(&exe).arg(flag).arg(url).spawn().map(|_| ());
        }
    }
    Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_by_browser() {
        assert_eq!(
            private_flag(Path::new(
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
            )),
            Some("--inprivate")
        );
        assert_eq!(
            private_flag(Path::new(
                r"C:\Program Files\Google\Chrome\Application\chrome.exe"
            )),
            Some("--incognito")
        );
        assert_eq!(
            private_flag(Path::new(r"C:\Program Files\Mozilla Firefox\firefox.exe")),
            Some("-private-window")
        );
        assert_eq!(private_flag(Path::new(r"C:\x\unknown.exe")), None);
    }

    #[test]
    fn parses_registry_commands() {
        assert_eq!(
            command_exe(
                r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --single-argument %1"#
            )
            .as_deref(),
            Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe")
        );
        assert_eq!(
            command_exe(r"C:\b\firefox.exe -osint -url %1").as_deref(),
            Some(r"C:\b\firefox.exe")
        );
        assert_eq!(command_exe("  "), None);
    }

    #[test]
    fn detects_handler_invocation() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            url_from_args(&a(&["app", "https://claude.ai/oauth?x=1"])),
            Some("https://claude.ai/oauth?x=1")
        );
        assert_eq!(url_from_args(&a(&["app"])), None);
        assert_eq!(url_from_args(&a(&["app", "--flag"])), None);
        assert_eq!(url_from_args(&a(&["app", "https://a", "b"])), None);
    }
}
