use super::*;
use crate::windows::main::state::{HeatBucket, HeatScrollbarState};
use crate::windows::main::window_drag::exclude_from_window_drag;

fn item_ratio(index: usize, total: usize, item_offset: &impl Fn(usize) -> Option<Pixels>) -> f32 {
    if let (Some(offset), Some(height)) = (item_offset(index), item_offset(total))
        && height > px(0.0)
    {
        return (f32::from(offset) / f32::from(height)).clamp(0.0, 1.0);
    }
    index as f32 / total.max(1) as f32
}

fn bucket_at(
    buckets: &[HeatBucket], bounds: gpui::Bounds<Pixels>, y: Pixels,
    item_offset: impl Fn(usize) -> Option<Pixels>,
) -> Option<usize> {
    let total = buckets.last()?.range.end;
    let height = f32::from(bounds.size.height);
    if height <= 0.0 {
        return None;
    }
    let ratio = (f32::from(y - bounds.top()) / height).clamp(0.0, 1.0);
    Some(buckets.partition_point(|bucket| item_ratio(bucket.range.end, total, &item_offset) <= ratio)
        .min(buckets.len() - 1))
}

fn heat_level(count: u64) -> u8 {
    match count {
        0 => 0,
        1..=9 => 1,
        10..=49 => 2,
        50..=99 => 3,
        100..=199 => 4,
        200..=499 => 5,
        _ => 6,
    }
}

fn heat_color(level: u8) -> gpui::Hsla {
    gpui::rgb_to_hsla(gpui::rgb(match level {
        0 => 0xd5e6da,
        1 => 0x8fba9c,
        2 => 0x4d9671,
        3 => 0xd5b75f,
        4 => 0xdd965b,
        5 => 0xcc7466,
        _ => 0xbd5c5b,
    }))
}

fn heat_runs(buckets: &[HeatBucket]) -> Vec<(Range<usize>, u8)> {
    let mut runs: Vec<(Range<usize>, u8)> = Vec::new();
    for bucket in buckets {
        let level = heat_level(bucket.max_count);
        if let Some((range, previous)) = runs.last_mut()
            && *previous == level && range.end == bucket.range.start
        {
            range.end = bucket.range.end;
        } else {
            runs.push((bucket.range.clone(), level));
        }
    }
    runs
}

fn overview_buckets(buckets: Rc<Vec<HeatBucket>>, height: Pixels, scale: f32) -> Rc<Vec<HeatBucket>> {
    let count = if height <= px(0.0) {
        256
    } else {
        ((f32::from(height) / (3.0 * scale)).floor() as usize).clamp(1, 256)
    };
    if buckets.len() <= count {
        return buckets;
    }
    // 只合并已有统计；概览和悬停共用同一分段，孤立热点仍按最大值保留。
    Rc::new((0..count).map(|slot| {
        let start = slot * buckets.len() / count;
        let end = (slot + 1) * buckets.len() / count;
        let mut merged = buckets[start].clone();
        for bucket in &buckets[start + 1..end] {
            merged.range.end = bucket.range.end;
            merged.total_count = merged.total_count.saturating_add(bucket.total_count);
            if bucket.max_count > merged.max_count {
                merged.max_count = bucket.max_count;
                merged.peak_index = bucket.peak_index;
            }
        }
        merged
    }).collect())
}

fn drag_delta(start: Point<Pixels>, current: Point<Pixels>, dragging: bool, scale: f32) -> Option<Pixels> {
    let delta = current - start;
    let distance_squared = f32::from(delta.x).powi(2) + f32::from(delta.y).powi(2);
    (dragging || distance_squared >= (4.0 * scale).powi(2)).then_some(delta.y)
}

