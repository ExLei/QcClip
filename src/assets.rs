use gpui::{AssetSource, SharedString};
use rust_embed::RustEmbed;
use std::borrow::Cow;

#[derive(RustEmbed)]
#[folder = "assets"]
struct EmbeddedAssets;

pub struct Assets;

impl Assets {
    pub fn new() -> Self {
        Self
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let Some(asset_path) = path.strip_prefix("assets/") else {
            // 非内嵌 assets/ 路径按本地文件系统读取（如数据目录里的搜索引擎图标）。
            return match std::fs::read(path) {
                Ok(bytes) => Ok(Some(Cow::Owned(bytes))),
                Err(_) => Ok(None),
            };
        };

        Ok(EmbeddedAssets::get(asset_path).map(|file| file.data))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}
