use crate::cdp::discovery::{probe_ipv4_debug_port_ws_url, validate_loopback_ws_url};
use std::env;
use std::ffi::OsString;
use std::fs::{self, DirBuilder};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(8);
const PROBE_INTERVAL: Duration = Duration::from_millis(100);

pub fn launch_remote(data_dir: &Path, port: u16) -> Result<String, String> {
    launch_with(data_dir, port, probe_ipv4_debug_port_ws_url, spawn_chrome)
}

fn launch_with(
    data_dir: &Path,
    port: u16,
    probe: impl Fn(u16) -> Option<String>,
    spawn: impl FnOnce(&[OsString]) -> Result<(), String>,
) -> Result<String, String> {
    if port == 0 {
        return Err("Remote Chrome requires a nonzero loopback port".into());
    }
    let probe_local = || {
        let url = probe(port)?;
        let (host, reported_port, _) = validate_loopback_ws_url(&url).ok()?;
        (host == "127.0.0.1" && reported_port == port).then_some(url)
    };
    if let Some(url) = probe_local() {
        return Ok(url);
    }

    let profile = create_profile(data_dir)
        .map_err(|err| format!("Could not prepare the isolated Remote Chrome profile: {err}"))?;
    spawn(&chrome_args(&profile, port))?;

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Some(url) = probe_local() {
            return Ok(url);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!(
                "Remote Chrome did not become ready on 127.0.0.1:{port}"
            ));
        }
        thread::sleep(PROBE_INTERVAL.min(remaining));
    }
}

fn create_profile(data_dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(data_dir)?;
    let profile = data_dir.canonicalize()?.join("remote-profile");
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&profile) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err),
    }
    if !fs::symlink_metadata(&profile)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Remote Chrome profile must be a directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(&profile)?;
        directory.set_permissions(fs::Permissions::from_mode(0o700))?;
    }
    Ok(profile)
}

fn chrome_args(profile: &Path, port: u16) -> [OsString; 6] {
    let mut profile_arg = OsString::from("--user-data-dir=");
    profile_arg.push(profile);
    [
        "--remote-debugging-address=127.0.0.1".into(),
        format!("--remote-debugging-port={port}").into(),
        profile_arg,
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "about:blank".into(),
    ]
}

fn chrome_candidates() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    return &[
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ];
    #[cfg(target_os = "windows")]
    return &["chrome.exe"];
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
    ];
}

fn spawn_chrome(args: &[OsString]) -> Result<(), String> {
    let override_bin = env::var_os("CHROME_STREAM_CHROME_BIN").filter(|bin| !bin.is_empty());
    let candidates = if let Some(bin) = override_bin {
        vec![bin]
    } else {
        chrome_candidates().iter().map(OsString::from).collect()
    };
    let mut last_error = None;
    for bin in candidates {
        let mut command = Command::new(bin);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: setsid is async-signal-safe and runs in the child before exec.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            command.creation_flags(DETACHED_PROCESS);
        }
        match command.spawn() {
            Ok(mut child) => {
                thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            Err(err) => last_error = Some(err),
        }
    }
    Err(format!(
        "Could not start a Chrome executable: {}",
        last_error.map_or_else(|| "no executable configured".into(), |err| err.to_string())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn passes_exact_loopback_arguments_and_a_single_profile_argument() {
        let profile = Path::new("/data with spaces/remote-profile");
        assert_eq!(
            chrome_args(profile, 9222),
            [
                "--remote-debugging-address=127.0.0.1",
                "--remote-debugging-port=9222",
                "--user-data-dir=/data with spaces/remote-profile",
                "--no-first-run",
                "--no-default-browser-check",
                "about:blank",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn reuses_existing_ipv4_port_before_preparing_profile_or_spawning() {
        let url = "ws://127.0.0.1:9222/devtools/browser/existing";
        let result = launch_with(
            Path::new(""),
            9222,
            |port| {
                assert_eq!(port, 9222);
                Some(url.into())
            },
            |_| panic!("existing Chrome must be reused"),
        );
        assert_eq!(result.unwrap(), url);
    }

    #[cfg(unix)]
    #[test]
    fn creates_private_profile_and_rejects_foreign_probe_urls() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = env::temp_dir().join(format!("chrome-stream-launcher-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let personal = root.join("personal");
        fs::create_dir(&personal).unwrap();
        let expected = root.canonicalize().unwrap().join("remote-profile");
        symlink(&personal, &expected).unwrap();
        assert!(create_profile(&root).is_err());
        fs::remove_file(&expected).unwrap();

        let probes = Cell::new(0);
        let ready = "ws://127.0.0.1:9222/devtools/browser/ready";
        for rejected in [
            "ws://192.0.2.1:9222/devtools/browser/foreign",
            "ws://127.0.0.1:9223/devtools/browser/foreign",
            "ws://[::1]:9222/devtools/browser/foreign",
        ] {
            probes.set(0);
            let result = launch_with(
                &root,
                9222,
                |_| {
                    probes.set(probes.get() + 1);
                    Some(if probes.get() == 1 { rejected } else { ready }.into())
                },
                |args| {
                    assert_eq!(args, chrome_args(&expected, 9222));
                    assert_eq!(
                        fs::metadata(&expected).unwrap().permissions().mode() & 0o777,
                        0o700
                    );
                    Ok(())
                },
            );
            assert_eq!(result.unwrap(), ready);
            assert_eq!(probes.get(), 2);
        }
        assert!(launch_with(&root, 0, |_| panic!(), |_| panic!()).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