fn thumb_bounds(
    track: gpui::Bounds<Pixels>, viewport_height: Pixels, max_offset: Pixels,
    offset: Pixels, scale: f32,
) -> gpui::Bounds<Pixels> {
    let height = track.size.height.max(px(0.0));
    let content_height = viewport_height + max_offset;
    let fraction = if content_height > px(0.0) {
        (f32::from(viewport_height) / f32::from(content_height)).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let thumb_height = (height * fraction).max(px(24.0 * scale)).min(height);
    let progress = if max_offset > px(0.0) {
        (-f32::from(offset) / f32::from(max_offset)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    gpui::Bounds::new(
        point(track.left() + track.size.width / 12.0, track.top() + (height - thumb_height) * progress),
        size(track.size.width * (5.0 / 6.0), thumb_height),
    )
}

fn pointer_offset(
    track: gpui::Bounds<Pixels>, thumb_height: Pixels, grip: f32, pointer_y: Pixels, max_offset: Pixels,
) -> Pixels {
    let travel = track.size.height - thumb_height;
    if travel <= px(0.0) || max_offset <= px(0.0) {
        return px(0.0);
    }
    (-max_offset * (f32::from(pointer_y - track.top() - thumb_height * grip) / f32::from(travel)))
        .clamp(-max_offset, px(0.0))
}

fn track_press_offset(
    track: gpui::Bounds<Pixels>, thumb: gpui::Bounds<Pixels>, y: Pixels, max_offset: Pixels,
) -> Option<Pixels> {
    // 滑块的抓取范围覆盖整条交互区，保留鼠标在滑块内的相对位置。
    if thumb.size.height > px(0.0) && y >= thumb.top() && y <= thumb.bottom() {
        return None;
    }
    Some(pointer_offset(track, thumb.size.height, 0.5, y, max_offset))
}

pub(super) fn render(
    buckets: Rc<Vec<HeatBucket>>,
    state: &HeatScrollbarState,
    scroll_handle: &VirtualListScrollHandle,
    list_width: Pixels,
    window: &Window,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    if buckets.is_empty() {
        return div().into_any_element();
    }
    let theme = cx.theme().colors;
    let dark = cx.theme().mode.is_dark();
    let scale = crate::ui::scale::window_scale(window);
    let buckets = overview_buckets(buckets, state.bounds.size.height, scale);
    let entity = cx.entity();
    let total = buckets.last().expect("热度分段不能为空").range.end;
    let paint_runs = heat_runs(&buckets);
    let paint_buckets = buckets.clone();
    let move_buckets = buckets.clone();
    let click_buckets = buckets.clone();
    let window_hovered = window.is_window_hovered();
    let pressed = state.drag_origin.is_some();
    let visible = state.visible && (window_hovered || pressed);
    let expanded = state.expanded() && (window_hovered || pressed);
    let animations_enabled = !cx.reduce_motion()
        && config_runtime::animation_enabled(cx, crate::config::AnimationKind::ListItemHover);
    let motion = Duration::from_millis(180).with_easing(gpui::ease_in_out);
    let mut rail_background = theme.background;
    rail_background.alpha = 0.88;
    let thumb_color = gpui::rgb_to_hsla(gpui::rgb(if dark { 0xafc9b5 } else { 0x4f6658 }));

    let rail = div()
        .id("clipboard-heat-scrollbar")
        .absolute()
        .right_0()
        .top_0()
        .bottom_0()
        .w(ui_rem(28.0))
        .cursor_pointer()
        .block_mouse_except_scroll()
        .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
            this.set_heat_scrollbar_hovered(*hovered && window.is_window_hovered(), cx);
        }))
        .on_mouse_move(cx.listener(move |this, event: &gpui::MouseMoveEvent, window, cx| {
            if !window.is_window_hovered() || this.heat_scrollbar.drag_origin.is_some()
                || !this.heat_scrollbar.bounds.contains(&event.position)
            {
                return;
            }
            if !this.heat_scrollbar.expanded() {
                this.set_heat_scrollbar_hovered(true, cx);
            }
            let slot = bucket_at(&move_buckets, this.heat_scrollbar.bounds, event.position.y,
                |index| this.list_scroll_handle.item_offset(index));
            if slot != this.heat_scrollbar.hovered_bucket {
                this.heat_scrollbar.hovered_bucket = slot;
                cx.notify();
            }
        }))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.set_heat_scrollbar_hovered(true, cx);
            this.list_smooth_scroll.reset();
            let handle = this.list_scroll_handle.as_ref();
            let jump_offset = track_press_offset(
                this.heat_scrollbar.bounds, this.heat_scrollbar.thumb_bounds,
                event.position.y, handle.max_offset().y,
            );
            let mut offset = handle.offset();
            if let Some(y) = jump_offset {
                offset.y = y;
                handle.set_offset(offset);
            }
            this.heat_scrollbar.dragging = jump_offset.is_none();
            this.heat_scrollbar.hovered_bucket = None;
            let thumb = thumb_bounds(
                this.heat_scrollbar.bounds, handle.bounds().size.height, handle.max_offset().y, offset.y, scale,
            );
            let grip = if thumb.size.height > px(0.0) {
                (f32::from(event.position.y - thumb.top()) / f32::from(thumb.size.height)).clamp(0.0, 1.0)
            } else { 0.5 };
            this.heat_scrollbar.drag_origin = Some((event.position, grip));
            this.heat_scrollbar.drag_pointer_y = event.position.y;
            cx.notify();
        }))
        .on_mouse_move_all({
            let entity = entity.clone();
            move |event: &gpui::MouseMoveEvent, phase, _, window, cx| {
                if phase != gpui::DispatchPhase::Bubble {
                    return;
                }
                entity.update(cx, |this, cx| {
                    let Some((start, grip)) = this.heat_scrollbar.drag_origin else { return };
                    if event.pressed_button != Some(MouseButton::Left) {
                        // 非客户区的移动事件不携带按键状态；窗口外等待捕获后的松键事件。
                        if !window.is_window_hovered() { return; }
                        this.heat_scrollbar.drag_origin = None;
                        this.heat_scrollbar.dragging = false;
                        cx.notify();
                        if !this.heat_scrollbar.bounds.contains(&event.position) {
                            this.set_heat_scrollbar_hovered(false, cx);
                        }
                        return;
                    }
                    if drag_delta(start, event.position, this.heat_scrollbar.dragging, scale).is_none() {
                        return;
                    }
                    this.heat_scrollbar.dragging = true;
                    this.heat_scrollbar.hovered_bucket = None;
                    this.heat_scrollbar.drag_pointer_y = event.position.y;
                    let handle = this.list_scroll_handle.as_ref();
                    let mut offset = handle.offset();
                    offset.y = pointer_offset(this.heat_scrollbar.bounds,
                        this.heat_scrollbar.thumb_bounds.size.height, grip, event.position.y, handle.max_offset().y);
                    handle.set_offset(offset);
                    cx.stop_propagation();
                    cx.notify();
                });
            }
        })
        .on_mouse_up(MouseButton::Left, cx.listener(move |this, event: &gpui::MouseUpEvent, window, cx| {
            cx.stop_propagation();
            let press = this.heat_scrollbar.drag_origin.take();
            let was_dragging = this.heat_scrollbar.dragging;
            this.heat_scrollbar.dragging = false;
            if let Some((start, _)) = press
                && !was_dragging && drag_delta(start, event.position, false, scale).is_none()
                && let Some(slot) = bucket_at(&click_buckets, this.heat_scrollbar.bounds, start.y,
                    |index| this.list_scroll_handle.item_offset(index))
                && let Some(bucket) = click_buckets.get(slot)
            {
                this.heat_scrollbar.hovered_bucket = Some(slot);
                this.navigate_heat_index(bucket.peak_index, window, cx);
            }
            if !window.is_window_hovered() {
                this.heat_scrollbar.clear_on_window_exit();
            }
            cx.notify();
        }))
        .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, window, cx| {
            if this.heat_scrollbar.drag_origin.take().is_some() {
                cx.stop_propagation();
                this.heat_scrollbar.dragging = false;
                this.set_heat_scrollbar_hovered(false, cx);
                if !window.is_window_hovered() {
                    this.heat_scrollbar.clear_on_window_exit();
                }
            }
        }))
        .child(canvas(
            {
                let entity = entity.clone();
                move |bounds, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.heat_scrollbar.bounds = bounds;
                        this.observe_heat_scrollbar_scroll(cx);
                        // Windows 离开事件只刷新窗口状态，不能依赖旧命中框清除悬停。
                        if !window.is_window_hovered() && this.heat_scrollbar.clear_on_window_exit() {
                            cx.notify();
                        }
                    });
                }
            },
            |_, _, _, _| {},
        ).absolute().size_full())
        .child(div()
            .id("clipboard-heat-scrollbar-track")
            .absolute()
            .right_0()
            .top_0()
            .bottom_0()
            .w(ui_rem(if expanded { 24.0 } else { 12.0 }))
            .bg(rail_background)
            .border_l_1()
            .border_color(gpui::ColorExt::opacity(&theme.muted_foreground, 0.16))
            .opacity(if visible { 1.0 } else { 0.0 })
            .when(animations_enabled, |this| {
                this.transitions(|transitions| transitions.w(motion.clone()).opacity(motion))
            })
            .child(canvas({
                let entity = entity.clone();
                move |bounds, _, cx| {
                    entity.update(cx, |this, cx| {
                        let handle = this.list_scroll_handle.as_ref();
                        let mut thumb = thumb_bounds(
                            bounds, handle.bounds().size.height, handle.max_offset().y, handle.offset().y, scale,
                        );
                        if this.heat_scrollbar.dragging
                            && let Some((_, grip)) = this.heat_scrollbar.drag_origin
                        {
                            let mut offset = handle.offset();
                            let y = pointer_offset(bounds, thumb.size.height, grip,
                                this.heat_scrollbar.drag_pointer_y, handle.max_offset().y);
                            if offset.y != y {
                                offset.y = y;
                                handle.set_offset(offset);
                                thumb = thumb_bounds(bounds, handle.bounds().size.height,
                                    handle.max_offset().y, y, scale);
                                cx.notify();
                            }
                        }
                        this.heat_scrollbar.thumb_bounds = thumb;
                    });
                }
            },
            move |bounds, _, window, cx| {
                let this = entity.read(cx);
                let state = &this.heat_scrollbar;
                let item_offset = |index| this.list_scroll_handle.item_offset(index);
                let width = bounds.size.width / 3.0;
                let x = bounds.left() + width;
                let heat_bounds = gpui::Bounds::new(point(x, bounds.top()), size(width, bounds.size.height));
                window.paint_quad(gpui::fill(heat_bounds, heat_color(0)).corner_radii(width / 2.0));
                // 合并相邻同档热度，避免大量分段边缘产生横纹。
                for (range, level) in &paint_runs {
                    if *level == 0 { continue; }
                    let top = bounds.top() + bounds.size.height * item_ratio(range.start, total, &item_offset);
                    let bottom = bounds.top() + bounds.size.height * item_ratio(range.end, total, &item_offset);
                    window.paint_quad(gpui::fill(
                        gpui::Bounds::new(point(x, top), size(width, (bottom - top - px(0.7 * scale)).max(px(0.0)))),
                        gpui::ColorExt::opacity(&heat_color(*level), 0.94),
                    ));
                }
                if let Some(bucket) = state.hovered_bucket.and_then(|slot| paint_buckets.get(slot)) {
                    let top = bounds.top() + bounds.size.height * item_ratio(bucket.range.start, total, &item_offset);
                    let bottom = bounds.top() + bounds.size.height * item_ratio(bucket.range.end, total, &item_offset);
                    window.paint_quad(gpui::outline(
                        gpui::Bounds::new(point(x, top), size(width, bottom - top)),
                        thumb_color,
                        gpui::BorderStyle::Solid,
                    ).border_widths(px(scale)));
                }
                let thumb = state.thumb_bounds;
                let border = bounds.size.width / 12.0;
                let radius = bounds.size.width / 6.0;
                window.paint_quad(gpui::outline(
                    gpui::Bounds::new(
                        point(thumb.left() - border, thumb.top()),
                        size(thumb.size.width + border * 2.0, thumb.size.height),
                    ),
                    rail_background,
                    gpui::BorderStyle::Solid,
                ).border_widths(border).corner_radii(radius + border));
                window.paint_quad(gpui::fill(
                    thumb,
                    gpui::ColorExt::opacity(&theme.background, if state.dragging { 0.12 } else { 0.28 }),
                ).border_widths(border).border_color(thumb_color).corner_radii(radius));
            },
        ).absolute().size_full()));

    let mut overlay = div()
        .absolute()
        .inset_0()
        .child(exclude_from_window_drag(rail, cx.entity()));
    if visible
        && expanded
        && window_hovered
        && !state.dragging
        && let Some(bucket) = state.hovered_bucket.and_then(|slot| buckets.get(slot))
    {
        let translator = i18n::translator(cx);
        let region = translator.translate_with_args("main.heatmap.region", &[
            ("start", (bucket.range.start + 1).to_string()),
            ("end", bucket.range.end.to_string()),
        ]);
        let summary = if bucket.max_count == 0 {
            translator.translate("main.heatmap.unused")
        } else {
            translator.translate_with_args("main.heatmap.summary", &[
                ("total", bucket.total_count.to_string()),
                ("peak", bucket.max_count.to_string()),
            ])
        };
        let top = (state.bounds.size.height * item_ratio(bucket.range.start, total,
            &|index| scroll_handle.item_offset(index)))
            .min((state.bounds.size.height - px(72.0 * scale)).max(px(0.0)));
        let mut background = theme.popover;
        background.alpha = 0.94;
        let foreground = theme.popover_foreground;
        let secondary = gpui::ColorExt::opacity(&foreground, 0.7);
        // 纯展示元素不注册命中区域，鼠标可穿透到下方列表。
        overlay = overlay.child(gpui::deferred(div()
            .absolute()
            .right(ui_rem(30.0))
            .top(top)
            .w_auto()
            .max_w((list_width - px(40.0 * scale)).max(px(0.0)))
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap(ui_rem(3.0))
            .rounded_md()
            .border_1()
            .border_color(gpui::ColorExt::opacity(&foreground, 0.12))
            .bg(background)
            .text_color(foreground)
            .text_xs()
            .shadow_sm()
            .child(region)
            .child(div().text_color(secondary).child(summary))).priority(1));
    }
    overlay.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_overview_limits_density_and_preserves_counts_and_peaks() {
        let buckets = Rc::new((0..256).map(|index| HeatBucket {
            range: index * 4..(index + 1) * 4,
            total_count: if index == 127 { 90 } else { 1 },
            max_count: if index == 127 { 90 } else { 1 },
            peak_index: index * 4,
        }).collect::<Vec<_>>());
        let overview = overview_buckets(buckets.clone(), px(360.0), 1.0);
        assert_eq!(overview.len(), 120);
        assert_eq!(overview.first().unwrap().range.start, 0);
        assert_eq!(overview.last().unwrap().range.end, 1024);
        assert!(overview.windows(2).all(|pair| pair[0].range.end == pair[1].range.start));
        assert_eq!(overview.iter().map(|bucket| bucket.total_count).sum::<u64>(), 345);
        let peak = overview.iter().find(|bucket| bucket.range.contains(&508)).unwrap();
        assert_eq!((peak.max_count, peak.peak_index), (90, 508));
        assert!(Rc::ptr_eq(&overview, &overview_buckets(overview.clone(), px(720.0), 1.0)));
        assert_eq!(overview_buckets(buckets.clone(), px(2.0), 1.0).len(), 1);
        assert_eq!(overview_buckets(buckets, px(0.0), 1.0).len(), 256);
    }

    #[test]
    fn heat_press_distinguishes_click_from_drag_anywhere_on_the_rail() {
        let start = point(px(14.0), px(200.0));
        assert_eq!(drag_delta(start, point(px(15.0), px(202.0)), false, 1.0), None);
        assert_eq!(drag_delta(start, point(px(14.0), px(204.0)), false, 1.0), Some(px(4.0)));
        assert_eq!(drag_delta(start, start, true, 1.0), Some(px(0.0)));
        assert_eq!(drag_delta(start, point(px(14.0), px(204.0)), false, 2.0), None);
        assert_eq!(drag_delta(start, point(px(14.0), px(208.0)), false, 2.0), Some(px(8.0)));
    }

    #[test]
    fn heat_runs_merge_similar_neighbors_without_losing_hotspots() {
        let counts = [0, 1, 9, 10, 49, 50, 99, 100, 199, 200, 499, 500, 0];
        let buckets = counts.iter().enumerate().map(|(index, count)| HeatBucket {
            range: index..index + 1, total_count: *count, max_count: *count, peak_index: index,
        }).collect::<Vec<_>>();
        assert_eq!(heat_runs(&buckets), vec![
            (0..1, 0), (1..3, 1), (3..5, 2), (5..7, 3),
            (7..9, 4), (9..11, 5), (11..12, 6), (12..13, 0),
        ]);
        assert!(heat_runs(&[]).is_empty());
    }

    #[test]
    fn heat_thumb_tracks_pixel_scroll_and_clamps_drag_at_both_ends() {
        let track = gpui::Bounds::new(point(px(88.0), px(10.0)), size(px(12.0), px(400.0)));
        let top = thumb_bounds(track, px(400.0), px(1600.0), px(0.0), 1.0);
        let bottom = thumb_bounds(track, px(400.0), px(1600.0), px(-1600.0), 1.0);
        assert_eq!(top.size.height, px(80.0));
        assert_eq!(top.top(), track.top());
        assert_eq!(bottom.bottom(), track.bottom());
        assert!(top.left() > track.left());
        assert!(top.right() < track.right());
        assert_eq!(thumb_bounds(track, px(400.0), px(0.0), px(0.0), 1.0).size.height, px(400.0));
        assert_eq!(pointer_offset(track, top.size.height, 0.5, px(370.0), px(1600.0)), px(-1600.0));
        assert_eq!(pointer_offset(track, top.size.height, 0.5, px(-900.0), px(1600.0)), px(0.0));
        assert_eq!(pointer_offset(track, top.size.height, 0.5, px(900.0), px(1600.0)), px(-1600.0));
        assert_eq!(pointer_offset(track, track.size.height, 0.5, px(900.0), px(0.0)), px(0.0));
    }

    #[test]
    fn heat_track_press_centers_thumb_before_drag_and_preserves_grab_position() {
        let track = gpui::Bounds::new(point(px(88.0), px(10.0)), size(px(24.0), px(400.0)));
        let thumb = thumb_bounds(track, px(400.0), px(1600.0), px(-400.0), 1.0);
        let start = track_press_offset(track, thumb, px(210.0), px(1600.0)).unwrap();
        assert_eq!(start, px(-800.0));
        let jumped = thumb_bounds(track, px(400.0), px(1600.0), start, 1.0);
        assert_eq!(jumped.top() + jumped.size.height / 2.0, px(210.0));

        let moved = pointer_offset(track, jumped.size.height, 0.5, px(250.0), px(1600.0));
        let dragged = thumb_bounds(track, px(400.0), px(1600.0), moved, 1.0);
        assert_eq!(dragged.top() + dragged.size.height / 2.0, px(250.0));
        assert_eq!(track_press_offset(track, thumb, thumb.top() + px(10.0), px(1600.0)), None);
        let grabbed = thumb_bounds(track, px(400.0), px(1600.0),
            pointer_offset(track, thumb.size.height, 0.125, thumb.top() + px(50.0), px(1600.0)), 1.0);
        assert_eq!(grabbed.top() + px(10.0), thumb.top() + px(50.0));

        assert_eq!(track_press_offset(track, thumb, track.top(), px(1600.0)), Some(px(0.0)));
        assert_eq!(track_press_offset(track, thumb, track.bottom(), px(1600.0)), Some(px(-1600.0)));
        assert_eq!(track_press_offset(track, thumb, px(500.0), px(0.0)), Some(px(0.0)));
    }

    #[test]
    fn heat_hit_testing_clamps_edges_and_matches_uneven_regions() {
        let buckets = vec![
            HeatBucket { range: 0..1, total_count: 0, max_count: 0, peak_index: 0 },
            HeatBucket { range: 1..3, total_count: 0, max_count: 0, peak_index: 1 },
            HeatBucket { range: 3..5, total_count: 0, max_count: 0, peak_index: 3 },
        ];
        let bounds = gpui::Bounds::new(point(px(0.0), px(10.0)), size(px(24.0), px(100.0)));
        assert_eq!(bucket_at(&buckets, bounds, px(-20.0), |_| None), Some(0));
        assert_eq!(bucket_at(&buckets, bounds, px(30.0), |_| None), Some(1));
        assert_eq!(bucket_at(&buckets, bounds, px(70.0), |_| None), Some(2));
        assert_eq!(bucket_at(&buckets, bounds, px(300.0), |_| None), Some(2));
        assert_eq!(bucket_at(&[], bounds, px(50.0), |_| None), None);
        assert_eq!(bucket_at(&buckets, gpui::Bounds::default(), px(50.0), |_| None), None);
    }

    #[test]
    fn heat_regions_follow_variable_row_heights_for_paint_hover_and_click() {
        let offsets = [0.0, 48.0, 96.0, 288.0, 480.0];
        let item_offset = |index| offsets.get(index).copied().map(px);
        let buckets = vec![
            HeatBucket { range: 0..2, total_count: 0, max_count: 0, peak_index: 0 },
            HeatBucket { range: 2..4, total_count: 80, max_count: 80, peak_index: 3 },
        ];
        let bounds = gpui::Bounds::new(point(px(0.0), px(10.0)), size(px(24.0), px(400.0)));
        let boundary = bounds.top() + bounds.size.height * item_ratio(2, 4, &item_offset);
        assert_eq!(boundary, px(90.0));
        assert_eq!(item_ratio(0, 4, &item_offset), 0.0);
        assert_eq!(item_ratio(4, 4, &item_offset), 1.0);
        assert_eq!(bucket_at(&buckets, bounds, boundary - px(0.1), item_offset), Some(0));
        assert_eq!(bucket_at(&buckets, bounds, boundary, item_offset), Some(1));
        assert_eq!(bucket_at(&buckets, bounds, px(210.0), item_offset), Some(1));
        let thumb = thumb_bounds(bounds, px(96.0), px(384.0), px(-96.0), 1.0);
        assert_eq!(thumb.top(), boundary);
    }

    #[test]
    fn heat_drag_keeps_pointer_anchor_when_content_height_or_track_changes() {
        let track = gpui::Bounds::new(point(px(0.0), px(10.0)), size(px(24.0), px(400.0)));
        for max_offset in [1600.0, 3200.0, 8000.0, 800.0] {
            let thumb = thumb_bounds(track, px(400.0), px(max_offset), px(0.0), 1.0);
            for grip in [0.0, 0.25, 0.5, 1.0] {
                let offset = pointer_offset(track, thumb.size.height, grip, px(210.0), px(max_offset));
                let dragged = thumb_bounds(track, px(400.0), px(max_offset), offset, 1.0);
                assert!(f32::from(dragged.top() + dragged.size.height * grip - px(210.0)).abs() < 0.001);
            }
        }
        let resized = gpui::Bounds::new(point(px(0.0), px(30.0)), size(px(24.0), px(600.0)));
        let thumb = thumb_bounds(resized, px(600.0), px(3200.0), px(0.0), 1.0);
        let offset = pointer_offset(resized, thumb.size.height, 0.25, px(210.0), px(3200.0));
        let dragged = thumb_bounds(resized, px(600.0), px(3200.0), offset, 1.0);
        assert!(f32::from(dragged.top() + dragged.size.height * 0.25 - px(210.0)).abs() < 0.001);
    }
}
