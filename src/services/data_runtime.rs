use std::{
    cell::Cell,
    marker::PhantomData,
    rc::Rc,
    sync::{Condvar, Mutex, OnceLock},
};

struct DataRuntime {
    activity: Mutex<DataActivity>,
    changed: Condvar,
}

struct DataActivity {
    switching: bool,
    active_operations: usize,
}

static DATA_RUNTIME: OnceLock<DataRuntime> = OnceLock::new();

thread_local! {
    static OPERATION_DEPTH: Cell<usize> = const { Cell::new(0) };
}

#[derive(Debug, thiserror::Error)]
#[error("数据正在导入，请稍后重试")]
pub(crate) struct DataRuntimeError;

pub(crate) struct DataOperationGuard {
    runtime: Option<&'static DataRuntime>,
    _not_send: PhantomData<Rc<()>>,
}

pub(crate) struct DataSwitchGuard {
    runtime: Option<&'static DataRuntime>,
}

pub(crate) fn initialize() {
    let _ = DATA_RUNTIME.set(DataRuntime {
        activity: Mutex::new(DataActivity {
            switching: false,
            active_operations: 0,
        }),
        changed: Condvar::new(),
    });
}

pub(crate) fn begin_operation() -> Result<DataOperationGuard, DataRuntimeError> {
    let Some(runtime) = DATA_RUNTIME.get() else {
        return Ok(DataOperationGuard {
            runtime: None,
            _not_send: PhantomData,
        });
    };

    let nested = OPERATION_DEPTH.with(|depth| {
        let current = depth.get();
        if current == 0 {
            false
        } else {
            depth.set(current.saturating_add(1));
            true
        }
    });
    if nested {
        return Ok(DataOperationGuard {
            runtime: None,
            _not_send: PhantomData,
        });
    }

    let mut activity = runtime
        .activity
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if activity.switching {
        return Err(DataRuntimeError);
    }
    activity.active_operations = activity.active_operations.saturating_add(1);
    OPERATION_DEPTH.with(|depth| depth.set(1));
    Ok(DataOperationGuard {
        runtime: Some(runtime),
        _not_send: PhantomData,
    })
}

pub(crate) fn begin_switch() -> DataSwitchGuard {
    let Some(runtime) = DATA_RUNTIME.get() else {
        return DataSwitchGuard { runtime: None };
    };

    let mut activity = runtime
        .activity
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    while activity.switching || activity.active_operations != 0 {
        activity = runtime
            .changed
            .wait(activity)
            .unwrap_or_else(|error| error.into_inner());
    }
    activity.switching = true;
    DataSwitchGuard {
        runtime: Some(runtime),
    }
}

pub(crate) fn with_exclusive_operation<T>(operation: impl FnOnce() -> T) -> T {
    let (result, guard) = with_exclusive_operation_and_guard(operation);
    drop(guard);
    result
}

pub(crate) fn with_exclusive_operation_and_guard<T>(
    operation: impl FnOnce() -> T,
) -> (T, DataSwitchGuard) {
    assert!(
        OPERATION_DEPTH.with(|depth| depth.get() == 0),
        "独占数据操作不能嵌套执行"
    );
    let switch = begin_switch();
    // 仅当前线程允许调用现有存储接口；其他线程保持暂停，避免备份和提交之间发生写入。
    OPERATION_DEPTH.with(|depth| depth.set(1));
    let _owner = DataOperationGuard {
        runtime: None,
        _not_send: PhantomData,
    };
    let result = operation();
    drop(_owner);
    (result, switch)
}

impl Drop for DataOperationGuard {
    fn drop(&mut self) {
        OPERATION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        let Some(runtime) = self.runtime else {
            return;
        };
        let mut activity = runtime
            .activity
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        activity.active_operations = activity.active_operations.saturating_sub(1);
        runtime.changed.notify_all();
    }
}

impl Drop for DataSwitchGuard {
    fn drop(&mut self) {
        let Some(runtime) = self.runtime else {
            return;
        };
        let mut activity = runtime
            .activity
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        activity.switching = false;
        runtime.changed.notify_all();
    }
}
