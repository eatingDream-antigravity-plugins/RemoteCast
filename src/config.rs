use std::env;
use std::path::{Path, PathBuf};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELAY_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub relay_sock: PathBuf,
    pub relay_pid_file: PathBuf,
    pub port_file: PathBuf,
    pub port_files: Vec<PathBuf>,
    pub debug_port: u16,
    pub panel_html: PathBuf,
}

impl Config {
    pub fn load() -> Self {
        let home = home_dir();
        let xdg = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty());

        let data_dir = env::var_os("CHROME_STREAM_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                home.join(".gemini")
                    .join("antigravity")
                    .join("plugin_data")
                    .join("chrome-stream-mcp")
            });

        let relay_sock = env::var_os("CHROME_STREAM_RELAY_SOCK")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("relay.sock"));

        let relay_pid_file = data_dir.join("relay.pid");

        let explicit_port_file = env::var_os("CHROME_STREAM_PORT_FILE")
            .or_else(|| env::var_os("CHROME_STREAM_CHROME_PORT_FILE"))
            .or_else(|| env::var_os("CHROME_DEVTOOLS_ACTIVE_PORT"))
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty());

        let explicit_user_data_dir = env::var_os("CHROME_STREAM_USER_DATA_DIR")
            .or_else(|| env::var_os("CHROME_USER_DATA_DIR"))
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty());

        let port_files = candidate_chrome_port_files(
            &home,
            xdg.as_deref(),
            explicit_port_file.as_deref(),
            explicit_user_data_dir.as_deref(),
        );
        let port_file = port_files
            .first()
            .cloned()
            .unwrap_or_else(|| default_chrome_port_file(&home));

        let debug_port = env::var("CHROME_STREAM_DEBUG_PORT")
            .or_else(|_| env::var("CHROME_STREAM_CHROME_DEBUG_PORT"))
            .or_else(|_| env::var("CHROME_DEBUG_PORT"))
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|&p| p > 0)
            .unwrap_or(9222);

        let panel_html = env::var_os("CHROME_STREAM_PANEL_HTML")
            .map(PathBuf::from)
            .unwrap_or_else(default_panel_html_path);

        Self {
            data_dir,
            relay_sock,
            relay_pid_file,
            port_file,
            port_files,
            debug_port,
            panel_html,
        }
    }
}

pub fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_chrome_port_file(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Application Support")
            .join("Google")
            .join("Chrome")
            .join("DevToolsActivePort")
    }
    #[cfg(target_os = "linux")]
    {
        home.join(".config")
            .join("google-chrome")
            .join("DevToolsActivePort")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        home.join("DevToolsActivePort")
    }
}

fn push_unique(out: &mut Vec<PathBuf>, p: PathBuf) {
    if !p.as_os_str().is_empty() && !out.iter().any(|existing| existing == &p) {
        out.push(p);
    }
}

