use crate::cdp::discovery::{probe_ipv4_debug_port_ws_url, validate_loopback_ws_url};
use crate::local_io::{create_private_directory, effective_uid, owned_directory};
#[cfg(not(test))]
use std::env;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
#[cfg(not(test))]
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(8);
const PROBE_INTERVAL: Duration = Duration::from_millis(100);
const MANAGED_ENDPOINT_FILE: &str = ".chrome-stream-endpoint";
const REMOTE_PORT_PROBE_SPAN: u16 = 10;

enum DebugPort {
    Available,
    Browser(String),
    Occupied,
}

fn probe_port(port: u16) -> io::Result<DebugPort> {
    if let Some(url) = probe_ipv4_debug_port_ws_url(port) {
        return Ok(DebugPort::Browser(url));
    }
    match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
        Ok(_) => Ok(DebugPort::Available),
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => Ok(DebugPort::Occupied),
        Err(err) => Err(err),
    }
}

fn pick_ephemeral_loopback_port() -> io::Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

/// Discovers a live managed Remote Chrome endpoint recorded under `<data_dir>/remote-profile`.
/// Never spawns Chrome and never attaches to an unverified foreign browser.
pub fn discover_managed_remote_ws_url(data_dir: &Path) -> Option<(u16, String)> {
    let data_path = std::path::absolute(data_dir).ok()?;
    let profile = data_path.join("remote-profile");
    let _profile_dir = owned_directory(&profile, false).ok()?;
    for (port, expected_path) in read_managed_endpoints(&profile) {
        if let Some(url) = probe_ipv4_debug_port_ws_url(port) {
            if browser_path(port, &url).as_deref() == Some(expected_path.as_str()) {
                return Some((port, url));
            }
        }
    }
    None
}

pub fn launch_remote(
    data_dir: &Path,
    personal_port: u16,
    explicit_remote_port: Option<u16>,
    cancelled: impl Fn() -> bool,
) -> Result<String, String> {
    launch_with(
        data_dir,
        personal_port,
        explicit_remote_port,
        probe_port,
        pick_ephemeral_loopback_port,
        spawn_chrome,
        cancelled,
    )
}

