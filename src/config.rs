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
    pub debug_port: u16,
    pub panel_html: PathBuf,
}

impl Config {
    pub fn load() -> Self {
        let home = home_dir();
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

        let port_file = env::var_os("CHROME_STREAM_CHROME_PORT_FILE")
            .or_else(|| env::var_os("CHROME_DEVTOOLS_ACTIVE_PORT"))
            .map(PathBuf::from)
            .unwrap_or_else(|| default_chrome_port_file(&home));

        let debug_port = env::var("CHROME_STREAM_CHROME_DEBUG_PORT")
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
    fn default_port_file_uses_home_dynamically() {
        let p = default_chrome_port_file(Path::new("/tmp/test-home"));
        assert!(p.starts_with("/tmp/test-home"));
        assert!(p.ends_with("DevToolsActivePort"));
    }
}
