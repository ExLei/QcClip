#[path = "../src/services/data_runtime.rs"]
mod data_runtime;

use std::{sync::mpsc, time::Duration};

#[test]
fn exclusive_operation_allows_owner_nesting_and_releases_after_failure() {
    data_runtime::initialize();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        data_runtime::with_exclusive_operation(|| {
            let _nested = data_runtime::begin_operation().expect("独占线程应允许嵌套存储调用");
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        data_runtime::begin_operation().is_err(),
        "其他线程的数据操作必须暂停"
    );
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    assert!(
        data_runtime::begin_operation().is_ok(),
        "完成后应恢复数据操作"
    );

    let result = std::panic::catch_unwind(|| {
        data_runtime::with_exclusive_operation(|| {
            let _nested = data_runtime::begin_operation().unwrap();
            panic!("模拟独占操作中断");
        });
    });
    assert!(result.is_err());
    assert!(
        data_runtime::begin_operation().is_ok(),
        "中断后应释放独占锁及嵌套状态"
    );
    data_runtime::with_exclusive_operation(|| assert!(data_runtime::begin_operation().is_ok()));
}
