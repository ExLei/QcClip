mod about;
mod application_filter;
mod clipboard;
mod data_management;
mod general;
mod legacy_migration;
mod personalization;
mod tab_bar_editor;
mod quick_paste;
mod shortcuts;
mod sound;

pub(in crate::windows::settings) use personalization::persist_appearance_change_in_view;
pub(in crate::windows::settings) use sound::persist_sound_change_in_view;
pub(in crate::windows::settings) use legacy_migration::start_source_analysis;
