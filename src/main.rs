mod cdp;
mod config;
mod i18n;
mod mcp;
mod relay;
mod stream;

use crate::cdp::discovery::{discover_port_file, probe_debug_port_ws_url};
use crate::config::{Config, APP_VERSION};
use crate::i18n::{current_lang, Lang};
use crate::relay::ipc::{probe_relay_ping, RelayBackend, RelayClient};
use serde_json::json;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::ExitCode;
use std::time::Duration;

fn print_usage() {
    match current_lang() {
        Lang::En => println!(
            "Chrome Stream MCP {APP_VERSION} — Native Rust Chrome DevTools screencast relay & MCP App server\n\
             \n\
             USAGE:\n\
               chrome-stream-mcp serve            Start the MCP stdio JSON-RPC server\n\
               chrome-stream-mcp relay            Start the persistent 0600 Unix-socket CDP relay daemon\n\
               chrome-stream-mcp doctor [--json]  Inspect Chrome DevToolsActivePort, port 9222, and relay socket\n\
               chrome-stream-mcp --version        Print version\n\
               chrome-stream-mcp --help           Show this help message"
        ),
        Lang::ZhCn => println!(
            "Chrome Stream MCP {APP_VERSION} — 本机 Chrome 远程调试投屏中继与 MCP App 服务\n\
             \n\
             用法:\n\
               chrome-stream-mcp serve            启动 MCP stdio JSON-RPC 服务\n\
               chrome-stream-mcp relay            启动持久化 0600 Unix 套接字中继守护进程\n\
               chrome-stream-mcp doctor [--json]  检查 Chrome 调试端口、9222 回退端口与中继状态\n\
               chrome-stream-mcp --version        打印版本号\n\
               chrome-stream-mcp --help           显示此帮助信息"
        ),
    }
}

