use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

#[test]
fn test_offline_execution_with_blackhole_proxies() {
    let proxy_url = "http://127.0.0.1:1";

    // 1. Version text
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.env("HTTP_PROXY", proxy_url)
        .env("HTTPS_PROXY", proxy_url)
        .env("ALL_PROXY", proxy_url)
        .env("http_proxy", proxy_url)
        .env("https_proxy", proxy_url)
        .env("all_proxy", proxy_url)
        .arg("--version")
        .timeout(Duration::from_secs(3))
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::starts_with("qdev 0.1.0"));

    // 2. Version JSON
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .env("HTTP_PROXY", proxy_url)
        .env("HTTPS_PROXY", proxy_url)
        .env("ALL_PROXY", proxy_url)
        .env("http_proxy", proxy_url)
        .env("https_proxy", proxy_url)
        .env("all_proxy", proxy_url)
        .args(["--version", "--json"])
        .timeout(Duration::from_secs(3))
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["version"], "0.1.0");

    // 3. Status JSON in non-interactive mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .env("HTTP_PROXY", proxy_url)
        .env("HTTPS_PROXY", proxy_url)
        .env("ALL_PROXY", proxy_url)
        .env("http_proxy", proxy_url)
        .env("https_proxy", proxy_url)
        .env("all_proxy", proxy_url)
        .args(["--non-interactive", "--json"])
        .timeout(Duration::from_secs(3))
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["interactivity"], "non_interactive");

    // 4. Unknown subcommand in JSON mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .env("HTTP_PROXY", proxy_url)
        .env("HTTPS_PROXY", proxy_url)
        .env("ALL_PROXY", proxy_url)
        .env("http_proxy", proxy_url)
        .env("https_proxy", proxy_url)
        .env("all_proxy", proxy_url)
        .args(["unknown-cmd", "--json"])
        .timeout(Duration::from_secs(3))
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_zero_network_connections_attempted() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind test listener");
    listener
        .set_nonblocking(true)
        .expect("Failed to set nonblocking");
    let port = listener.local_addr().unwrap().port();
    let proxy_url = format!("http://127.0.0.1:{}", port);

    let connection_detected = Arc::new(AtomicBool::new(false));
    let connection_detected_clone = connection_detected.clone();

    let stop_probe = Arc::new(AtomicBool::new(false));
    let stop_probe_clone = stop_probe.clone();

    // Run background listener probe
    let listener_thread = std::thread::spawn(move || {
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_millis(500)
            && !stop_probe_clone.load(Ordering::SeqCst)
        {
            if listener.accept().is_ok() {
                connection_detected_clone.store(true, Ordering::SeqCst);
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.env("HTTP_PROXY", &proxy_url)
        .env("HTTPS_PROXY", &proxy_url)
        .env("ALL_PROXY", &proxy_url)
        .env("http_proxy", &proxy_url)
        .env("https_proxy", &proxy_url)
        .env("all_proxy", &proxy_url)
        .args(["--version", "--json"])
        .assert()
        .success()
        .code(0);

    stop_probe.store(true, Ordering::SeqCst);
    let _ = listener_thread.join();
    assert!(
        !connection_detected.load(Ordering::SeqCst),
        "Network connection was initiated to proxy listener!"
    );
}

#[test]
fn test_no_forbidden_network_dependencies_in_workspace() {
    let forbidden_network_crates = ["reqwest", "hyper", "curl", "ureq"];

    let root_cargo_lock = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("Cargo.lock");

    assert!(
        root_cargo_lock.exists(),
        "Cargo.lock must exist at workspace root"
    );

    let lock_content = std::fs::read_to_string(&root_cargo_lock).unwrap();
    for &forbidden in &forbidden_network_crates {
        assert!(
            !lock_content.contains(&format!("name = \"{}\"", forbidden)),
            "Cargo.lock contains forbidden network client dependency '{}' violating NFR-403",
            forbidden
        );
    }
}
