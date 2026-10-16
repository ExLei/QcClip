use super::*;

const MAX_HEAT_BUCKETS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HeatBucket {
    pub(crate) range: Range<usize>,
    pub(crate) total_count: u64,
    pub(crate) max_count: u64,
    pub(crate) peak_index: usize,
}

impl HeatBucket {
    fn new(counts: &[u64], range: Range<usize>) -> Self {
        let mut bucket = Self {
            peak_index: range.start,
            range,
            total_count: 0,
            max_count: 0,
        };
        for index in bucket.range.clone() {
            let count = counts[index];
            bucket.total_count = bucket.total_count.saturating_add(count);
            if count > bucket.max_count {
                bucket.max_count = count;
                bucket.peak_index = index;
            }
        }
        bucket
    }
}

#[derive(Default)]
pub(crate) struct ListHeatmap {
    counts: Vec<u64>,
    buckets: Rc<Vec<HeatBucket>>,
    dirty: bool,
}

impl ListHeatmap {
    pub(crate) fn replace(&mut self, counts: Vec<u64>) {
        self.counts = counts;
        self.buckets = Rc::default();
        self.dirty = true;
    }

    pub(crate) fn update(&mut self, index: usize, count: u64) {
        let Some(previous) = self.counts.get_mut(index) else { return };
        if *previous == count {
            return;
        }
        *previous = count;
        if !self.dirty {
            let slot = self.buckets.partition_point(|bucket| bucket.range.end <= index);
            if let Some(bucket) = Rc::make_mut(&mut self.buckets).get_mut(slot) {
                *bucket = HeatBucket::new(&self.counts, bucket.range.clone());
            }
        }
    }

    pub(crate) fn insert(&mut self, index: usize, count: u64) {
        self.counts.insert(index, count);
        self.dirty = true;
    }

    pub(crate) fn move_item(&mut self, from: usize, to: usize) {
        if from < to {
            self.counts[from..=to].rotate_left(1);
        } else if from > to {
            self.counts[to..=from].rotate_right(1);
        } else {
            return;
        }
        if !self.dirty {
            let start = from.min(to);
            let end = from.max(to) + 1;
            let first = self.buckets.partition_point(|bucket| bucket.range.end <= start);
            let last = self.buckets.partition_point(|bucket| bucket.range.start < end);
            for bucket in &mut Rc::make_mut(&mut self.buckets)[first..last] {
                *bucket = HeatBucket::new(&self.counts, bucket.range.clone());
            }
        }
    }

    pub(crate) fn remove_positions(&mut self, positions: &[usize]) {
        let mut removed = positions.iter().copied().peekable();
        let mut index = 0;
        self.counts.retain(|_| {
            let keep = removed.peek() != Some(&index);
            if !keep {
                removed.next();
            }
            index += 1;
            keep
        });
        self.dirty = true;
    }

    pub(crate) fn buckets(&mut self) -> Rc<Vec<HeatBucket>> {
        if self.dirty {
            let count = self.counts.len().min(MAX_HEAT_BUCKETS);
            self.buckets = Rc::new((0..count).map(|slot| {
                let start = slot * self.counts.len() / count;
                let end = (slot + 1) * self.counts.len() / count;
                HeatBucket::new(&self.counts, start..end)
            }).collect());
            self.dirty = false;
        }
        self.buckets.clone()
    }
}

#[derive(Default)]
pub(crate) struct HeatScrollbarState {
    pub(crate) bounds: gpui::Bounds<Pixels>,
    pub(crate) visible: bool,
    pub(crate) hovered_bucket: Option<usize>,
    pub(crate) dragging: bool,
    pub(crate) thumb_bounds: gpui::Bounds<Pixels>,
    // 保存按下位置和滑块内的抓取比例，行高变化后仍让滑块跟随鼠标。
    pub(crate) drag_origin: Option<(Point<Pixels>, f32)>,
    pub(crate) drag_pointer_y: Pixels,
    hovered: bool,
    last_scroll_offset: Option<Pixels>,
    hide_deadline: Option<Instant>,
    hide_task: Option<Task<()>>,
}

impl HeatScrollbarState {
    pub(crate) fn expanded(&self) -> bool {
        self.hovered || self.drag_origin.is_some()
    }

    pub(crate) fn clear_on_window_exit(&mut self) -> bool {
        let pressed = self.drag_origin.is_some();
        let changed = (!pressed && self.visible) || self.hovered || self.hovered_bucket.is_some();
        self.visible &= pressed;
        self.hovered = false;
        self.hovered_bucket = None;
        self.hide_deadline = None;
        self.hide_task = None;
        changed
    }

