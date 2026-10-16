#![allow(dead_code)]

#[path = "../src/platform/single_instance.rs"]
mod single_instance;

use std::{process::Command, time::Duration};

use single_instance::{SingleInstance, take_activation_request};

fn child(directory: &std::path::Path, role: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "subprocess_helper", "--nocapture"])
        .env("QCCLIP_INSTANCE_TEST_DIRECTORY", directory)
        .env("QCCLIP_INSTANCE_TEST_ROLE", role);
    command
}

#[test]
fn duplicate_processes_are_blocked_and_request_activation() {
    let directory = tempfile::tempdir().unwrap();
    let instance = SingleInstance::acquire_at(directory.path(), Duration::ZERO)
        .unwrap()
        .unwrap();
    let mut children: Vec<_> = (0..4)
        .map(|_| child(directory.path(), "duplicate").spawn().unwrap())
        .collect();
    for process in &mut children {
        assert!(process.wait().unwrap().success());
    }
    assert!(take_activation_request(&instance.activation_path()).unwrap());
    assert!(!take_activation_request(&instance.activation_path()).unwrap());
}

#[test]
fn normal_exit_releases_lock_even_when_lock_file_remains() {
    let directory = tempfile::tempdir().unwrap();
    let instance = SingleInstance::acquire_at(directory.path(), Duration::ZERO)
        .unwrap()
        .unwrap();
    drop(instance);
    assert!(directory.path().join("single-instance.lock").exists());
    assert!(
        SingleInstance::acquire_at(directory.path(), Duration::ZERO)
            .unwrap()
            .is_some()
    );
}

#[test]
fn abrupt_process_exit_releases_lock() {
    let directory = tempfile::tempdir().unwrap();
    assert!(
        child(directory.path(), "abrupt-exit")
            .status()
            .unwrap()
            .success()
    );
    assert!(
        SingleInstance::acquire_at(directory.path(), Duration::ZERO)
            .unwrap()
            .is_some()
    );
}

#[test]
fn elevated_relaunch_waits_for_the_old_instance() {
    let directory = tempfile::tempdir().unwrap();
    let instance = SingleInstance::acquire_at(directory.path(), Duration::ZERO)
        .unwrap()
        .unwrap();
    let mut process = child(directory.path(), "wait").spawn().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !directory.path().join("waiting").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(process.try_wait().unwrap().is_none());
    assert!(!take_activation_request(&instance.activation_path()).unwrap());
    drop(instance);
    assert!(process.wait().unwrap().success());
}

#[test]
fn invalid_lock_directory_fails_instead_of_allowing_another_instance() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file");
    std::fs::write(&path, "占位文件").unwrap();
    assert!(SingleInstance::acquire_at(&path, Duration::ZERO).is_err());
}

#[test]
fn subprocess_helper() {
    let Some(directory) = std::env::var_os("QCCLIP_INSTANCE_TEST_DIRECTORY") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    match std::env::var("QCCLIP_INSTANCE_TEST_ROLE").unwrap().as_str() {
        "duplicate" => {
            assert!(
                SingleInstance::acquire_at(&directory, Duration::ZERO)
                    .unwrap()
                    .is_none()
            );
        }
        "abrupt-exit" => {
            let _instance = SingleInstance::acquire_at(&directory, Duration::ZERO)
                .unwrap()
                .unwrap();
            std::process::exit(0);
        }
        "wait" => {
            std::fs::write(directory.join("waiting"), []).unwrap();
            let _instance = SingleInstance::acquire_at(&directory, Duration::from_secs(5))
                .unwrap()
                .unwrap();
        }
        _ => panic!("未知单实例测试模式"),
    }
}
