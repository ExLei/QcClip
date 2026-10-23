#![allow(dead_code, unused_imports)]

// 独立验证更新客户端和配置，避免依赖界面及数据库测试。
#[path = "../src/services/updates/client.rs"]
mod client;
#[path = "../src/config/mod.rs"]
mod config;

#[path = "../src/services/updates/download.rs"]
mod download;
#[path = "../src/services/updates/installer.rs"]
mod installer;
#[path = "../src/platform/single_instance.rs"]
pub(crate) mod single_instance;

mod platform {
    pub(crate) use crate::single_instance;
}
