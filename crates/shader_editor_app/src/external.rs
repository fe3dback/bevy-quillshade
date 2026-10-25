//! Opening files in the user's default application ("Show generated WESL/Rust", spec §9).
//!
//! The child never inherits the editor's stdio: xdg-open 1.2 ignores `Terminal=true` in desktop entries and
//! simply execs the handler, so a terminal editor (vim, helix — the usual handler of `text/x-csrc`, which is
//! what `.wesl` files are sniffed as) would take over the terminal the editor was started from. Such handlers
//! are run in a new terminal window instead.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Opens `path` in the background; the error (if any) is returned through `report`.
pub fn open(path: PathBuf, report: impl FnOnce(String) + Send + 'static) {
    std::thread::spawn(move || {
        let result = command(&path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        match result {
            Ok(s) if s.success() => {}
            Ok(s) => report(format!("cannot open {}: opener exited with {s}", path.display())),
            Err(e) => report(format!("cannot open {}: {e}", path.display())),
        }
    });
}

#[cfg(target_os = "macos")]
fn command(path: &Path) -> Command {
    let mut c = Command::new("open");
    c.arg(path);
    c
}

#[cfg(target_os = "windows")]
fn command(path: &Path) -> Command {
    let mut c = Command::new("cmd");
    c.args(["/C", "start", ""]).arg(path);
    c
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn command(path: &Path) -> Command {
    use std::os::unix::process::CommandExt;

    let mut xdg_open = Command::new("xdg-open");
    xdg_open.arg(path);
    let mut c = match handler_needs_terminal(path).then(terminal).flatten() {
        Some((term, args)) => {
            let mut c = Command::new(term);
            c.args(args).arg("xdg-open").arg(path);
            c
        }
        None => xdg_open,
    };
    // Own process group: terminal signals (Ctrl+C in the editor's shell) don't reach it.
    c.process_group(0);
    c
}

/// Whether the desktop entry xdg-mime picks for `path` has `Terminal=true`.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn handler_needs_terminal(path: &Path) -> bool {
    let query = |args: &[&std::ffi::OsStr]| {
        let out = Command::new("xdg-mime").args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        let text = String::from_utf8(out.stdout).ok()?;
        let line = text.lines().next()?.split(';').next()?.trim().to_string();
        (out.status.success() && !line.is_empty()).then_some(line)
    };
    let Some(mime) = query(&["query".as_ref(), "filetype".as_ref(), path.as_os_str()]) else { return false };
    let Some(entry) = query(&["query".as_ref(), "default".as_ref(), mime.as_ref()]) else { return false };
    data_dirs()
        .into_iter()
        .map(|d| d.join("applications").join(&entry))
        .find_map(|f| std::fs::read_to_string(f).ok())
        .is_some_and(|text| desktop_entry_terminal(&text))
}

/// `$XDG_DATA_HOME` then `$XDG_DATA_DIRS`, as the desktop entry spec looks them up.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter().chain(system.split(':').filter(|s| !s.is_empty()).map(PathBuf::from)).collect()
}

/// `Terminal=true` in the `[Desktop Entry]` group.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn desktop_entry_terminal(text: &str) -> bool {
    let mut in_entry = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry
            && let Some(v) = line.strip_prefix("Terminal")
            && let Some(v) = v.trim_start().strip_prefix('=')
        {
            return v.trim() == "true";
        }
    }
    false
}

/// A terminal emulator and the arguments that make it run a command: `$TERMINAL` first, then known ones.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn terminal() -> Option<(String, Vec<&'static str>)> {
    if let Ok(t) = std::env::var("TERMINAL")
        && !t.is_empty()
        && in_path(&t)
    {
        return Some((t, vec!["-e"]));
    }
    const KNOWN: &[(&str, &[&str])] = &[
        ("xdg-terminal-exec", &[]),
        ("kitty", &[]),
        ("foot", &[]),
        ("alacritty", &["-e"]),
        ("wezterm", &["start", "--"]),
        ("ghostty", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("kgx", &["--"]),
        ("konsole", &["-e"]),
        ("xfce4-terminal", &["-x"]),
        ("x-terminal-emulator", &["-e"]),
        ("xterm", &["-e"]),
    ];
    KNOWN.iter().find(|(t, _)| in_path(t)).map(|(t, a)| (t.to_string(), a.to_vec()))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn in_path(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&paths).any(|d| d.join(program).is_file())
}

#[cfg(all(test, not(any(target_os = "macos", target_os = "windows"))))]
mod tests {
    use super::*;

    #[test]
    fn terminal_key_of_desktop_entry() {
        let helix = "[Desktop Entry]\nName=Helix\nExec=hx %F\nTerminal = true\n";
        assert!(desktop_entry_terminal(helix));
        assert!(!desktop_entry_terminal("[Desktop Entry]\nExec=gnome-text-editor %U\nTerminal=false\n"));
        assert!(!desktop_entry_terminal("[Desktop Entry]\nExec=code %F\n"));
        // Keys of other groups (actions) don't count.
        assert!(!desktop_entry_terminal("[Desktop Entry]\nExec=x\n[Desktop Action new]\nTerminal=true\n"));
    }
}