fn launch_with(
    data_dir: &Path,
    personal_port: u16,
    explicit_remote_port: Option<u16>,
    probe: impl Fn(u16) -> io::Result<DebugPort>,
    pick_ephemeral: impl Fn() -> io::Result<u16>,
    spawn: impl FnOnce(&[OsString]) -> Result<Receiver<String>, String>,
    cancelled: impl Fn() -> bool,
) -> Result<String, String> {
    if let Some(0) = explicit_remote_port {
        return Err("Remote Chrome requires a nonzero loopback port".into());
    }
    let cancelled_error = "CDP connection cancelled by disconnect";
    let data_path = std::path::absolute(data_dir)
        .map_err(|err| format!("Invalid Remote Chrome data directory: {err}"))?;
    let profile = data_path.join("remote-profile");
    let _profile_dir = create_private_directory(&data_path)
        .and_then(|_| create_private_directory(&profile))
        .map_err(|err| format!("Could not prepare the isolated Remote Chrome profile: {err}"))?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(profile.join(".chrome-stream-launch.lock"))
        .map_err(|err| format!("Could not open the Remote Chrome launch lock: {err}"))?;
    if !lock
        .metadata()
        .is_ok_and(|meta| meta.is_file() && meta.uid() == effective_uid())
    {
        return Err("Unsafe Remote Chrome launch lock".into());
    }
    // SAFETY: lock owns the descriptor. The lock is released on drop.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("Could not acquire the Remote Chrome launch lock".into());
    }
    lock.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|err| format!("Could not secure the Remote Chrome launch lock: {err}"))?;

    if cancelled() {
        return Err(cancelled_error.into());
    }

    let port = match explicit_remote_port {
        Some(pinned) => {
            match probe(pinned)
                .map_err(|err| format!("Could not check loopback debug port {pinned}: {err}"))?
            {
                DebugPort::Browser(url) if managed_endpoint_matches(&profile, pinned, &url) => {
                    return Ok(url);
                }
                DebugPort::Browser(_) | DebugPort::Occupied => {
                    return Err(pinned_port_occupied_error(pinned));
                }
                DebugPort::Available => pinned,
            }
        }
        None => {
            // 1. Reuse an already-running managed Remote Chrome on its recorded port.
            for (recorded_port, expected_path) in read_managed_endpoints(&profile) {
                if cancelled() {
                    return Err(cancelled_error.into());
                }
                if let Ok(DebugPort::Browser(url)) = probe(recorded_port) {
                    if browser_path(recorded_port, &url).as_deref() == Some(expected_path.as_str())
                    {
                        return Ok(url);
                    }
                }
            }

            // 2. Select an unoccupied loopback port starting from personal_port + 1 (e.g. 9223..=9232),
            //    never colliding with Personal Chrome on 9222.
            let base = if personal_port > 0 {
                personal_port
            } else {
                9222
            };
            let mut selected = None;
            for offset in 1..=REMOTE_PORT_PROBE_SPAN {
                let Some(candidate) = base.checked_add(offset) else {
                    break;
                };
                if cancelled() {
                    return Err(cancelled_error.into());
                }
                match probe(candidate).map_err(|err| {
                    format!("Could not check loopback debug port {candidate}: {err}")
                })? {
                    DebugPort::Available => {
                        selected = Some(candidate);
                        break;
                    }
                    DebugPort::Browser(url)
                        if managed_endpoint_matches(&profile, candidate, &url) =>
                    {
                        return Ok(url);
                    }
                    DebugPort::Browser(_) | DebugPort::Occupied => {}
                }
            }

            // 3. Fall back to an OS-assigned free loopback port if all 10 candidate ports are busy.
            match selected {
                Some(port) => port,
                None => {
                    let mut port = pick_ephemeral().map_err(|err| {
                        format!("Could not allocate a free loopback port for Remote Chrome: {err}")
                    })?;
                    if port == personal_port {
                        port = pick_ephemeral().map_err(|err| {
                            format!(
                                "Could not allocate a free loopback port for Remote Chrome: {err}"
                            )
                        })?;
                    }
                    if port == 0 || port == personal_port {
                        return Err(
                            "Could not find an unoccupied loopback port for Remote Chrome.".into(),
                        );
                    }
                    port
                }
            }
        }
    };

    let occupied = || {
        if explicit_remote_port.is_some() {
            pinned_port_occupied_error(port)
        } else {
            format!("Debug port {port} became occupied while starting Remote Chrome.")
        }
    };

    // A listener that races the port check cannot supply the startup announcement
    // from our Chrome child. Accept it only if both browser endpoints agree.
    if cancelled() {
        return Err(cancelled_error.into());
    }
    let startup = spawn(&chrome_args(&profile, port))?;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut announced: Option<String> = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = PROBE_INTERVAL.min(remaining);
        if let Some(url) = &announced {
            if matches!(probe(port), Ok(DebugPort::Browser(live)) if live == *url) {
                let path = browser_path(port, url).ok_or_else(occupied)?;
                record_managed_endpoint(&profile, port, &path)
                    .map_err(|err| format!("Could not record the Remote Chrome endpoint: {err}"))?;
                return if cancelled() {
                    Err(cancelled_error.into())
                } else {
                    Ok(url.clone())
                };
            }
            thread::sleep(wait);
        } else {
            match startup.recv_timeout(wait) {
                Ok(url) if browser_path(port, &url).is_some() => announced = Some(url),
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }
    if cancelled() {
        return Err(cancelled_error.into());
    }
    if matches!(probe(port), Ok(DebugPort::Browser(_) | DebugPort::Occupied)) {
        return Err(occupied());
    }
    Err(format!(
        "Remote Chrome did not become ready on 127.0.0.1:{port}"
    ))
}

fn pinned_port_occupied_error(port: u16) -> String {
    format!("Debug port {port} is already in use by another browser, profile, or service.")
}

fn browser_path(port: u16, url: &str) -> Option<String> {
    let (host, reported_port, path) = validate_loopback_ws_url(url).ok()?;
    (host == "127.0.0.1"
        && reported_port == port
        && path
            .strip_prefix("/devtools/browser/")
            .is_some_and(|id| !id.is_empty()))
    .then_some(path)
}

fn read_managed_endpoints(profile: &Path) -> Vec<(u16, String)> {
    let mut endpoints = Vec::with_capacity(2);
    for name in [MANAGED_ENDPOINT_FILE, "DevToolsActivePort"] {
        let Ok(file) = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(profile.join(name))
        else {
            continue;
        };
        if !file.metadata().is_ok_and(|meta| {
            meta.is_file() && meta.uid() == effective_uid() && meta.mode() & 0o022 == 0
        }) {
            continue;
        }
        let mut text = String::new();
        if file.take(4097).read_to_string(&mut text).is_err() || text.len() > 4096 {
            continue;
        }
        let mut lines = text.lines();
        let Some(port) = lines
            .next()
            .and_then(|line| line.trim().parse::<u16>().ok())
            .filter(|&port| port > 0)
        else {
            continue;
        };
        let Some(path) = lines.next().map(str::trim) else {
            continue;
        };
        let candidate_url = format!("ws://127.0.0.1:{port}{path}");
        if let Some(valid_path) = browser_path(port, &candidate_url) {
            let entry = (port, valid_path);
            if !endpoints.contains(&entry) {
                endpoints.push(entry);
            }
        }
    }
    endpoints
}