/// Builds an ordered list of candidate `DevToolsActivePort` files across explicit overrides,
/// macOS channels (Stable, Beta, Chrome for Testing, Canary, Chromium), and Linux distributions
/// (`$XDG_CONFIG_HOME`, `~/.config`, Ubuntu Snap Chromium, Flatpak Chrome/Chromium).
pub fn candidate_chrome_port_files(
    home: &Path,
    xdg_config_home: Option<&Path>,
    explicit_port_file: Option<&Path>,
    explicit_user_data_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(14);
    if let Some(pf) = explicit_port_file {
        push_unique(&mut out, pf.to_path_buf());
    }
    if let Some(dir) = explicit_user_data_dir {
        push_unique(&mut out, dir.join("DevToolsActivePort"));
    }

    let mac_dirs = [
        home.join("Library/Application Support/Google/Chrome"),
        home.join("Library/Application Support/Google/Chrome Beta"),
        home.join("Library/Application Support/Google/Chrome for Testing"),
        home.join("Library/Application Support/Google/Chrome Canary"),
        home.join("Library/Application Support/Chromium"),
    ];

    let default_cfg = home.join(".config");
    let mut cfg_roots = Vec::with_capacity(2);
    if let Some(xdg) = xdg_config_home {
        cfg_roots.push(xdg.to_path_buf());
    }
    if !cfg_roots.iter().any(|r| r == &default_cfg) {
        cfg_roots.push(default_cfg);
    }

    let mut linux_dirs = Vec::with_capacity(10);
    for root in &cfg_roots {
        for sub in [
            "google-chrome",
            "google-chrome-beta",
            "google-chrome-unstable",
            "chromium",
        ] {
            linux_dirs.push(root.join(sub));
        }
    }
    linux_dirs.push(home.join("snap/chromium/common/chromium"));
    linux_dirs.push(home.join(".var/app/com.google.Chrome/config/google-chrome"));
    linux_dirs.push(home.join(".var/app/org.chromium.Chromium/config/chromium"));

    #[cfg(target_os = "macos")]
    {
        for d in &mac_dirs {
            push_unique(&mut out, d.join("DevToolsActivePort"));
        }
        for d in &linux_dirs {
            push_unique(&mut out, d.join("DevToolsActivePort"));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        for d in &linux_dirs {
            push_unique(&mut out, d.join("DevToolsActivePort"));
        }
        for d in &mac_dirs {
            push_unique(&mut out, d.join("DevToolsActivePort"));
        }
    }

    if out.is_empty() {
        out.push(default_chrome_port_file(home));
    }
    out
}

pub fn default_panel_html_path() -> PathBuf {
    if let Ok(exe) = env::current_exe() {
        // Installed plugin layout: <plugin_root>/bin/chrome-stream-mcp -> <plugin_root>/panel/dist/panel.html
        if let Some(parent) = exe.parent() {
            if let Some(root) = parent.parent() {
                let candidate = root.join("panel").join("dist").join("panel.html");
                if candidate.exists() {
                    return candidate;
                }
            }
            // Cargo target layout: <repo>/target/release/chrome-stream-mcp -> <repo>/panel/dist/panel.html
            if let Some(grandparent) = parent.parent().and_then(|p| p.parent()) {
                let candidate = grandparent.join("panel").join("dist").join("panel.html");
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("panel").join("dist").join("panel.html")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_file_discovery_uses_home_and_covers_all_platform_candidates() {
        let p = default_chrome_port_file(Path::new("/tmp/test-home"));
        assert!(p.starts_with("/tmp/test-home"));
        assert!(p.ends_with("DevToolsActivePort"));

        let home = Path::new("/tmp/fake-home");
        let xdg = Path::new("/tmp/fake-xdg");
        let explicit_pf = Path::new("/tmp/custom/DevToolsActivePort");
        let explicit_udd = Path::new("/tmp/chrome-debug-profile");

        let list =
            candidate_chrome_port_files(home, Some(xdg), Some(explicit_pf), Some(explicit_udd));
        assert_eq!(list[0], explicit_pf);
        assert_eq!(
            list[1],
            PathBuf::from("/tmp/chrome-debug-profile/DevToolsActivePort")
        );
        assert!(list.contains(&PathBuf::from(
            "/tmp/fake-xdg/google-chrome/DevToolsActivePort"
        )));
        assert!(list.contains(&PathBuf::from(
            "/tmp/fake-home/.config/google-chrome/DevToolsActivePort"
        )));
        assert!(list.contains(&PathBuf::from(
            "/tmp/fake-home/.config/chromium/DevToolsActivePort"
        )));
        assert!(list.contains(&PathBuf::from(
            "/tmp/fake-home/snap/chromium/common/chromium/DevToolsActivePort"
        )));
        assert!(list.contains(&PathBuf::from(
            "/tmp/fake-home/.var/app/com.google.Chrome/config/google-chrome/DevToolsActivePort"
        )));
        assert!(list.contains(&PathBuf::from(
            "/tmp/fake-home/Library/Application Support/Google/Chrome Canary/DevToolsActivePort"
        )));
    }
}