fn run_doctor(cfg: &Config, as_json: bool) -> ExitCode {
    let lang = current_lang();
    let port_file = discover_port_file(&cfg.port_files);
    let debug_port_ws = if port_file.valid {
        None
    } else {
        probe_debug_port_ws_url(cfg.debug_port)
    };

    let sock_exists = cfg.relay_sock.exists();
    let sock_mode = fs::metadata(&cfg.relay_sock)
        .ok()
        .map(|m| format!("{:04o}", m.permissions().mode() & 0o777));
    let relay_ping = if sock_exists {
        probe_relay_ping(&cfg.relay_sock)
    } else {
        None
    };
    let relay_status = if relay_ping.is_some() {
        RelayClient::connect_only(cfg.relay_sock.clone())
            .call("status", json!({}), Duration::from_secs(2))
            .ok()
    } else {
        None
    };

    let panel_meta = fs::metadata(&cfg.panel_html).ok();
    let report = json!({
        "version": APP_VERSION,
        "locale": lang.as_str(),
        "chrome": {
            "portFile": port_file,
            "candidatesChecked": cfg.port_files.len(),
            "fallbackDebugPort": cfg.debug_port,
            "fallbackWsUrl": debug_port_ws,
            "reachable": port_file.valid || debug_port_ws.is_some(),
        },
        "relay": {
            "socketPath": cfg.relay_sock.to_string_lossy(),
            "socketExists": sock_exists,
            "socketMode": sock_mode,
            "alive": relay_ping.is_some(),
            "ping": relay_ping,
            "status": relay_status,
        },
        "panel": {
            "path": cfg.panel_html.to_string_lossy(),
            "exists": panel_meta.is_some(),
            "bytes": panel_meta.as_ref().map(|m| m.len()).unwrap_or(0),
        }
    });

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into())
        );
    } else {
        let chrome_ok = report["chrome"]["reachable"].as_bool().unwrap_or(false);
        let relay_ok = report["relay"]["alive"].as_bool().unwrap_or(false);
        let panel_ok = report["panel"]["exists"].as_bool().unwrap_or(false);
        let panel_kib = report["panel"]["bytes"].as_u64().unwrap_or(0) as f64 / 1024.0;
        match lang {
            Lang::En => {
                println!("Chrome Stream MCP Doctor (v{APP_VERSION})");
                println!(
                    "  Chrome CDP : {} (DevToolsActivePort valid={}, stale={}, port={:?}, fallback :{}={})",
                    if chrome_ok { "READY" } else { "NOT FOUND" },
                    port_file.valid,
                    port_file.stale,
                    port_file.port,
                    cfg.debug_port,
                    debug_port_ws.is_some()
                );
                println!(
                    "  Relay Sock : {} ({}, mode={})",
                    if relay_ok { "ALIVE" } else { "STOPPED" },
                    cfg.relay_sock.display(),
                    sock_mode.as_deref().unwrap_or("-")
                );
                if let Some(st) = &relay_status {
                    println!(
                        "  CDP Session: connected={}, activeTargetId={}",
                        st["connected"].as_bool().unwrap_or(false),
                        st["activeTargetId"].as_str().unwrap_or("-")
                    );
                }
                println!(
                    "  Panel HTML : {} ({}, {:.1} KiB)",
                    if panel_ok { "READY" } else { "MISSING" },
                    cfg.panel_html.display(),
                    panel_kib
                );
            }
            Lang::ZhCn => {
                println!("Chrome Stream MCP 诊断报告 (v{APP_VERSION})");
                println!(
                    "  Chrome 调试: {} (DevToolsActivePort 有效={}, 过期={}, 端口={:?}, 备用 :{}={})",
                    if chrome_ok { "已就绪" } else { "未检测到" },
                    port_file.valid,
                    port_file.stale,
                    port_file.port,
                    cfg.debug_port,
                    debug_port_ws.is_some()
                );
                println!(
                    "  本地中继   : {} ({}, 权限={})",
                    if relay_ok { "运行中" } else { "未运行" },
                    cfg.relay_sock.display(),
                    sock_mode.as_deref().unwrap_or("-")
                );
                if let Some(st) = &relay_status {
                    println!(
                        "  调试会话   : 已连接={}, 当前标签页={}",
                        st["connected"].as_bool().unwrap_or(false),
                        st["activeTargetId"].as_str().unwrap_or("-")
                    );
                }
                println!(
                    "  面板文件   : {} ({}, {:.1} KiB)",
                    if panel_ok { "已就绪" } else { "缺失" },
                    cfg.panel_html.display(),
                    panel_kib
                );
            }
        }
    }

    ExitCode::SUCCESS
}

fn apply_cli_overrides(cfg: &mut Config, args: &[String]) {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" if i + 1 < args.len() => {
                let d = std::path::PathBuf::from(&args[i + 1]);
                if !d.as_os_str().is_empty() && !args[i + 1].contains("${") {
                    cfg.data_dir = d.clone();
                    cfg.relay_sock = d.join("relay.sock");
                    cfg.relay_pid_file = d.join("relay.pid");
                }
                i += 2;
            }
            "--panel-html" if i + 1 < args.len() => {
                let p = std::path::PathBuf::from(&args[i + 1]);
                if !p.as_os_str().is_empty() && !args[i + 1].contains("${") {
                    cfg.panel_html = p;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut cfg = Config::load();
    if args.len() > 1 {
        apply_cli_overrides(&mut cfg, &args[1..]);
    }

    match args.first().map(String::as_str) {
        Some("serve") => match mcp::server::run_stdio_server(&cfg) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("[chrome-stream-mcp serve] fatal: {err}");
                ExitCode::FAILURE
            }
        },
        Some("relay") => match relay::daemon::run_relay_daemon(&cfg) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("[chrome-stream-mcp relay] fatal: {err}");
                ExitCode::FAILURE
            }
        },
        Some("doctor") => {
            let as_json = args.iter().any(|a| a == "--json");
            run_doctor(&cfg, as_json)
        }
        Some("--version") | Some("-V") => {
            println!("chrome-stream-mcp {APP_VERSION}");
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") | None => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("Unknown command: {other}\n");
            print_usage();
            ExitCode::FAILURE
        }
    }
}
