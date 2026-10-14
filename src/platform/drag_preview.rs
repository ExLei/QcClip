use gpui::{
    AnyView, AnyWindowHandle, App, AppContext, BorrowAppContext, Bounds, Context, DisplayId,
    Entity, IntoElement, ParentElement, Pixels, Point, Render, Size, Window, WindowHandle, div, px,
};

use crate::platform::input::pointer_snapshot;
use crate::platform::window::{BorderlessPopup, open_borderless_mouse_passthrough_popup};

#[derive(Clone, Copy, Debug)]
pub(crate) struct NativeDragPreviewTarget {
    pub(crate) hwnd: isize,
    pub(crate) cursor_offset_physical: Point<f32>,
    pub(crate) width_physical: f32,
    pub(crate) height_physical: f32,
}

pub(crate) struct DragPreviewService {
    window: Option<WindowHandle<DragPreviewWindow>>,
}

impl Default for DragPreviewService {
    fn default() -> Self {
        Self { window: None }
    }
}

impl gpui::Global for DragPreviewService {}

impl DragPreviewService {
    pub(crate) fn install(cx: &mut App) {
        cx.set_global(Self::default());
        cx.on_window_closed(|cx, window_id| {
            cx.update_global::<Self, _>(|service, _| {
                if service
                    .window
                    .is_some_and(|window| window.window_id() == window_id)
                {
                    service.window = None;
                }
            });
        })
        .detach();
    }

    pub(crate) fn start<V>(
        source_window: AnyWindowHandle,
        preview: Entity<V>,
        initial_bounds: Bounds<Pixels>,
        display_id: Option<DisplayId>,
        cursor_offset_physical: Point<f32>,
        rem_size: Pixels,
        on_finish: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) -> Result<(), String>
    where
        V: 'static + Render,
    {
        cx.update_global::<Self, _>(|service, cx| {
            let window = match service.window {
                Some(window) => window,
                None => {
                    let window = open_borderless_mouse_passthrough_popup(
                        cx,
                        initial_bounds,
                        display_id,
                        |_window, cx, popup| cx.new(|_| DragPreviewWindow::new(popup)),
                    )?;
                    service.window = Some(window);
                    window
                }
            };

            window
                .update(cx, |preview_window, window, cx| {
                    preview_window.start(
                        window,
                        AnyView::from(preview),
                        source_window,
                        initial_bounds.size,
                        cursor_offset_physical,
                        rem_size,
                        on_finish,
                        cx,
                    );
                })
                .map_err(|error| format!("更新拖拽预览窗口失败：{error}"))
        })
    }

    pub(crate) fn hide(cx: &mut App) {
        cx.update_global::<Self, _>(|service, cx| {
            let Some(window) = service.window else {
                return;
            };
            if let Err(error) = window.update(cx, |preview_window, window, cx| {
                preview_window.hide(window, cx);
            }) {
                tracing::debug!("隐藏拖拽预览窗口失败：{error}");
            }
        });
    }

    pub(crate) fn finish(cx: &mut App) {
        cx.update_global::<Self, _>(|service, cx| {
            let Some(window) = service.window else {
                return;
            };
            if let Err(error) = window.update(cx, |preview_window, window, cx| {
                preview_window.finish(window, cx);
            }) {
                tracing::debug!("完成拖拽预览时更新窗口失败：{error}");
            }
        });
    }

    pub(crate) fn refresh(cx: &mut App) {
        let Some(window) = cx.try_global::<Self>().and_then(|service| service.window) else {
            return;
        };
        if let Err(error) = window.update(cx, |_, window, _| window.refresh()) {
            tracing::debug!("刷新拖拽预览窗口失败：{error}");
        }
    }

    pub(crate) fn native_follow_target(cx: &mut App) -> Option<NativeDragPreviewTarget> {
        let window = cx.try_global::<Self>().and_then(|service| service.window)?;
        window
            .update(cx, |preview_window, window, _| {
                preview_window.native_follow_target(window)
            })
            .ok()
            .flatten()
    }
}

struct DragPreviewWindow {
    popup: BorderlessPopup,
    preview: Option<AnyView>,
    source_window: Option<AnyWindowHandle>,
    preview_size: Size<Pixels>,
    cursor_offset_physical: Point<f32>,
    rem_size: Pixels,
    on_finish: Option<Box<dyn FnOnce(&mut App) + 'static>>,
}

impl DragPreviewWindow {
    fn new(popup: BorderlessPopup) -> Self {
        Self {
            popup,
            preview: None,
            source_window: None,
            preview_size: Size::default(),
            cursor_offset_physical: Point::default(),
            rem_size: px(16.0),
            on_finish: None,
        }
    }

    fn start(
        &mut self,
        window: &Window,
        preview: AnyView,
        source_window: AnyWindowHandle,
        preview_size: Size<Pixels>,
        cursor_offset_physical: Point<f32>,
        rem_size: Pixels,
        on_finish: impl FnOnce(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.preview = Some(preview);
        self.source_window = Some(source_window);
        self.preview_size = preview_size;
        self.cursor_offset_physical = cursor_offset_physical;
        self.rem_size = rem_size;
        self.on_finish = Some(Box::new(on_finish));
        self.popup.set_visible(window, true);
        cx.notify();
    }

    fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.popup.set_visible(window, false);
        self.preview = None;
        self.source_window = None;
        self.on_finish = None;
        cx.notify();
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.popup.set_visible(window, false);
        self.preview = None;
        let source_window = self.source_window.take();
        let on_finish = self.on_finish.take();
        if let Some(source_window) = source_window {
            if let Err(error) = source_window.update(cx, |_, source_window, cx| {
                cx.stop_active_drag(source_window);
            }) {
                tracing::debug!("结束拖拽时恢复源窗口状态失败：{error}");
            }
        }
        if let Some(on_finish) = on_finish {
            on_finish(cx);
        }
        cx.notify();
    }

    #[cfg(target_os = "windows")]
    fn native_follow_target(&self, window: &Window) -> Option<NativeDragPreviewTarget> {
        Some(NativeDragPreviewTarget {
            hwnd: self.popup.native_window_handle(window)?,
            cursor_offset_physical: self.cursor_offset_physical,
            width_physical: f32::from(self.preview_size.width) * window.scale_factor().max(1.0),
            height_physical: f32::from(self.preview_size.height) * window.scale_factor().max(1.0),
        })
    }

    #[cfg(not(target_os = "windows"))]
    fn native_follow_target(&self, _window: &Window) -> Option<NativeDragPreviewTarget> {
        None
    }
}

impl Render for DragPreviewWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(preview) = self.preview.clone() else {
            return div().into_any_element();
        };

        window.set_rem_size(self.rem_size);
        if let Some(pointer) = pointer_snapshot() {
            if !pointer.buttons.left {
                self.finish(window, cx);
            } else {
                let scale_factor = window.scale_factor().max(1.0);
                let x = pointer.position.x - self.cursor_offset_physical.x;
                let y = pointer.position.y - self.cursor_offset_physical.y;
                let width = f32::from(self.preview_size.width) * scale_factor;
                let height = f32::from(self.preview_size.height) * scale_factor;
                let popup = self.popup;
                window.defer(cx, move |window, cx| {
                    popup.resize_to_physical_bounds(window, cx, x, y, width, height);
                });
                window.on_next_frame(|window, _| window.refresh());
            }
        }

        div().child(preview).into_any_element()
    }
}
