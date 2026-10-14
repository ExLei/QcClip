use super::*;

impl MainWindow {
    pub(crate) fn set_gallery_sort(
        &mut self,
        sort: crate::state_runtime::PersistedGallerySort,
        cx: &mut Context<Self>,
    ) {
        if self.gallery_sort == sort {
            return;
        }
        self.gallery_sort = sort;
        self.gallery_scroll_handle = VirtualListScrollHandle::new();
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn ensure_image_library_loaded(&mut self, cx: &mut Context<Self>) {
        if self.gallery_groups_loaded || self.gallery_groups_loading {
            return;
        }
        self.gallery_groups_loading = true;
        let generation = self.gallery_groups_generation;
        let service = self.image_library.clone();
        let task = cx.background_spawn(async move { service.load_groups() });
        cx.spawn(async move |this, cx| match task.await {
            Ok(groups) => {
                this.update(cx, |this, cx| {
                    if this.gallery_groups_generation != generation {
                        return;
                    }
                    this.gallery_groups = groups;
                    this.gallery_groups_loading = false;
                    this.gallery_groups_loaded = true;
                    let selected_group = this
                        .selected_gallery_group_id
                        .as_ref()
                        .filter(|id| this.gallery_groups.iter().any(|group| &group.id == *id))
                        .cloned()
                        .or_else(|| this.gallery_groups.first().map(|group| group.id.clone()));
                    if let Some(group_id) = selected_group {
                        this.select_gallery_group(&group_id, cx);
                    }
                    this.ensure_gallery_group_icons(cx);
                    cx.notify();
                })
                .ok();
            }
            Err(error) => {
                this.update(cx, |this, cx| {
                    if this.gallery_groups_generation != generation {
                        return;
                    }
                    this.gallery_groups_loading = false;
                    this.gallery_groups_loaded = true;
                    tracing::error!("图库组加载失败：{error}");
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn add_image_library_group(
        &mut self,
        group: crate::services::image_library::ImageLibraryGroup,
        cx: &mut Context<Self>,
    ) {
        if !self.is_gallery_active() {
            return;
        }
        self.gallery_groups_generation = self.gallery_groups_generation.wrapping_add(1);
        self.gallery_groups_loading = false;
        self.gallery_group_icon_loading.clear();
        self.gallery_groups.push(group.clone());
        self.gallery_groups_loaded = true;
        self.select_gallery_group(&group.id, cx);
        self.ensure_gallery_group_icons(cx);
        cx.notify();
    }

    pub(crate) fn update_image_library_group(
        &mut self,
        group: crate::services::image_library::ImageLibraryGroup,
        cx: &mut Context<Self>,
    ) {
        if !self.is_gallery_active() {
            return;
        }
        let Some(index) = self
            .gallery_groups
            .iter()
            .position(|candidate| candidate.id == group.id)
        else {
            return;
        };
        self.gallery_groups_generation = self.gallery_groups_generation.wrapping_add(1);
        self.gallery_group_icon_loading.clear();
        let old_group = self.gallery_groups[index].clone();
        let old_icon_path = self.image_library.group_icon_path(&old_group);
        let new_icon_path = self.image_library.group_icon_path(&group);
        self.gallery_groups[index] = group.clone();

        if let Some(path) = old_icon_path.or(new_icon_path) {
            self.gallery_group_icon_failures.remove(&path);
            self.gallery_group_icon_loading.remove(&path);
            if let Some(image) = self.loaded_gallery_group_icons.remove(&path) {
                self.pending_gallery_image_drops.push(image);
            }
        }
        self.ensure_gallery_group_icons(cx);
        if self.selected_gallery_group_id.as_deref() == Some(group.id.as_str()) {
            self.select_gallery_group(&group.id, cx);
        }
        cx.notify();
    }

    pub(crate) fn remove_image_library_group(&mut self, group_id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.gallery_groups.iter().position(|group| group.id == group_id) else {
            return;
        };
        let group = self.gallery_groups.remove(index);
        self.gallery_groups_generation = self.gallery_groups_generation.wrapping_add(1);
        self.gallery_groups_loading = false;
        self.gallery_group_icon_loading.clear();
        if let Some(path) = self.image_library.group_icon_path(&group) {
            self.gallery_group_icon_failures.remove(&path);
            if let Some(image) = self.loaded_gallery_group_icons.remove(&path) {
                self.pending_gallery_image_drops.push(image);
            }
        }
        if self.selected_gallery_group_id.as_deref() == Some(group_id) {
            self.selected_gallery_group_id = None;
            self.gallery_images_generation = self.gallery_images_generation.wrapping_add(1);
            self.gallery_images.clear();
            self.gallery_images_loading = false;
            self.gallery_drop_hovering = false;
            self.symbol_smooth_scroll.reset();
            self.gallery_scroll_handle = VirtualListScrollHandle::new();
            self.clear_gallery_image_cache();
            let next_group = self.gallery_groups.get(index)
                .or_else(|| self.gallery_groups.last())
                .map(|group| group.id.clone());
            if let Some(id) = next_group {
                self.select_gallery_group(&id, cx);
            }
        }
        if self.is_gallery_active() {
            self.ensure_gallery_group_icons(cx);
        }
        cx.notify();
    }

    pub(crate) fn select_gallery_group(&mut self, group_id: &str, cx: &mut Context<Self>) {
        if !self.is_gallery_active() {
            return;
        }
        let Some(group) = self
            .gallery_groups
            .iter()
            .find(|group| group.id == group_id)
            .cloned()
        else {
            return;
        };
        self.selected_gallery_group_id = Some(group.id.clone());
        self.symbol_smooth_scroll.reset();
        self.gallery_images.clear();
        self.gallery_images_loading = true;
        self.gallery_images_generation = self.gallery_images_generation.wrapping_add(1);
        let generation = self.gallery_images_generation;
        self.gallery_scroll_handle = VirtualListScrollHandle::new();
        self.clear_gallery_image_cache();

        let service = self.image_library.clone();
        let expected_group_id = group.id.clone();
        let task = cx.background_spawn(async move { service.list_images(&group) });
        cx.spawn(async move |this, cx| match task.await {
            Ok(images) => {
                this.update(cx, |this, cx| {
                    if this.gallery_images_generation != generation
                        || this.selected_gallery_group_id.as_deref()
                            != Some(expected_group_id.as_str())
                    {
                        return;
                    }
                    this.gallery_images = images;
                    this.gallery_images_loading = false;
                    cx.notify();
                })
                .ok();
            }
            Err(error) => {
                this.update(cx, |this, cx| {
                    if this.gallery_images_generation != generation {
                        return;
                    }
                    this.gallery_images_loading = false;
                    tracing::error!("图库图片列表加载失败：{error}");
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn refresh_gallery_images(&mut self, cx: &mut Context<Self>) {
        if let Some(group_id) = self.selected_gallery_group_id.clone() {
            self.select_gallery_group(&group_id, cx);
        }
    }

    pub(crate) fn import_gallery_images(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let Some(group) = self
            .selected_gallery_group_id
            .as_ref()
            .and_then(|id| self.gallery_groups.iter().find(|group| &group.id == id))
            .cloned()
        else {
            return;
        };
        let total = paths.len();
        self.gallery_import_progress = Some(GalleryImportProgress {
            completed: 0,
            total,
        });
        cx.notify();

        let service = self.image_library.clone();
        let (progress_sender, progress_receiver) = async_channel::unbounded();
        let task = cx.background_spawn(async move {
            service.import_images_with_progress(&group, &paths, |completed, _| {
                let _ = progress_sender.try_send(completed);
            })
        });
        cx.spawn(async move |this, cx| {
            while let Ok(completed) = progress_receiver.recv().await {
                let Some(this) = this.upgrade() else {
                    break;
                };
                this.update(cx, |this, cx| {
                    if let Some(progress) = &mut this.gallery_import_progress {
                        progress.completed = completed;
                        cx.notify();
                    }
                });
            }

            let result = task.await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    this.gallery_import_progress = None;
                    match result {
                        Ok(copied) => {
                            if copied > 0 {
                                this.refresh_gallery_images(cx);
                            }
                        }
                        Err(error) => tracing::error!("导入图库图片失败：{error}"),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn ensure_gallery_group_icons(&mut self, cx: &mut Context<Self>) {
        let generation = self.gallery_groups_generation;
        let paths = self
            .gallery_groups
            .iter()
            .filter_map(|group| self.image_library.group_icon_path(group))
            .collect::<Vec<_>>();
        for path in paths {
            if self.loaded_gallery_group_icons.contains_key(&path)
                || self.gallery_group_icon_failures.contains(&path)
                || !self.gallery_group_icon_loading.insert(path.clone())
            {
                continue;
            }
            let service = self.image_preview_service.clone();
            let load_path = path.clone();
            let task = cx.background_spawn(async move {
                service.load_local_path(&load_path, (48, 48))
            });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| {
                        if this.gallery_groups_generation != generation {
                            return;
                        }
                        this.gallery_group_icon_loading.remove(&path);
                        match result {
                            Ok(image) if this.is_gallery_active() => {
                                this.loaded_gallery_group_icons.insert(path, image);
                                cx.notify();
                            }
                            Err(error) if this.is_gallery_active() => {
                                tracing::error!("图库组图标加载失败：{error}");
                                this.gallery_group_icon_failures.insert(path);
                            }
                            Ok(_) | Err(_) => {}
                        }
                    });
                }
            })
            .detach();
        }
    }

    pub(crate) fn ensure_gallery_thumbnails(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible_paths = paths.iter().cloned().collect::<HashSet<_>>();
        self.gallery_visible_image_paths = visible_paths.clone();
        self.gallery_image_queue
            .retain(|path| visible_paths.contains(path));
        self.trim_gallery_thumbnail_cache(paths, window);

        for path in paths {
            if self.loaded_gallery_images.contains_key(path)
                || self.gallery_image_load_failures.contains(path)
                || self.gallery_image_loading.contains(path)
                || self.gallery_image_queue.contains(path)
            {
                continue;
            }
            self.gallery_image_queue.push_back(path.clone());
        }

        self.start_gallery_thumbnail_loads(cx);
    }

    pub(crate) fn trim_gallery_thumbnail_cache(
        &mut self,
        allowed_paths: &[PathBuf],
        window: &mut Window,
    ) {
        let allowed = allowed_paths.iter().collect::<HashSet<_>>();
        let stale_paths = self
            .loaded_gallery_images
            .keys()
            .filter(|path| !allowed.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        for path in stale_paths {
            if let Some(image) = self.loaded_gallery_images.remove(&path) {
                if let Err(error) = window.drop_image(image) {
                    tracing::warn!("释放图库缩略图纹理失败：{error}");
                }
            }
        }
        self.gallery_image_load_failures
            .retain(|path| allowed.contains(path));
        let stale_animations = self
            .loaded_gallery_animations
            .keys()
            .filter(|path| !allowed.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        for path in stale_animations {
            if let Some(image) = self.loaded_gallery_animations.remove(&path) {
                if let Err(error) = window.drop_image(image) {
                    tracing::warn!("释放图库 GIF 纹理失败：{error}");
                }
            }
        }
    }

    pub(crate) fn cleanup_gallery_image_atlas(&mut self, window: &mut Window) {
        for image in self.pending_gallery_image_drops.drain(..) {
            if let Err(error) = window.drop_image(image) {
                tracing::warn!("释放图库图片纹理失败：{error}");
            }
        }
    }

    pub(crate) fn clear_gallery_resources(&mut self) {
        self.gallery_groups_generation = self.gallery_groups_generation.wrapping_add(1);
        self.gallery_images_generation = self.gallery_images_generation.wrapping_add(1);
        self.gallery_groups.clear();
        self.gallery_groups_loading = false;
        self.gallery_groups_loaded = false;
        self.selected_gallery_group_id = None;
        self.gallery_images.clear();
        self.gallery_images_loading = false;
        self.gallery_scroll_handle = VirtualListScrollHandle::new();
        self.clear_gallery_image_cache();
        self.pending_gallery_image_drops.extend(
            std::mem::take(&mut self.loaded_gallery_group_icons).into_values(),
        );
        self.gallery_group_icon_loading.clear();
        self.gallery_group_icon_failures.clear();
        self.gallery_drop_hovering = false;
        self.gallery_import_progress = None;
    }

    pub(super) fn clear_gallery_image_cache(&mut self) {
        self.gallery_visible_image_paths.clear();
        self.gallery_image_queue.clear();
        self.gallery_image_loading.clear();
        self.gallery_image_load_failures.clear();
        self.gallery_animation_loading.clear();
        self.gallery_animation_failures.clear();
        self.gallery_hovered_image = None;
        self.pending_gallery_image_drops
            .extend(std::mem::take(&mut self.loaded_gallery_images).into_values());
        self.pending_gallery_image_drops
            .extend(std::mem::take(&mut self.loaded_gallery_animations).into_values());
    }

    pub(super) fn is_gallery_active(&self) -> bool {
        self.active_tab == TabId::Emoji && self.symbol_category == SymbolCategoryId::Gallery
    }

    pub(super) fn start_gallery_thumbnail_loads(&mut self, cx: &mut Context<Self>) {
        while self.gallery_image_loading.len() < MAX_GALLERY_IMAGE_LOADS {
            let Some(path) = self.gallery_image_queue.pop_front() else {
                break;
            };
            if !self.gallery_visible_image_paths.contains(&path)
                || self.loaded_gallery_images.contains_key(&path)
                || self.gallery_image_load_failures.contains(&path)
                || !self.gallery_image_loading.insert(path.clone())
            {
                continue;
            }

            let generation = self.gallery_images_generation;
            let service = self.image_preview_service.clone();
            let load_path = path.clone();
            let task = cx.background_spawn(async move {
                service.load_local_path(&load_path, (256, 256))
            });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| {
                        if this.gallery_images_generation != generation {
                            return;
                        }
                        this.gallery_image_loading.remove(&path);
                        if this.is_gallery_active()
                            && this.gallery_visible_image_paths.contains(&path)
                        {
                            match result {
                                Ok(image) => {
                                    this.loaded_gallery_images.insert(path, image);
                                }
                                Err(error) => {
                                    tracing::error!("图库图片缩略图加载失败：{error}");
                                    this.gallery_image_load_failures.insert(path);
                                }
                            }
                        }
                        if !this.is_gallery_active() {
                            return;
                        }
                        this.start_gallery_thumbnail_loads(cx);
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    }

    pub(crate) fn ensure_gallery_animation(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.is_gallery_active() {
            return;
        }
        self.gallery_hovered_image = Some(path.clone());
        if self.gallery_animation_failures.contains(&path)
            || self.loaded_gallery_animations.contains_key(&path)
            || self.gallery_animation_loading.contains(&path)
        {
            cx.notify();
            return;
        }
        if !self.gallery_animation_loading.is_empty() {
            return;
        }
        self.gallery_animation_loading.insert(path.clone());
        let generation = self.gallery_images_generation;
        let service = self.image_preview_service.clone();
        let load_path = path.clone();
        let task = cx.background_spawn(async move {
            service.load_animated_gif(&load_path, (256, 256))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if this.gallery_images_generation != generation {
                        return;
                    }
                    this.gallery_animation_loading.remove(&path);
                    if this.is_gallery_active()
                        && this.gallery_hovered_image.as_ref() == Some(&path)
                    {
                        match result {
                            Ok(Some(image)) => {
                                this.loaded_gallery_animations.insert(path.clone(), image);
                                cx.notify();
                            }
                            Ok(None) => {
                                this.gallery_animation_failures.insert(path.clone());
                            }
                            Err(error) => {
                                tracing::warn!("GIF 动画无法预览：{error}");
                                this.gallery_animation_failures.insert(path.clone());
                            }
                        }
                    }
                    if let Some(next_path) = this
                        .gallery_hovered_image
                        .clone()
                        .filter(|next_path| next_path != &path)
                    {
                        this.ensure_gallery_animation(next_path, cx);
                    }
                });
            }
        })
        .detach();
    }

    pub(crate) fn clear_gallery_hover(&mut self, path: &Path) {
        if self.gallery_hovered_image.as_deref() == Some(path) {
            self.gallery_hovered_image = None;
            if let Some(image) = self.loaded_gallery_animations.remove(path) {
                self.pending_gallery_image_drops.push(image);
            }
        }
    }

    pub(crate) fn paste_gallery_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let paste_service = self.paste_service.clone();
        cx.defer(move |cx| {
            crate::windows::main::hide_main_window(cx);
            if let Err(error) = crate::services::focus::restore_previous_focus() {
                tracing::error!("粘贴图库图片前恢复窗口焦点失败：{error}");
            }
            let task = cx.background_spawn(async move { paste_service.paste_gallery_image(&path) });
            cx.spawn(async move |cx| {
                if let Err(error) = task.await {
                    tracing::error!("粘贴图库图片失败：{error}");
                }
            })
            .detach();
        });
    }

    pub(crate) fn copy_gallery_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let paste_service = self.paste_service.clone();
        let task = cx.background_spawn(async move { paste_service.copy_gallery_image(&path) });
        cx.spawn(async move |_this, _cx| {
            if let Err(error) = task.await {
                tracing::error!("复制图库图片失败：{error}");
            }
        })
        .detach();
    }

    pub(crate) fn rename_gallery_image(
        &mut self,
        path: PathBuf,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self
            .selected_gallery_group_id
            .as_ref()
            .and_then(|id| self.gallery_groups.iter().find(|group| &group.id == id))
            .cloned()
        else {
            return;
        };
        let service = self.image_library.clone();
        let task = cx.background_spawn(async move { service.rename_image(&group, &path, &name) });
        cx.spawn(async move |this, cx| match task.await {
            Ok(_) => {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| this.refresh_gallery_images(cx));
                }
            }
            Err(error) => tracing::error!("重命名图库图片失败：{error}"),
        })
        .detach();
    }

    pub(crate) fn move_gallery_image(
        &mut self,
        path: PathBuf,
        target_group_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(source_group) = self
            .selected_gallery_group_id
            .as_ref()
            .and_then(|id| self.gallery_groups.iter().find(|group| &group.id == id))
            .cloned()
        else {
            return;
        };
        let Some(target_group) = self
            .gallery_groups
            .iter()
            .find(|group| group.id == target_group_id)
            .cloned()
        else {
            return;
        };
        let service = self.image_library.clone();
        let task = cx.background_spawn(async move {
            service.move_image(&source_group, &target_group, &path)
        });
        cx.spawn(async move |this, cx| match task.await {
            Ok(()) => {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| {
                        this.gallery_images_generation =
                            this.gallery_images_generation.wrapping_add(1);
                        this.clear_gallery_image_cache();
                        this.refresh_gallery_images(cx);
                    });
                }
            }
            Err(error) => tracing::error!("移动图库图片失败：{error}"),
        })
        .detach();
    }

    pub(crate) fn delete_gallery_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(group) = self
            .selected_gallery_group_id
            .as_ref()
            .and_then(|id| self.gallery_groups.iter().find(|group| &group.id == id))
            .cloned()
        else {
            return;
        };
        let service = self.image_library.clone();
        let task = cx.background_spawn(async move { service.delete_image(&group, &path) });
        cx.spawn(async move |this, cx| match task.await {
            Ok(()) => {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| {
                        this.gallery_images_generation =
                            this.gallery_images_generation.wrapping_add(1);
                        this.clear_gallery_image_cache();
                        this.refresh_gallery_images(cx);
                    });
                }
            }
            Err(error) => tracing::error!("删除图库图片失败：{error}"),
        })
        .detach();
    }

}
