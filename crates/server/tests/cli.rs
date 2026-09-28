//! The `doris` binary as an operator meets it: startup failures read as one
//! plain sentence on stderr and exit non-zero.

use std::net::TcpListener;
use std::process::Command;

fn doris(env: &[(&str, &str)]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_doris"))
        .envs(env.iter().copied())
        .output()
        .expect("binary runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_busy_port_is_reported_in_plain_words() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap().to_string();
    let dir = tempfile::tempdir().unwrap();
    let database = format!("sqlite://{}", dir.path().join("doris.db").display());

    let (ok, stderr) = doris(&[("DORIS_LISTEN", &addr), ("DORIS_DATABASE", &database)]);

    assert!(!ok);
    assert!(
        stderr.contains(&format!("doris: cannot listen on {addr}: ")),
        "{stderr}"
    );
    assert!(!stderr.contains("Os {"), "{stderr}");
}

#[test]
fn an_unopenable_database_is_reported_in_plain_words() {
    let (ok, stderr) = doris(&[
        ("DORIS_DATABASE", "sqlite:///nonexistent-dir/doris.db"),
        ("DORIS_LISTEN", "127.0.0.1:0"),
    ]);

    assert!(!ok);
    assert!(
        stderr.contains("doris: cannot open database sqlite:///nonexistent-dir/doris.db: "),
        "{stderr}"
    );
    assert!(!stderr.contains("SqliteError {"), "{stderr}");
}
