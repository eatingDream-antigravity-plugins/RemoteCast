use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "chrome-stream-install-test-{tag}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn target_triple() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        other => panic!("unsupported test target: {other:?}"),
    }
}

fn setup_fake_release_fixture(root: &Path, version: &str) -> (PathBuf, PathBuf, PathBuf) {
    let stage = root.join("fixture_stage");
    fs::create_dir_all(stage.join("bin")).unwrap();
    fs::create_dir_all(stage.join("panel/dist")).unwrap();
    fs::create_dir_all(stage.join("skills/chrome-stream")).unwrap();
    fs::create_dir_all(stage.join("assets")).unwrap();
    fs::create_dir_all(stage.join("docs")).unwrap();

    fs::write(stage.join("bin/chrome-stream-mcp"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(
        stage.join("bin/chrome-stream-mcp"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::write(stage.join("panel/dist/panel.html"), "<html></html>\n").unwrap();
    fs::write(stage.join("plugin.json"), "{}\n").unwrap();
    fs::write(stage.join("mcp_config.json"), "{}\n").unwrap();
    fs::write(stage.join("skills/chrome-stream/SKILL.md"), "# skill\n").unwrap();
    fs::write(stage.join("assets/logo.svg"), "<svg/>\n").unwrap();
    fs::write(stage.join("docs/ai-guide.md"), "# guide\n").unwrap();
    fs::write(stage.join("llms.txt"), "llms\n").unwrap();
    fs::write(stage.join("LICENSE"), "MIT\n").unwrap();
    fs::write(stage.join("README.md"), "# README\n").unwrap();

    let archive = root.join(format!(
        "chrome-stream-mcp-{version}-{}.tar.gz",
        target_triple()
    ));
    let tar_status = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&stage)
        .arg(".")
        .status()
        .unwrap();
    assert!(tar_status.success());

    let sha_out = if Command::new("shasum").arg("--version").output().is_ok() {
        Command::new("shasum")
            .args(["-a", "256"])
            .arg(&archive)
            .output()
            .unwrap()
    } else {
        Command::new("sha256sum").arg(&archive).output().unwrap()
    };
    assert!(sha_out.status.success());
    let sha_line = String::from_utf8(sha_out.stdout).unwrap();
    let sha_file = root.join("archive.sha256");
    fs::write(&sha_file, sha_line).unwrap();

    let mock_bin = root.join("mock_bin");
    fs::create_dir_all(&mock_bin).unwrap();
    let curl_log = root.join("curl.log");
    let mock_curl = mock_bin.join("curl");
    fs::write(
        &mock_curl,
        format!(
            r#"#!/bin/sh
set -eu
out=""
prev=""
is_stdin=0
for arg in "$@"; do
  if [ "$prev" = "-o" ]; then
    out="$arg"
  fi
  if [ "$prev" = "-K" ] && [ "$arg" = "-" ]; then
    is_stdin=1
  fi
  prev="$arg"
done
case "$*" in
  *RefreshMcpServers*)
    printf 'ARGV:%s\n' "$*" >> "{log}"
    if [ "$is_stdin" -eq 1 ]; then
      printf 'STDIN:' >> "{log}"
      cat >> "{log}"
    fi
    exit 0
    ;;
esac
if [ -n "$out" ]; then
  case "$out" in
    *.sha256) cp "{sha}" "$out" ;;
    *.tar.gz) cp "{arc}" "$out" ;;
  esac
fi
"#,
            log = curl_log.display(),
            sha = sha_file.display(),
            arc = archive.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&mock_curl, fs::Permissions::from_mode(0o755)).unwrap();

    (mock_bin, curl_log, archive)
}

#[test]
fn install_script_uses_curl_config_stdin_and_guards_ls_addr_version_and_symlinks() {
    let root = temp_dir("guard");
    let script = repo_root().join("scripts/install.sh");
    let version = "v0.6.1";
    let (mock_bin, curl_log, _archive) = setup_fake_release_fixture(&root, version);
    let orig_path = std::env::var("PATH").unwrap_or_default();
    let test_path = format!("{}:{orig_path}", mock_bin.display());
    let dest = root.join("plugins/chrome-stream-mcp");
    let csrf_val = "fixture-header-val";

    // 1. Valid loopback LS_ADDRESS passes CSRF token via `curl -K -` stdin and NEVER in argv
    let ok_out = Command::new("sh")
        .arg(&script)
        .args(["--version", version, "--dir"])
        .arg(&dest)
        .env("PATH", &test_path)
        .env("ANTIGRAVITY_CSRF_TOKEN", csrf_val)
        .env("ANTIGRAVITY_LS_ADDRESS", "127.0.0.1:5387")
        .output()
        .unwrap();
    assert!(
        ok_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&ok_out.stderr)
    );
    let log_body = fs::read_to_string(&curl_log).unwrap();
    assert!(log_body.contains("--connect-timeout 2 --max-time 5 -K -"));
    assert!(!log_body.contains(&format!("-H x-codeium-csrf-token: {csrf_val}")));
    assert!(!log_body
        .lines()
        .any(|l| l.starts_with("ARGV:") && l.contains(csrf_val)));
    assert!(log_body.contains(&format!("header = \"x-codeium-csrf-token: {csrf_val}\"")));

    // 2. Non-loopback / tampered / newline-injected ANTIGRAVITY_LS_ADDRESS or CSRF must NOT invoke curl RefreshMcpServers
    fs::write(&curl_log, "").unwrap();
    for bad_addr in [
        "evil.example:5387",
        "127.0.0.1:5387@192.0.2.1",
        "localhost:5387/extra",
        "127.0.0.1:99999",
        "127.0.0.1:5387\nurl = \"https://evil.example\"",
    ] {
        let out = Command::new("sh")
            .arg(&script)
            .args(["--version", version, "--dir"])
            .arg(&dest)
            .env("PATH", &test_path)
            .env("ANTIGRAVITY_CSRF_TOKEN", csrf_val)
            .env("ANTIGRAVITY_LS_ADDRESS", bad_addr)
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    let bad_csrf_out = Command::new("sh")
        .arg(&script)
        .args(["--version", version, "--dir"])
        .arg(&dest)
        .env("PATH", &test_path)
        .env(
            "ANTIGRAVITY_CSRF_TOKEN",
            "bad\nurl = \"https://evil.example\"",
        )
        .env("ANTIGRAVITY_LS_ADDRESS", "127.0.0.1:5387")
        .output()
        .unwrap();
    assert!(bad_csrf_out.status.success());
    let bad_log = fs::read_to_string(&curl_log).unwrap_or_default();
    assert!(
        !bad_log.contains("RefreshMcpServers"),
        "RefreshMcpServers must not be called for non-loopback LS_ADDRESS or unsafe CSRF: {bad_log}"
    );

    // 3. Invalid VERSION tag is rejected
    let bad_ver = Command::new("sh")
        .arg(&script)
        .args(["--version", "v1.0.0/../../evil", "--dir"])
        .arg(&dest)
        .env("PATH", &test_path)
        .output()
        .unwrap();
    assert!(!bad_ver.status.success());

    // 4. Symlink target inside install dir is rejected before copying
    let secret_target = root.join("outside_secret.txt");
    fs::write(&secret_target, "keep-me").unwrap();
    fs::remove_file(dest.join("README.md")).unwrap();
    symlink(&secret_target, dest.join("README.md")).unwrap();
    let sym_out = Command::new("sh")
        .arg(&script)
        .args(["--version", version, "--dir"])
        .arg(&dest)
        .env("PATH", &test_path)
        .env("ANTIGRAVITY_CSRF_TOKEN", "")
        .output()
        .unwrap();
    assert!(!sym_out.status.success());
    assert_eq!(fs::read_to_string(&secret_target).unwrap(), "keep-me");

    // 5. --uninstall refuses canonicalized $HOME/. and $PLUGINS_DIR/
    let fake_home = root.join("home");
    let fake_plugins = fake_home.join(".gemini/config/plugins");
    fs::create_dir_all(&fake_plugins).unwrap();
    for unsafe_dir in [
        format!("{}/.", fake_home.display()),
        format!("{}//", fake_plugins.display()),
    ] {
        let u_out = Command::new("sh")
            .arg(&script)
            .args(["--uninstall", "--dir", &unsafe_dir])
            .env("HOME", &fake_home)
            .env("ANTIGRAVITY_PLUGINS_DIR", &fake_plugins)
            .output()
            .unwrap();
        assert!(!u_out.status.success());
    }

    let _ = fs::remove_dir_all(&root);
}
