#![cfg_attr(all(target_os = "windows", qcclip_release_gui), windows_subsystem = "windows")]

mod app;
mod app_paths;
mod assets;
mod config;
mod config_runtime;
mod domain;
mod emoji;
mod hotkey;
pub mod i18n;
mod logging;
mod platform;
mod services;
mod state_runtime;
mod symbols;
mod ui;
mod window_material;
mod windows;

fn main() {
    app::run();
}
