use std::{
    borrow::{Borrow, BorrowMut},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use atomic_write_file::AtomicWriteFile;
use gpui::{App, AppContext, BorrowAppContext, Global};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

use crate::windows::main::tabs::{SymbolCategoryId, TabId};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum PersistedQuickPasteTab {
    #[default]
    Clipboard,
    Favorites,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum GallerySortField {
    Name,
    Type,
    ModifiedDate,
    CreatedDate,
}

impl Default for GallerySortField {
    fn default() -> Self {
        Self::Name
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum GallerySortOrder {
    Ascending,
    Descending,
}

impl Default for GallerySortOrder {
    fn default() -> Self {
        Self::Ascending
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub(crate) struct PersistedGallerySort {
    pub(crate) field: GallerySortField,
    pub(crate) order: GallerySortOrder,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum PersistedEdgeSide {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PersistedEdgePosition {
    /// 所属显示器的稳定标识。
    pub(crate) display_key: u64,
    /// 贴边方向。
    pub(crate) side: PersistedEdgeSide,
    /// 窗口沿贴边方向的逻辑像素偏移。
    pub(crate) offset: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PersistedWindowSize {
    /// 主窗口客户区的逻辑宽度。
    pub(crate) width: f32,
    /// 主窗口客户区的逻辑高度。
    pub(crate) height: f32,
}

impl PersistedWindowSize {
    pub(crate) const DEFAULT: Self = Self {
        width: 360.0,
        height: 520.0,
    };
    pub(crate) const MIN_SIZE: f32 = 150.0;

    pub(crate) fn is_valid(self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && self.width >= Self::MIN_SIZE
            && self.height >= Self::MIN_SIZE
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub(crate) struct PersistedMainWindowState {
    pub(crate) active_tab: TabId,
    pub(crate) symbol_category: SymbolCategoryId,
    pub(crate) recent_emojis: Vec<String>,
    pub(crate) recent_symbols: Vec<String>,
    /// 上次使用的搜索引擎地址模板，用于右键菜单父项显示与直接点击。
    pub(crate) last_search_engine_template: Option<String>,
    /// 普通窗口的记忆位置；窗口处于贴边状态时不代表当前窗口位置。
    pub(crate) normal_position: Option<PersistedWindowPosition>,
    /// 普通窗口和贴边窗口共用的客户区逻辑尺寸。
    pub(crate) size: Option<PersistedWindowSize>,
    /// 当前保存的贴边状态；为空表示上次退出时没有贴边。
    pub(crate) edge: Option<PersistedEdgePosition>,
    pub(crate) gallery_sort: PersistedGallerySort,
    #[serde(default)]
    pub(crate) quick_paste_tab: PersistedQuickPasteTab,
    #[serde(default)]
    pub(crate) quick_paste_index: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub(crate) struct PersistedState {
    pub(crate) main_window: PersistedMainWindowState,
    pub(crate) legacy_migration_prompt_dismissed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PersistedWindowPosition {
    /// 相对所属显示器左上角的逻辑坐标，不是 Win32 物理设备像素。
    pub(crate) x: f32,
    pub(crate) y: f32,
    /// 根据显示器设备名生成的稳定标识；缺失时兼容旧的全局逻辑坐标。
    pub(crate) display_key: Option<u64>,
}

impl Default for PersistedMainWindowState {
    fn default() -> Self {
        Self {
            active_tab: TabId::Clipboard,
            symbol_category: SymbolCategoryId::Emoji,
            recent_emojis: Vec::new(),
            recent_symbols: Vec::new(),
            last_search_engine_template: None,
            normal_position: None,
            size: None,
            edge: None,
            gallery_sort: PersistedGallerySort::default(),
            quick_paste_tab: PersistedQuickPasteTab::default(),
            quick_paste_index: 0,
        }
    }
}

impl PersistedMainWindowState {
    pub(crate) fn restored_size(&self) -> PersistedWindowSize {
        self.size
            .filter(|size| size.is_valid())
            .unwrap_or(PersistedWindowSize::DEFAULT)
    }
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            main_window: PersistedMainWindowState::default(),
            legacy_migration_prompt_dismissed: false,
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum StateError {
    #[error("无法创建状态目录：{source}")]
    CreateDirectory { source: std::io::Error },
    #[error("无法打开状态文件：{source}")]
    Open { source: std::io::Error },
    #[error("无法写入状态文件：{source}")]
    Write { source: std::io::Error },
    #[error("无法提交状态文件：{source}")]
    Commit { source: std::io::Error },
    #[error("无法序列化状态：{source}")]
    Serialize { source: serde_json::Error },
    #[error("数据正在导入，请稍后重试")]
    DataSwitching,
}

pub(crate) struct StateRuntime {
    state: PersistedState,
    path: PathBuf,
}

impl Global for StateRuntime {}

impl StateRuntime {
    pub(crate) fn install(path: impl Into<PathBuf>, cx: &mut App) {
        let path = path.into();
        let state = load_state(&path);
        if !path.exists() {
            if let Err(error) = persist_state(&path, &state) {
                warn!("创建状态文件失败：{error}");
            }
        }
        cx.set_global(Self { state, path });
    }

    pub(crate) fn reload(path: impl Into<PathBuf>, cx: &mut App) {
        let path = path.into();
        let state = load_state(&path);
        cx.update_global::<Self, _>(|runtime, _| {
            runtime.state = state.clone();
            runtime.path = path.clone();
        });
    }
}

pub(crate) fn state(cx: &impl AppContext) -> PersistedState {
    cx.read_global::<StateRuntime, _>(|runtime, _| runtime.state.clone())
}

pub(crate) fn set_state<C>(state: PersistedState, cx: &mut C) -> Result<bool, StateError>
where
    C: BorrowMut<App>,
{
    let app: &App = Borrow::borrow(&*cx);
    let (current, path) =
        app.read_global::<StateRuntime, _>(|runtime, _| (runtime.state.clone(), runtime.path.clone()));
    if current == state {
        return Ok(false);
    }

    persist_state(&path, &state)?;
    cx.update_global::<StateRuntime, _>(|runtime, _| runtime.state = state);
    Ok(true)
}

/// 记录上次使用的搜索引擎地址模板，用于右键菜单父项显示与直接点击。
pub(crate) fn set_last_search_engine_template(template: Option<String>, cx: &mut App) {
    let mut state = state(cx);
    if state.main_window.last_search_engine_template == template {
        return;
    }
    state.main_window.last_search_engine_template = template;
    if let Err(error) = set_state(state, cx) {
        tracing::error!("保存上次搜索引擎失败：{error}");
    }
}

pub(crate) fn set_main_window_normal_position(
    position: PersistedWindowPosition,
    cx: &mut App,
) {
    let mut state = state(cx);
    if state.main_window.normal_position == Some(position) {
        return;
    }
    state.main_window.normal_position = Some(position);
    if let Err(error) = set_state(state, cx) {
        tracing::error!("保存主窗口位置失败：{error}");
    }
}

pub(crate) fn set_main_window_size<C>(size: PersistedWindowSize, cx: &mut C)
where
    C: BorrowMut<App>,
{
    if !size.is_valid() {
        return;
    }
    let app: &App = Borrow::borrow(&*cx);
    let mut state = state(app);
    if state.main_window.size == Some(size) {
        return;
    }
    state.main_window.size = Some(size);
    if let Err(error) = set_state(state, cx) {
        tracing::error!("保存主窗口大小失败：{error}");
    }
}

pub(crate) fn set_main_window_edge(
    position: Option<PersistedEdgePosition>,
    cx: &mut App,
) {
    let mut state = state(cx);
    if state.main_window.edge == position {
        return;
    }
    state.main_window.edge = position;
    if let Err(error) = set_state(state, cx) {
        tracing::error!("保存主窗口贴边位置失败：{error}");
    }
}

fn load_state(path: &Path) -> PersistedState {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return PersistedState::default();
        }
        Err(error) => {
            warn!("读取状态失败，已使用默认状态：{error}");
            return PersistedState::default();
        }
    };

    match serde_json::from_str(&content) {
        Ok(state) => state,
        Err(error) => {
            warn!("状态格式无效，已使用默认状态：{error}");
            PersistedState::default()
        }
    }
}

fn persist_state(path: &Path, state: &PersistedState) -> Result<(), StateError> {
    let _operation = crate::services::data_runtime::begin_operation()
        .map_err(|_| StateError::DataSwitching)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| StateError::CreateDirectory { source })?;
    }

    let mut content =
        serde_json::to_string_pretty(&state).map_err(|source| StateError::Serialize { source })?;
    content.push('\n');
    let mut file = AtomicWriteFile::open(path).map_err(|source| StateError::Open { source })?;
    file.write_all(content.as_bytes())
        .map_err(|source| StateError::Write { source })?;
    file.commit()
        .map_err(|source| StateError::Commit { source })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        PersistedGallerySort, PersistedMainWindowState, PersistedState, PersistedWindowPosition,
        PersistedWindowSize, persist_state,
    };
    use crate::windows::main::tabs::{SymbolCategoryId, TabId};
    use tempfile::tempdir;

    #[test]
    fn restored_window_size_rejects_collapsed_and_non_finite_sizes() {
        for size in [
            PersistedWindowSize { width: 360.0, height: 1.0 },
            PersistedWindowSize { width: 360.0, height: 149.0 },
            PersistedWindowSize { width: f32::NAN, height: 520.0 },
            PersistedWindowSize { width: 360.0, height: f32::INFINITY },
        ] {
            let state = PersistedMainWindowState { size: Some(size), ..Default::default() };
            assert_eq!(state.restored_size(), PersistedWindowSize::DEFAULT);
        }
        let size = PersistedWindowSize { width: 480.0, height: 640.0 };
        let state = PersistedMainWindowState { size: Some(size), ..Default::default() };
        assert_eq!(state.restored_size(), size);
    }

    #[test]
    fn persisted_state_can_be_loaded_by_json() {
        let directory = tempdir().expect("无法创建临时目录");
        let path = directory.path().join("state.json");
        let state = PersistedState {
            main_window: PersistedMainWindowState {
                active_tab: TabId::Emoji,
                symbol_category: SymbolCategoryId::Gallery,
                recent_emojis: vec!["😀".to_owned(), "🥰".to_owned()],
                recent_symbols: vec!["★".to_owned(), "→".to_owned()],
                last_search_engine_template: None,
                normal_position: Some(PersistedWindowPosition {
                    x: 120.0,
                    y: 240.0,
                    display_key: None,
                }),
                size: Some(PersistedWindowSize {
                    width: 480.0,
                    height: 640.0,
                }),
                edge: None,
                gallery_sort: PersistedGallerySort::default(),
                quick_paste_tab: Default::default(),
                quick_paste_index: 0,
            },
            ..Default::default()
        };

        persist_state(&path, &state).expect("状态保存失败");
        let loaded: PersistedState =
            serde_json::from_str(&std::fs::read_to_string(path).expect("无法读取状态文件"))
                .expect("状态读取失败");
        assert_eq!(loaded, state);
    }

    #[test]
    fn persisted_state_is_grouped_by_window() {
        let state: PersistedState = serde_json::from_str(
            r#"{
                "main_window": {
                    "active_tab": "Clipboard",
                    "symbol_category": "Emoji",
                    "last_search_engine_template": null,
                    "normal_position": null,
                    "size": null,
                    "edge": null
                }
            }"#,
        )
        .expect("状态读取失败");

        assert_eq!(state.main_window.size, None);
        assert_eq!(state.main_window.edge, None);
        assert!(state.main_window.recent_emojis.is_empty());
        assert!(state.main_window.recent_symbols.is_empty());
    }
}
