//! Runs the real `kvs` binary against a temporary store directory.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use tempfile::TempDir;

fn kvs(dir: &TempDir, args: &[&str]) -> assert_cmd::assert::Assert {
    cargo_bin_cmd!("kvs")
        .arg("--dir")
        .arg(dir.path())
        .args(args)
        .assert()
}

#[test]
fn set_then_get_across_runs() {
    let dir = TempDir::new().unwrap();
    kvs(&dir, &["set", "name", "zaid"]).success().stdout("");
    kvs(&dir, &["get", "name"]).success().stdout("zaid\n");
}

#[test]
fn overwrite_keeps_the_latest() {
    let dir = TempDir::new().unwrap();
    kvs(&dir, &["set", "lang", "c++"]).success();
    kvs(&dir, &["set", "lang", "rust"]).success();
    kvs(&dir, &["get", "lang"]).success().stdout("rust\n");
}

#[test]
fn missing_key_is_reported_with_exit_code_1() {
    let dir = TempDir::new().unwrap();
    kvs(&dir, &["get", "nope"])
        .code(1)
        .stdout("Key not found\n");
    kvs(&dir, &["rm", "nope"]).code(1).stdout("Key not found\n");
}

#[test]
fn rm_deletes() {
    let dir = TempDir::new().unwrap();
    kvs(&dir, &["set", "tmp", "x"]).success();
    kvs(&dir, &["rm", "tmp"]).success().stdout("");
    kvs(&dir, &["get", "tmp"]).code(1);
}

#[test]
fn stats_show_keys_and_stale_bytes() {
    let dir = TempDir::new().unwrap();
    kvs(&dir, &["set", "a", "1"]).success();
    kvs(&dir, &["set", "a", "2"]).success();
    kvs(&dir, &["set", "b", "3"]).success();
    kvs(&dir, &["stats"])
        .success()
        .stdout(predicate::str::contains("keys: 2").and(predicate::str::contains("stale bytes: ")));
}

#[test]
fn corrupt_log_is_an_error_with_exit_code_2() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("kvs.log"),
        "not json\n{\"op\":\"rm\",\"key\":\"a\"}\n",
    )
    .unwrap();
    kvs(&dir, &["get", "a"])
        .code(2)
        .stderr(predicate::str::contains("corrupt log"));
}

#[test]
fn missing_arguments_show_usage() {
    let dir = TempDir::new().unwrap();
    kvs(&dir, &["set", "only-key"])
        .failure()
        .stderr(predicate::str::contains("Usage"));
}
