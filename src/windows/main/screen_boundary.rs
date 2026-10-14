use gpui::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MonitorBoundary {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) left_edge: bool,
    pub(crate) right_edge: bool,
    pub(crate) top_edge: bool,
    pub(crate) bottom_edge: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ScreenBoundaries {
    monitors: Vec<MonitorBoundary>,
    virtual_left: i32,
    virtual_top: i32,
    virtual_right: i32,
    virtual_bottom: i32,
}

impl ScreenBoundaries {
    pub(crate) fn capture() -> Self {
        #[cfg(target_os = "windows")]
        {
            capture_windows()
        }

        #[cfg(not(target_os = "windows"))]
        {
            Self::default()
        }
    }

    pub(crate) fn is_available(&self) -> bool {
        !self.monitors.is_empty()
    }

    pub(crate) fn capture_for_app(cx: &App) -> Self {
        #[cfg(target_os = "windows")]
        {
            let _ = cx;
            Self::capture()
        }

        #[cfg(not(target_os = "windows"))]
        {
            let monitors = cx
                .displays()
                .into_iter()
                .map(|display| {
                    let bounds = display.bounds();
                    (
                        f32::from(bounds.origin.x).round() as i32,
                        f32::from(bounds.origin.y).round() as i32,
                        f32::from(bounds.size.width).round() as i32,
                        f32::from(bounds.size.height).round() as i32,
                    )
                })
                .collect();
            build_boundaries(monitors)
        }
    }

    pub(crate) fn monitor_for_point(&self, x: i32, y: i32) -> Option<MonitorBoundary> {
        self.monitors.iter().copied().find(|monitor| {
            x >= monitor.x
                && x < monitor.x.saturating_add(monitor.width)
                && y >= monitor.y
                && y < monitor.y.saturating_add(monitor.height)
        })
    }

    pub(crate) fn constrain_client_position(
        &self,
        client_x: i32,
        client_y: i32,
        client_width: i32,
        client_height: i32,
    ) -> (i32, i32) {
        let width = client_width.max(1);
        let height = client_height.max(1);
        let center_x = client_x.saturating_add(width / 2);
        let center_y = client_y.saturating_add(height / 2);

        if let Some(monitor) = self.monitors.iter().find(|monitor| {
            center_x >= monitor.x
                && center_x < monitor.x.saturating_add(monitor.width)
                && center_y >= monitor.y
                && center_y < monitor.y.saturating_add(monitor.height)
        }) {
            let mut constrained_x = client_x;
            let mut constrained_y = client_y;
            let monitor_right = monitor.x.saturating_add(monitor.width);
            let monitor_bottom = monitor.y.saturating_add(monitor.height);

            if monitor.width >= width {
                if monitor.left_edge {
                    constrained_x = constrained_x.max(monitor.x);
                }
                if monitor.right_edge {
                    constrained_x = constrained_x.min(monitor_right - width);
                }
            }
            if monitor.height >= height {
                if monitor.top_edge {
                    constrained_y = constrained_y.max(monitor.y);
                }
                if monitor.bottom_edge {
                    constrained_y = constrained_y.min(monitor_bottom - height);
                }
            }
            return (constrained_x, constrained_y);
        }

        if self.is_available() {
            (
                client_x.clamp(self.virtual_left, self.virtual_right.saturating_sub(width)),
                client_y.clamp(self.virtual_top, self.virtual_bottom.saturating_sub(height)),
            )
        } else {
            (client_x, client_y)
        }
    }
}

impl MonitorBoundary {
    pub(crate) fn physical_bounds(&self) -> crate::platform::window::PhysicalDisplayBounds {
        crate::platform::window::PhysicalDisplayBounds {
            x: self.x as f32,
            y: self.y as f32,
            width: self.width as f32,
            height: self.height as f32,
        }
    }
}

