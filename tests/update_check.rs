#![allow(dead_code, unused_imports)]

// 独立验证更新客户端和配置，避免依赖界面及数据库测试。
#[path = "../src/config/mod.rs"]
mod config;
#[path = "../src/services/updates/client.rs"]
mod update_client;
