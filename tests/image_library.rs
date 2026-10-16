#![allow(dead_code)]

#[path = "../src/services/data_runtime.rs"]
pub mod data_runtime;

mod services {
    pub use crate::data_runtime;
}

#[path = "../src/services/image_library.rs"]
mod image_library;

use std::fs;

use image_library::ImageLibraryService;

#[test]
fn delete_group_removes_images_and_icon_and_preserves_other_groups() {
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let icon = source.path().join("icon.png");
    image::RgbaImage::new(2, 2).save(&icon).unwrap();
    let service = ImageLibraryService::new(root.path());
    let group = service.create_group("待删除", Some(&icon)).unwrap();
    let other = service.create_group("保留", None).unwrap();
    fs::write(root.path().join(&group.name).join("image.png"), b"image").unwrap();
    fs::write(root.path().join(&other.name).join("other.png"), b"retained").unwrap();
    let icon_path = service.group_icon_path(&group).unwrap();

    service.delete_group(&group).unwrap();

    let groups = service.load_groups().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, other.id);
    assert!(!root.path().join(&group.name).exists());
    assert!(!icon_path.exists());
    assert!(root.path().join(&other.name).join("other.png").exists());
    assert!(icon.exists(), "图库外的源文件应保留");
    assert!(!fs::read_dir(root.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".deleted-group-")
    }));
}

#[test]
fn delete_last_group_with_missing_directory_leaves_empty_group_list() {
    let root = tempfile::tempdir().unwrap();
    let service = ImageLibraryService::new(root.path());
    let group = service.create_group("最后一组", None).unwrap();
    fs::remove_dir(root.path().join(&group.name)).unwrap();

    service.delete_group(&group).unwrap();

    assert!(service.load_groups().unwrap().is_empty());
    assert!(service.delete_group(&group).is_err());
}

#[test]
fn delete_group_uses_persisted_name_after_rename() {
    let root = tempfile::tempdir().unwrap();
    let service = ImageLibraryService::new(root.path());
    let original = service.create_group("原名", None).unwrap();
    let renamed = service.update_group(&original, "新名", None).unwrap();
    let other = service.create_group("原名", None).unwrap();

    service.delete_group(&original).unwrap();

    assert!(!root.path().join(&renamed.name).exists());
    assert!(root.path().join(&other.name).exists());
    assert_eq!(service.load_groups().unwrap()[0].id, other.id);
}

#[cfg(windows)]
#[test]
fn delete_group_restores_images_when_group_list_cannot_be_saved() {
    use std::os::windows::fs::OpenOptionsExt;

    let root = tempfile::tempdir().unwrap();
    let service = ImageLibraryService::new(root.path());
    let group = service.create_group("保存失败", None).unwrap();
    let image_path = root.path().join(&group.name).join("image.png");
    fs::write(&image_path, b"retained").unwrap();
    let _locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(root.path().join("groups.json"))
        .unwrap();

    assert!(service.delete_group(&group).is_err());

    assert_eq!(fs::read(&image_path).unwrap(), b"retained");
    assert_eq!(service.load_groups().unwrap()[0].id, group.id);
}