    fn update_scroll_offset(&mut self, offset: Pixels) -> bool {
        self.last_scroll_offset.replace(offset).is_some_and(|previous| previous != offset)
    }
}

impl MainWindow {
    pub(crate) fn set_heat_scrollbar_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.heat_scrollbar.hovered = hovered;
        if !hovered && self.heat_scrollbar.drag_origin.is_none() {
            self.heat_scrollbar.hovered_bucket = None;
        }
        cx.notify();
        if hovered {
            self.heat_scrollbar.hide_task = None;
            self.heat_scrollbar.hide_deadline = None;
            PreviewWindowService::hide(cx);
            if !self.heat_scrollbar.visible {
                self.heat_scrollbar.visible = true;
                cx.notify();
            }
        } else if self.heat_scrollbar.drag_origin.is_none() {
            self.schedule_heat_scrollbar_hide(Duration::from_millis(350), cx);
        }
    }

    pub(crate) fn observe_heat_scrollbar_scroll(&mut self, cx: &mut Context<Self>) {
        let offset = self.list_scroll_handle.as_ref().offset().y;
        if !self.heat_scrollbar.update_scroll_offset(offset) {
            return;
        }
        if !self.heat_scrollbar.visible {
            self.heat_scrollbar.visible = true;
            cx.notify();
        }
        if !self.heat_scrollbar.hovered && self.heat_scrollbar.drag_origin.is_none() {
            self.schedule_heat_scrollbar_hide(Duration::from_millis(900), cx);
        }
    }

    fn schedule_heat_scrollbar_hide(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let deadline = Instant::now() + delay;
        self.heat_scrollbar.hide_deadline = Some(self.heat_scrollbar.hide_deadline
            .map_or(deadline, |previous| previous.max(deadline)));
        if self.heat_scrollbar.hide_task.is_some() {
            return;
        }
        // 连续滚动只延长截止时间，复用同一个隐藏任务。
        self.heat_scrollbar.hide_task = Some(cx.spawn(async move |this, cx| {
            let mut delay = delay;
            loop {
                cx.background_executor().timer(delay).await;
                let next_delay = this.update(cx, |this, cx| {
                    let state = &mut this.heat_scrollbar;
                    if !state.hovered && state.drag_origin.is_none() {
                        if let Some(remaining) = state.hide_deadline
                            .and_then(|deadline| deadline.checked_duration_since(Instant::now()))
                        {
                            return Some(remaining);
                        }
                        state.visible = false;
                        state.hovered_bucket = None;
                        cx.notify();
                    }
                    this.heat_scrollbar.hide_deadline = None;
                    this.heat_scrollbar.hide_task = None;
                    None
                }).ok().flatten();
                let Some(remaining) = next_delay else { break };
                delay = remaining;
            }
        }));
    }

    pub(crate) fn navigate_heat_index(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.list_order_ids.get(index).copied() else { return };
        self.list_smooth_scroll.reset();
        PreviewWindowService::hide(cx);
        self.select_list_item_at(index, id, window, cx);
        self.list_scroll_handle.scroll_to_item(index, gpui::ScrollStrategy::Top);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_scrollbar_detects_scroll_without_showing_on_initial_layout() {
        let mut state = HeatScrollbarState::default();
        assert!(!state.update_scroll_offset(px(0.0)));
        assert!(!state.update_scroll_offset(px(0.0)));
        assert!(state.update_scroll_offset(px(-0.5)));
        assert!(!state.update_scroll_offset(px(-0.5)));
        assert!(state.update_scroll_offset(px(-400.0)));
        assert!(state.update_scroll_offset(px(0.0)));
    }

    #[test]
    fn heat_scrollbar_window_exit_clears_hover_and_pending_hide() {
        let mut state = HeatScrollbarState {
            visible: true,
            hovered: true,
            hovered_bucket: Some(3),
            hide_deadline: Some(Instant::now() + Duration::from_millis(900)),
            last_scroll_offset: Some(px(-400.0)),
            ..Default::default()
        };
        assert!(state.clear_on_window_exit());
        assert!(!state.visible);
        assert!(!state.expanded());
        assert!(state.hovered_bucket.is_none());
        assert!(!state.dragging);
        assert!(state.drag_origin.is_none());
        assert!(state.hide_deadline.is_none());
        assert!(state.hide_task.is_none());
        assert!(!state.update_scroll_offset(px(-400.0)));
        assert!(!state.clear_on_window_exit());
    }

    #[test]
    fn heat_scrollbar_window_exit_preserves_drag_until_release() {
        let mut state = HeatScrollbarState {
            visible: true,
            hovered: true,
            hovered_bucket: Some(3),
            dragging: true,
            drag_origin: Some((gpui::point(px(20.0), px(200.0)), 0.5)),
            ..Default::default()
        };
        assert!(state.clear_on_window_exit());
        assert!(state.visible && state.expanded() && state.dragging);
        assert!(state.drag_origin.is_some());
        assert!(state.hovered_bucket.is_none());
        assert!(!state.clear_on_window_exit());
        state.drag_origin = None;
        state.dragging = false;
        assert!(state.clear_on_window_exit());
        assert!(!state.visible && !state.expanded());
    }

    #[test]
    fn heatmap_covers_all_rows_and_preserves_isolated_peaks() {
        let mut map = ListHeatmap::default();
        let mut counts = vec![0; 100_003];
        counts[81_234] = 120;
        counts[100_002] = 7;
        map.replace(counts);
        let buckets = map.buckets();
        assert_eq!(buckets.len(), MAX_HEAT_BUCKETS);
        assert_eq!(buckets.first().unwrap().range.start, 0);
        assert_eq!(buckets.last().unwrap().range.end, 100_003);
        assert!(buckets.windows(2).all(|pair| pair[0].range.end == pair[1].range.start));
        let hot = buckets.iter().find(|bucket| bucket.range.contains(&81_234)).unwrap();
        assert_eq!((hot.peak_index, hot.max_count, hot.total_count), (81_234, 120, 120));
        assert!(Rc::ptr_eq(&buckets, &map.buckets()));
    }

    #[test]
    fn heatmap_updates_one_region_without_mutating_previous_snapshot() {
        let mut map = ListHeatmap::default();
        map.replace(vec![0; 1_000]);
        let before = map.buckets();
        map.update(521, 32);
        let after = map.buckets();
        let slot = after.iter().position(|bucket| bucket.range.contains(&521)).unwrap();
        assert_eq!(after[slot].max_count, 32);
        assert_eq!(before[slot].max_count, 0);
        assert!(before.iter().zip(after.iter()).enumerate()
            .all(|(index, (left, right))| index == slot || left == right));
        map.update(521, 0);
        assert_eq!(map.buckets()[slot].max_count, 0);
    }

    #[test]
    fn heatmap_tracks_reordering_insertions_and_bulk_deletion() {
        let mut map = ListHeatmap::default();
        map.replace(vec![1, 2, 3, 4]);
        map.move_item(0, 3);
        map.move_item(2, 0);
        map.insert(2, 8);
        map.remove_positions(&[1, 4]);
        assert_eq!(map.counts, vec![4, 8, 3]);
        assert_eq!(map.buckets().iter().map(|bucket| bucket.max_count).collect::<Vec<_>>(), vec![4, 8, 3]);
        map.replace(Vec::new());
        assert!(map.buckets().is_empty());
    }

    #[test]
    fn heatmap_reordering_rebuilds_only_intersecting_cached_regions() {
        let mut map = ListHeatmap::default();
        map.replace((0..4096).map(|index| (index % 13) as u64).collect());
        let before = map.buckets();
        let snapshot = before.as_ref().clone();
        for (from, to) in [(521, 527), (527, 510), (510, 521)] {
            map.move_item(from, to);
            assert!(!map.dirty);
            let after = map.buckets();
            for (slot, bucket) in after.iter().enumerate() {
                assert_eq!(*bucket, HeatBucket::new(&map.counts, bucket.range.clone()));
                if bucket.range.end <= 510 || bucket.range.start > 527 {
                    assert_eq!(*bucket, before[slot]);
                }
            }
        }
        assert_eq!(*before, snapshot);
    }

    #[test]
    fn heatmap_uses_first_peak_on_ties_and_saturates_totals() {
        let bucket = HeatBucket::new(&[4, 9, 9], 0..3);
        assert_eq!((bucket.peak_index, bucket.total_count), (1, 22));
        let bucket = HeatBucket::new(&[u64::MAX, 1], 0..2);
        assert_eq!(bucket.total_count, u64::MAX);
        assert_eq!(HeatBucket::new(&[0, 0], 0..2).peak_index, 0);
    }
}
