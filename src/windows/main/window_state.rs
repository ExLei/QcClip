use crate::platform::window as platform_window;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeSide {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct EdgeAttachment {
    pub(crate) side: EdgeSide,
    pub(crate) client_bounds: platform_window::PhysicalClientBounds,
    pub(crate) screen_bounds: platform_window::PhysicalDisplayBounds,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DragState {
    #[default]
    Idle,
    NativeMove,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MouseLeaveState {
    #[default]
    Disarmed,
    Armed,
}

#[derive(Clone, Copy, Debug)]
enum WindowPresentation {
    Normal {
        visible: bool,
    },
    Edge {
        attachment: EdgeAttachment,
        visible: bool,
    },
}

impl Default for WindowPresentation {
    fn default() -> Self {
        Self::Normal { visible: true }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct State {
    presentation: WindowPresentation,
    drag_state: DragState,
    drag_generation: u64,
    visibility_transition_generation: u64,
    visibility_transition_active: bool,
    normal_bounds: Option<platform_window::PhysicalClientBounds>,
    normal_logical_size: Option<(f32, f32)>,
    mouse_leave_state: MouseLeaveState,
    edge_reveal_suppressed: bool,
    edge_reveal_blocked: bool,
    fixed: bool,
}

impl State {
    pub(crate) fn reset(&mut self) {
        self.presentation = WindowPresentation::default();
        self.drag_state = DragState::Idle;
        self.drag_generation = self.drag_generation.wrapping_add(1);
        self.visibility_transition_generation =
            self.visibility_transition_generation.wrapping_add(1);
        self.visibility_transition_active = false;
        self.normal_bounds = None;
        self.normal_logical_size = None;
        self.mouse_leave_state = MouseLeaveState::Disarmed;
        self.edge_reveal_suppressed = false;
        self.edge_reveal_blocked = false;
        self.fixed = false;
    }

    pub(crate) fn begin_drag(&mut self) {
        // 离开贴边位置后，只有释放时重新贴边才恢复 Edge 状态。
        self.cancel_visibility_transition();
        self.presentation = WindowPresentation::Normal { visible: true };
        self.drag_state = DragState::NativeMove;
        self.drag_generation = self.drag_generation.wrapping_add(1);
        self.mouse_leave_state = MouseLeaveState::Disarmed;
    }

    pub(crate) fn finish_drag(&mut self) {
        self.drag_state = DragState::Idle;
    }

    pub(crate) fn invalidate_drag(&mut self) {
        self.drag_state = DragState::Idle;
        self.drag_generation = self.drag_generation.wrapping_add(1);
        self.mouse_leave_state = MouseLeaveState::Disarmed;
    }

    pub(crate) fn disable_edge_mode(&mut self) {
        self.cancel_visibility_transition();
        self.detach_edge();
        self.invalidate_drag();
    }

    pub(crate) fn drag_state(&self) -> DragState {
        self.drag_state
    }

    pub(crate) fn drag_generation(&self) -> u64 {
        self.drag_generation
    }

    pub(crate) fn mouse_leave_state(&self) -> MouseLeaveState {
        self.mouse_leave_state
    }

    pub(crate) fn set_mouse_leave_state(&mut self, state: MouseLeaveState) {
        self.mouse_leave_state = state;
    }

    pub(crate) fn edge_reveal_suppressed(&self) -> bool {
        self.edge_reveal_suppressed
    }

    pub(crate) fn set_edge_reveal_suppressed(&mut self, suppressed: bool) {
        self.edge_reveal_suppressed = suppressed;
    }

    pub(crate) fn edge_reveal_blocked(&self) -> bool {
        self.edge_reveal_blocked
    }

    pub(crate) fn set_edge_reveal_blocked(&mut self, blocked: bool) {
        self.edge_reveal_blocked = blocked;
    }

    pub(crate) fn is_fixed(&self) -> bool {
        self.fixed
    }

    pub(crate) fn set_fixed(&mut self, fixed: bool) {
        self.fixed = fixed;
    }

    pub(crate) fn edge_attachment(&self) -> Option<EdgeAttachment> {
        match self.presentation {
            WindowPresentation::Edge { attachment, .. } => Some(attachment),
            WindowPresentation::Normal { .. } => None,
        }
    }

    /// 只有“贴边 + 逻辑可见 + 没有正在进行的显示/隐藏动画”时，窗口当前坐标才代表贴边可见位置。
    pub(crate) fn can_record_edge_bounds(&self) -> bool {
        self.edge_attachment().is_some() && self.can_record_window_geometry()
    }

    pub(crate) fn can_record_window_geometry(&self) -> bool {
        self.is_logically_visible() && !self.visibility_transition_active
    }

    pub(crate) fn is_logically_visible(&self) -> bool {
        match self.presentation {
            WindowPresentation::Normal { visible } | WindowPresentation::Edge { visible, .. } => {
                visible
            }
        }
    }

    pub(crate) fn set_visibility(&mut self, visible: bool) {
        self.presentation = match self.presentation {
            WindowPresentation::Normal { .. } => WindowPresentation::Normal { visible },
            WindowPresentation::Edge { attachment, .. } => WindowPresentation::Edge {
                attachment,
                visible,
            },
        };
    }

    pub(crate) fn begin_visibility_transition(&mut self, visible: bool) -> u64 {
        self.visibility_transition_generation =
            self.visibility_transition_generation.wrapping_add(1);
        self.visibility_transition_active = true;
        self.set_visibility(visible);
        self.visibility_transition_generation
    }

    pub(crate) fn is_visibility_transition_active(&self) -> bool {
        self.visibility_transition_active
    }

    pub(crate) fn visibility_transition_generation(&self) -> u64 {
        self.visibility_transition_generation
    }

    pub(crate) fn is_current_visibility_transition(&self, generation: u64) -> bool {
        self.visibility_transition_active && self.visibility_transition_generation == generation
    }

    pub(crate) fn finish_visibility_transition(&mut self, generation: u64) -> bool {
        if !self.is_current_visibility_transition(generation) {
            return false;
        }
        self.visibility_transition_active = false;
        true
    }

    pub(crate) fn cancel_visibility_transition(&mut self) {
        self.visibility_transition_generation =
            self.visibility_transition_generation.wrapping_add(1);
        self.visibility_transition_active = false;
    }

    pub(crate) fn normal_bounds(&self) -> Option<platform_window::PhysicalClientBounds> {
        self.normal_bounds
    }

    pub(crate) fn normal_logical_size(&self) -> Option<(f32, f32)> {
        self.normal_logical_size
    }

    pub(crate) fn remember_normal_bounds(&mut self, bounds: platform_window::PhysicalClientBounds) {
        self.normal_bounds = Some(bounds);
    }

    pub(crate) fn remember_normal_geometry(
        &mut self,
        bounds: platform_window::PhysicalClientBounds,
        logical_size: (f32, f32),
    ) {
        self.normal_bounds = Some(bounds);
        self.normal_logical_size = Some(logical_size);
    }

    pub(crate) fn attach_edge(&mut self, attachment: EdgeAttachment) {
        let visible = self.is_logically_visible();
        self.cancel_visibility_transition();
        self.presentation = WindowPresentation::Edge {
            attachment,
            visible,
        };
        self.mouse_leave_state = MouseLeaveState::Armed;
        self.edge_reveal_suppressed = false;
    }

    pub(crate) fn update_edge_bounds(
        &mut self,
        client_bounds: platform_window::PhysicalClientBounds,
        screen_bounds: platform_window::PhysicalDisplayBounds,
    ) {
        if let WindowPresentation::Edge {
            attachment,
            visible,
        } = self.presentation
        {
            self.presentation = WindowPresentation::Edge {
                attachment: EdgeAttachment {
                    side: attachment.side,
                    client_bounds,
                    screen_bounds,
                },
                visible,
            };
        }
    }

    pub(crate) fn detach_edge(&mut self) {
        self.presentation = match self.presentation {
            WindowPresentation::Normal { visible } => WindowPresentation::Normal { visible },
            WindowPresentation::Edge { .. } => WindowPresentation::Normal { visible: true },
        };
        self.mouse_leave_state = MouseLeaveState::Disarmed;
        self.edge_reveal_suppressed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment() -> EdgeAttachment {
        EdgeAttachment {
            side: EdgeSide::Left,
            client_bounds: platform_window::PhysicalClientBounds {
                left: 0,
                top: 100,
                right: 360,
                bottom: 620,
            },
            screen_bounds: platform_window::PhysicalDisplayBounds {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
        }
    }

    #[test]
    fn edge_attachment_controls_all_visibility_transitions() {
        let mut state = State::default();
        state.attach_edge(attachment());

        assert!(state.edge_attachment().is_some());
        assert!(state.is_logically_visible());

        state.set_visibility(false);
        assert!(!state.is_logically_visible());
        assert!(state.edge_attachment().is_some());

        state.set_visibility(true);
        assert!(state.is_logically_visible());
    }

    #[test]
    fn detaching_edge_preserves_a_normal_hidden_window() {
        let mut state = State::default();
        state.set_visibility(false);
        state.detach_edge();

        assert!(state.edge_attachment().is_none());
        assert!(!state.is_logically_visible());
    }

    #[test]
    fn updating_edge_bounds_preserves_side_and_visibility() {
        let mut state = State::default();
        state.attach_edge(attachment());
        state.set_visibility(false);

        let client_bounds = platform_window::PhysicalClientBounds {
            left: 0,
            top: 120,
            right: 420,
            bottom: 760,
        };
        state.update_edge_bounds(
            client_bounds,
            platform_window::PhysicalDisplayBounds {
                x: 0.0,
                y: 0.0,
                width: 2560.0,
                height: 1440.0,
            },
        );

        let attachment = state.edge_attachment().expect("贴边状态应当保留");
        assert_eq!(attachment.side, EdgeSide::Left);
        assert_eq!(attachment.client_bounds, client_bounds);
        assert_eq!(attachment.screen_bounds.width, 2560.0);
        assert!(!state.is_logically_visible());
    }

    #[test]
    fn newer_visibility_transition_invalidates_the_previous_one() {
        let mut state = State::default();
        let first = state.begin_visibility_transition(false);
        let second = state.begin_visibility_transition(true);

        assert_ne!(first, second);
        assert!(!state.finish_visibility_transition(first));
        assert!(state.finish_visibility_transition(second));
        assert!(state.is_logically_visible());
        assert!(!state.is_visibility_transition_active());
    }

    #[test]
    fn normal_geometry_is_only_recorded_while_visible_and_idle() {
        let mut state = State::default();
        assert!(state.can_record_window_geometry());
        let hide = state.begin_visibility_transition(false);
        assert!(!state.can_record_window_geometry());
        assert!(state.finish_visibility_transition(hide));
        assert!(!state.can_record_window_geometry());
        let show = state.begin_visibility_transition(true);
        assert!(!state.can_record_window_geometry());
        assert!(state.finish_visibility_transition(show));
        assert!(state.can_record_window_geometry());
    }

    #[test]
    fn reversing_visibility_preserves_the_full_normal_geometry() {
        let mut state = State::default();
        let bounds = attachment().client_bounds;
        state.remember_normal_geometry(bounds, (360.0, 520.0));
        let hide = state.begin_visibility_transition(false);
        let show = state.begin_visibility_transition(true);
        assert!(!state.finish_visibility_transition(hide));
        assert!(!state.can_record_window_geometry());
        assert_eq!(state.normal_bounds(), Some(bounds));
        assert_eq!(state.normal_logical_size(), Some((360.0, 520.0)));
        assert!(state.finish_visibility_transition(show));
        assert!(state.can_record_window_geometry());
    }

    #[test]
    fn edge_bounds_are_only_recorded_while_visible_and_idle() {
        let mut state = State::default();
        state.attach_edge(attachment());
        state.set_visibility(false);

        assert!(!state.can_record_edge_bounds());

        state.set_visibility(true);
        assert!(state.can_record_edge_bounds());

        state.begin_visibility_transition(false);
        assert!(!state.can_record_edge_bounds());
    }

    #[test]
    fn edge_reveal_suppression_is_cleared_when_the_edge_is_attached_again() {
        let mut state = State::default();
        assert!(!state.edge_reveal_suppressed());

        state.set_edge_reveal_suppressed(true);
        assert!(state.edge_reveal_suppressed());

        state.attach_edge(attachment());
        assert!(!state.edge_reveal_suppressed());

        state.set_edge_reveal_suppressed(true);
        state.reset();
        assert!(!state.edge_reveal_suppressed());
    }
}