#[cfg(target_os = "windows")]
fn capture_windows() -> ScreenBoundaries {
    use windows::Win32::{Foundation::LPARAM, Graphics::Gdi::EnumDisplayMonitors};

    let mut rectangles = Vec::new();
    // SAFETY: EnumDisplayMonitors 只读取当前显示器布局，回调中的 LPARAM 指向本次调用栈上的 Vec。
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(monitor_enum_proc),
            LPARAM(&mut rectangles as *mut _ as isize),
        )
    };

    build_boundaries(rectangles)
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn monitor_enum_proc(
    monitor: windows::Win32::Graphics::Gdi::HMONITOR,
    _dc: windows::Win32::Graphics::Gdi::HDC,
    _rect: *mut windows::Win32::Foundation::RECT,
    data: windows::Win32::Foundation::LPARAM,
) -> windows::core::BOOL {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};

    let rectangles = unsafe { &mut *(data.0 as *mut Vec<(i32, i32, i32, i32)>) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: monitor 由 EnumDisplayMonitors 提供，info 是有效的可写结构体。
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        rectangles.push((
            info.rcMonitor.left,
            info.rcMonitor.top,
            info.rcMonitor.right - info.rcMonitor.left,
            info.rcMonitor.bottom - info.rcMonitor.top,
        ));
    }
    windows::core::BOOL(1)
}

fn build_boundaries(rectangles: Vec<(i32, i32, i32, i32)>) -> ScreenBoundaries {
    const TOLERANCE: i32 = 5;

    let monitors: Vec<_> = rectangles
        .iter()
        .copied()
        .filter(|(_, _, width, height)| *width > 0 && *height > 0)
        .map(|(x, y, width, height)| {
            let right = x.saturating_add(width);
            let bottom = y.saturating_add(height);
            let mut left_edge = true;
            let mut right_edge = true;
            let mut top_edge = true;
            let mut bottom_edge = true;

            for (other_x, other_y, other_width, other_height) in rectangles.iter().copied() {
                if (x, y, width, height) == (other_x, other_y, other_width, other_height) {
                    continue;
                }
                let other_right = other_x.saturating_add(other_width);
                let other_bottom = other_y.saturating_add(other_height);
                if (other_right - x).abs() <= TOLERANCE && other_y < bottom && other_bottom > y {
                    left_edge = false;
                }
                if (other_x - right).abs() <= TOLERANCE && other_y < bottom && other_bottom > y {
                    right_edge = false;
                }
                if (other_bottom - y).abs() <= TOLERANCE && other_x < right && other_right > x {
                    top_edge = false;
                }
                if (other_y - bottom).abs() <= TOLERANCE && other_x < right && other_right > x {
                    bottom_edge = false;
                }
            }

            MonitorBoundary {
                x,
                y,
                width,
                height,
                left_edge,
                right_edge,
                top_edge,
                bottom_edge,
            }
        })
        .collect();

    let virtual_left = monitors.iter().map(|monitor| monitor.x).min().unwrap_or(0);
    let virtual_top = monitors.iter().map(|monitor| monitor.y).min().unwrap_or(0);
    let virtual_right = monitors
        .iter()
        .map(|monitor| monitor.x.saturating_add(monitor.width))
        .max()
        .unwrap_or(0);
    let virtual_bottom = monitors
        .iter()
        .map(|monitor| monitor.y.saturating_add(monitor.height))
        .max()
        .unwrap_or(0);

    ScreenBoundaries {
        monitors,
        virtual_left,
        virtual_top,
        virtual_right,
        virtual_bottom,
    }
}

#[cfg(test)]
mod tests {
    use super::build_boundaries;

    #[test]
    fn adjacent_monitors_do_not_create_internal_drag_boundaries() {
        let boundaries = build_boundaries(vec![(0, 0, 1920, 1080), (1920, 0, 1920, 1080)]);

        assert!(!boundaries.monitors[0].right_edge);
        assert!(!boundaries.monitors[1].left_edge);
        assert_eq!(
            boundaries.constrain_client_position(1900, 100, 400, 400),
            (1900, 100)
        );
    }

    #[test]
    fn outer_virtual_edges_keep_window_inside_the_desktop() {
        let boundaries = build_boundaries(vec![(0, 0, 1920, 1080)]);

        assert_eq!(
            boundaries.constrain_client_position(-100, -50, 400, 300),
            (0, 0)
        );
        assert_eq!(
            boundaries.constrain_client_position(1800, 900, 400, 300),
            (1520, 780)
        );
    }
}