fn managed_endpoint_matches(profile: &Path, port: u16, url: &str) -> bool {
    let Some(path) = browser_path(port, url) else {
        return false;
    };
    read_managed_endpoints(profile)
        .into_iter()
        .any(|(recorded_port, recorded_path)| recorded_port == port && recorded_path == path)
}

fn record_managed_endpoint(profile: &Path, port: u16, path: &str) -> io::Result<()> {
    // Serialize writers with the profile lock and atomically replace the marker.
    let temporary = profile.join(".chrome-stream-endpoint.tmp");
    match fs::remove_file(&temporary) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    let mut marker = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    writeln!(marker, "{port}\n{path}")?;
    fs::rename(temporary, profile.join(MANAGED_ENDPOINT_FILE))
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
        "chrome://newtab".into(),
    ]
}

#[cfg(not(test))]
fn chrome_candidates() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    return &[
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
        "/Applications/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ];
    #[cfg(not(target_os = "macos"))]
    return &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
    ];
}

#[cfg(test)]
fn spawn_chrome(_args: &[OsString]) -> Result<Receiver<String>, String> {
    Err("Starting Chrome is disabled in test builds".into())
}

#[cfg(not(test))]
fn spawn_chrome(args: &[OsString]) -> Result<Receiver<String>, String> {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::CommandExt;

    let candidates = match env::var_os("CHROME_STREAM_CHROME_BIN").filter(|bin| !bin.is_empty()) {
        Some(bin) => vec![bin],
        None => chrome_candidates().iter().map(OsString::from).collect(),
    };
    let mut last_error = None;
    for bin in candidates {
        let mut command = Command::new(bin);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        // SAFETY: setsid is async-signal-safe and runs in the child before exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        match command.spawn() {
            Ok(mut child) => {
                let stderr = child.stderr.take().expect("Chrome stderr is piped");
                let (tx, rx) = mpsc::sync_channel(1);
                thread::spawn(move || {
                    let mut reader = BufReader::new(stderr);
                    let mut line = Vec::new();
                    let mut reported = false;
                    loop {
                        match crate::local_io::read_bounded_line(&mut reader, &mut line, 4096) {
                            Ok(0) => break,
                            Ok(_) if !reported => {
                                if let Some(url) =
                                    std::str::from_utf8(&line).ok().and_then(|text| {
                                        text.trim().strip_prefix("DevTools listening on ")
                                    })
                                {
                                    let _ = tx.try_send(url.to_string());
                                    reported = true;
                                }
                            }
                            Ok(_) => {}
                            Err(err) if err.kind() == io::ErrorKind::InvalidData => {
                                if line.last() != Some(&b'\n') && reader.skip_until(b'\n').is_err()
                                {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let _ = child.wait();
                });
                return Ok(rx);
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
    use std::os::unix::fs::symlink;

    #[test]
    fn chrome_spawn_is_disabled_in_test_builds() {
        assert!(spawn_chrome(&[])
            .unwrap_err()
            .contains("disabled in test builds"));
    }

    #[test]
    fn rejects_unsafe_profile_and_data_directories() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("csm-launcher-dirs-{}", std::process::id()));
        let unsafe_parent = root.join("shared");
        fs::create_dir_all(&unsafe_parent).unwrap();
        fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o777)).unwrap();
        let no_spawn = |_: &[OsString]| panic!("unsafe directory must prevent launch");
        assert!(launch_with(
            &unsafe_parent.join("data"),
            9222,
            None,
            |_| Ok(DebugPort::Available),
            || Ok(9300),
            no_spawn,
            || false
        )
        .is_err());
        assert!(!unsafe_parent.join("data").exists());
        symlink(&unsafe_parent, root.join("remote-profile")).unwrap();
        assert!(launch_with(
            &root,
            9222,
            None,
            |_| Ok(DebugPort::Available),
            || Ok(9300),
            no_spawn,
            || false
        )
        .is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_foreign_browser_and_reuses_only_the_matching_managed_endpoint() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("csm-launcher-reuse-{}", std::process::id()));
        let url = "ws://127.0.0.1:9225/devtools/browser/current";
        let launch = || {
            launch_with(
                &root,
                9222,
                Some(9225),
                |_| Ok(DebugPort::Browser(url.into())),
                || panic!("pinned port must not fall back to ephemeral"),
                |_| panic!("port is occupied"),
                || false,
            )
        };
        assert!(launch()
            .unwrap_err()
            .contains("Debug port 9225 is already in use"));
        let marker = root.join("remote-profile/DevToolsActivePort");
        fs::write(&marker, "9225\n/devtools/browser/previous\n").unwrap();
        assert!(launch().is_err());
        fs::write(&marker, "9225\n/devtools/browser/current\n").unwrap();
        assert_eq!(launch().unwrap(), url);
        assert_eq!(
            fs::metadata(marker.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn launch_verifies_the_child_announcement_and_records_it_for_reuse() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("csm-launcher-start-{}", std::process::id()));
        let personal_port = 9222;
        let expected_port = 9223;
        let url = format!("ws://127.0.0.1:{expected_port}/devtools/browser/started");
        let spawned = std::cell::Cell::new(false);
        let probe = |p| {
            Ok(if p == expected_port && spawned.get() {
                DebugPort::Browser(url.clone())
            } else if p == personal_port {
                DebugPort::Occupied
            } else {
                DebugPort::Available
            })
        };
        let result = launch_with(
            &root,
            personal_port,
            None,
            probe,
            || panic!("default candidate port should be selected before ephemeral"),
            |args| {
                assert_eq!(
                    args,
                    chrome_args(&root.join("remote-profile"), expected_port)
                );
                let (tx, rx) = mpsc::channel();
                tx.send(url.clone()).unwrap();
                spawned.set(true);
                Ok(rx)
            },
            || false,
        );
        assert_eq!(result.unwrap(), url);
        assert_eq!(
            launch_with(
                &root,
                personal_port,
                None,
                probe,
                || panic!("verified browser is live"),
                |_| panic!("verified browser is live"),
                || false
            )
            .unwrap(),
            url
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn auto_port_skips_occupied_personal_port_instead_of_failing() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("csm-launcher-auto-red-{}", std::process::id()));
        let personal_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let personal_port = personal_listener.local_addr().unwrap().port();
        let spawned_port = std::cell::Cell::new(0u16);
        let probe = |p| {
            if spawned_port.get() == p && p != 0 {
                Ok(DebugPort::Browser(format!(
                    "ws://127.0.0.1:{p}/devtools/browser/remote-auto"
                )))
            } else {
                probe_port(p)
            }
        };
        let result = launch_with(
            &root,
            personal_port,
            None,
            probe,
            pick_ephemeral_loopback_port,
            |args| {
                let port = args
                    .iter()
                    .find_map(|a| a.to_str()?.strip_prefix("--remote-debugging-port="))
                    .unwrap()
                    .parse::<u16>()
                    .unwrap();
                assert_ne!(
                    port, personal_port,
                    "Remote Chrome must pick an unoccupied port instead of Personal Chrome's port"
                );
                spawned_port.set(port);
                let (tx, rx) = mpsc::channel();
                tx.send(format!(
                    "ws://127.0.0.1:{port}/devtools/browser/remote-auto"
                ))
                .unwrap();
                Ok(rx)
            },
            || false,
        );
        let url = result.expect("launch_remote must not fail when personal port is occupied");
        let chosen = spawned_port.get();
        assert_ne!(chosen, personal_port);
        assert!(url.contains(&format!(":{chosen}/devtools/browser/remote-auto")));

        // Second call must reuse the live managed Remote Chrome without spawning again.
        let reused = launch_with(
            &root,
            personal_port,
            None,
            probe,
            || panic!("live managed Remote Chrome must be reused"),
            |_| panic!("live managed Remote Chrome must not spawn a second window"),
            || false,
        )
        .unwrap();
        assert_eq!(reused, url);

        drop(personal_listener);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_port_skips_busy_range_and_falls_back_to_ephemeral_port_and_reuses_it() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("csm-launcher-ephemeral-{}", std::process::id()));
        let ephemeral_port = 49_321u16;
        let url = format!("ws://127.0.0.1:{ephemeral_port}/devtools/browser/ephemeral-id");
        let spawned = std::cell::Cell::new(false);
        let probe = |p| {
            if p == ephemeral_port && spawned.get() {
                Ok(DebugPort::Browser(url.clone()))
            } else if (9222..=9232).contains(&p) {
                Ok(DebugPort::Occupied)
            } else {
                Ok(DebugPort::Available)
            }
        };
        let launched = launch_with(
            &root,
            9222,
            None,
            probe,
            || Ok(ephemeral_port),
            |args| {
                assert_eq!(
                    args,
                    chrome_args(&root.join("remote-profile"), ephemeral_port)
                );
                let (tx, rx) = mpsc::channel();
                tx.send(url.clone()).unwrap();
                spawned.set(true);
                Ok(rx)
            },
            || false,
        )
        .unwrap();
        assert_eq!(launched, url);

        // Even though the recorded port is outside 9223..=9232, reuse finds it via .chrome-stream-endpoint!
        let reused = launch_with(
            &root,
            9222,
            None,
            probe,
            || panic!("must reuse recorded ephemeral Remote Chrome"),
            |_| panic!("must not spawn when recorded ephemeral Remote Chrome is alive"),
            || false,
        )
        .unwrap();
        assert_eq!(reused, url);

        let _ = fs::remove_dir_all(root);
    }
}
