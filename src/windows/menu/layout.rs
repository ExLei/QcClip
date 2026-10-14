use super::model::{MenuItemKind, MenuModel, MenuPosition, RootMenuPlacement};

pub(super) const MENU_ITEM_HEIGHT: f32 = 30.0;
pub(super) const MENU_SEPARATOR_HEIGHT: f32 = 9.0;
pub(super) const MENU_MIN_WIDTH: f32 = 100.0;
pub(super) const MENU_PADDING_Y: f32 = 5.0;
pub(super) const MENU_SCREEN_MARGIN: f32 = 6.0;
pub(super) const MENU_MAX_HEIGHT_RATIO: f32 = 0.9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MenuRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl MenuRect {
    pub(super) fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width: width.max(1.0),
            height: height.max(1.0),
        }
    }

    pub(super) fn right(self) -> f32 {
        self.x + self.width
    }

    pub(super) fn bottom(self) -> f32 {
        self.y + self.height
    }
}

pub(super) fn max_menu_height(screen: MenuRect) -> f32 {
    (screen.height * MENU_MAX_HEIGHT_RATIO).max(1.0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MenuDirection {
    Left,
    Right,
}

impl MenuDirection {
    fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

pub(super) fn fit_root_bounds(
    anchor: MenuPosition,
    size: (f32, f32),
    screen: MenuRect,
) -> MenuRect {
    match anchor.root_placement {
        RootMenuPlacement::AtPoint => {
            fit_rect_to_screen(MenuRect::new(anchor.x, anchor.y, size.0, size.1), screen)
        }
        RootMenuPlacement::Tray => {
            let candidates = tray_root_candidates(anchor, size);
            if let Some(candidate) = candidates
                .into_iter()
                .find(|candidate| rect_fits_screen(*candidate, screen))
            {
                return candidate;
            }
            fit_rect_to_screen(candidates[0], screen)
        }
    }
}

fn tray_root_candidates(anchor: MenuPosition, size: (f32, f32)) -> [MenuRect; 4] {
    [
        // 右上：菜单的左下角贴住鼠标点。
        MenuRect::new(anchor.x, anchor.y - size.1, size.0, size.1),
        // 左上：菜单的右下角贴住鼠标点。
        MenuRect::new(anchor.x - size.0, anchor.y - size.1, size.0, size.1),
        // 右下：菜单的左上角贴住鼠标点。
        MenuRect::new(anchor.x, anchor.y, size.0, size.1),
        // 左下：菜单的右上角贴住鼠标点。
        MenuRect::new(anchor.x - size.0, anchor.y, size.0, size.1),
    ]
}

pub(super) fn fit_submenu_bounds(
    parent: MenuRect,
    parent_menu: &MenuModel,
    item_index: usize,
    size: (f32, f32),
    screen: MenuRect,
    scale_factor: f32,
    scroll_offset_y: f32,
    preferred_direction: MenuDirection,
) -> (MenuRect, MenuDirection) {
    let scale_factor = scale_factor.max(1.0);
    let item_bounds = submenu_item_bounds(
        parent,
        parent_menu,
        item_index,
        scale_factor,
        scroll_offset_y,
    );
    let right_x = parent.right() - scale_factor;
    let left_x = parent.x - size.0 + scale_factor;
    let direction = if submenu_fits(
        preferred_direction,
        left_x,
        right_x,
        size.0,
        screen,
        scale_factor,
    ) {
        preferred_direction
    } else {
        preferred_direction.opposite()
    };
    let x = match direction {
        MenuDirection::Left => left_x,
        MenuDirection::Right => right_x,
    };
    (
        fit_rect_to_screen(
            MenuRect::new(
                x,
                item_bounds.y,
                size.0,
                size.1,
            ),
            screen,
        ),
        direction,
    )
}

pub(super) fn submenu_item_bounds(
    parent: MenuRect,
    parent_menu: &MenuModel,
    item_index: usize,
    scale_factor: f32,
    scroll_offset_y: f32,
) -> MenuRect {
    let scale_factor = scale_factor.max(1.0);
    MenuRect::new(
        parent.x,
        parent.y
            + menu_item_top(parent_menu, item_index) * scale_factor
            + scroll_offset_y * scale_factor
            - MENU_PADDING_Y * scale_factor,
        parent.width,
        MENU_ITEM_HEIGHT * scale_factor,
    )
}

pub(super) fn menu_item_top(menu: &MenuModel, index: usize) -> f32 {
    MENU_PADDING_Y
        + menu
            .items()
            .iter()
            .take(index)
            .map(|item| match item.kind {
                MenuItemKind::Separator => MENU_SEPARATOR_HEIGHT,
                MenuItemKind::Action | MenuItemKind::Submenu => MENU_ITEM_HEIGHT,
            })
            .sum::<f32>()
}

fn submenu_fits(
    direction: MenuDirection,
    left_x: f32,
    right_x: f32,
    width: f32,
    screen: MenuRect,
    scale_factor: f32,
) -> bool {
    let x = match direction {
        MenuDirection::Left => left_x,
        MenuDirection::Right => right_x,
    };
    x >= screen.x + MENU_SCREEN_MARGIN * scale_factor
        && x + width <= screen.right() - MENU_SCREEN_MARGIN * scale_factor
}

pub(super) fn fit_rect_to_screen(rect: MenuRect, screen: MenuRect) -> MenuRect {
    let x = clamp_axis(
        rect.x,
        screen.x + MENU_SCREEN_MARGIN,
        screen.right() - rect.width - MENU_SCREEN_MARGIN,
    );
    let y = clamp_axis(
        rect.y,
        screen.y + MENU_SCREEN_MARGIN,
        screen.bottom() - rect.height - MENU_SCREEN_MARGIN,
    );
    MenuRect::new(x, y, rect.width, rect.height)
}

fn rect_fits_screen(rect: MenuRect, screen: MenuRect) -> bool {
    rect.x >= screen.x + MENU_SCREEN_MARGIN
        && rect.y >= screen.y + MENU_SCREEN_MARGIN
        && rect.right() <= screen.right() - MENU_SCREEN_MARGIN
        && rect.bottom() <= screen.bottom() - MENU_SCREEN_MARGIN
}

fn clamp_axis(value: f32, min: f32, max: f32) -> f32 {
    if min > max {
        min
    } else {
        value.clamp(min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows::menu::{MenuActionId, MenuItem};

    #[test]
    fn root_menu_is_clamped_to_work_area() {
        let bounds = fit_root_bounds(
            MenuPosition::new(1198.0, 898.0, 1.0),
            (140.0, 100.0),
            MenuRect::new(0.0, 0.0, 1200.0, 900.0),
        );

        assert_eq!(bounds.x, 1054.0);
        assert_eq!(bounds.y, 794.0);
        assert!(bounds.right() <= 1194.0);
        assert!(bounds.bottom() <= 894.0);
    }

    #[test]
    fn submenu_flips_left_when_right_space_is_insufficient() {
        let parent = MenuRect::new(112.0, 20.0, 132.0, 80.0);
        let menu = MenuModel::new(vec![
            MenuItem::action(MenuActionId::new("first"), "First"),
            MenuItem::action(MenuActionId::new("second"), "Second"),
        ]);

        let (bounds, direction) = fit_submenu_bounds(
            parent,
            &menu,
            1,
            (168.0, 40.0),
            MenuRect::new(0.0, 0.0, 260.0, 200.0),
            1.0,
            0.0,
            MenuDirection::Right,
        );

        assert_eq!(direction, MenuDirection::Left);
        assert!(bounds.x < parent.x);
        assert_eq!(bounds.y, 50.0);
    }

    #[test]
    fn menu_item_top_position_includes_separator_height() {
        let menu = MenuModel::new(vec![
            MenuItem::action(MenuActionId::new("first"), "First"),
            MenuItem::separator(),
            MenuItem::action(MenuActionId::new("second"), "Second"),
        ]);

        assert_eq!(
            menu_item_top(&menu, 2),
            MENU_PADDING_Y + MENU_ITEM_HEIGHT + MENU_SEPARATOR_HEIGHT
        );
    }

    #[test]
    fn submenu_aligns_with_parent_first_row_using_scale() {
        let menu = MenuModel::new(vec![
            MenuItem::action(MenuActionId::new("first"), "First"),
            MenuItem::action(MenuActionId::new("second"), "Second"),
        ]);

        let (bounds, _) = fit_submenu_bounds(
            MenuRect::new(200.0, 20.0, 132.0, 80.0),
            &menu,
            1,
            (168.0, 40.0),
            MenuRect::new(0.0, 0.0, 800.0, 600.0),
            1.25,
            0.0,
            MenuDirection::Right,
        );

        assert_eq!(bounds.y, 57.5);
        assert_eq!(bounds.x, 330.75);
    }

    #[test]
    fn menu_max_height_is_ninety_percent_of_screen_height() {
        assert_eq!(
            max_menu_height(MenuRect::new(0.0, 0.0, 1200.0, 900.0)),
            810.0
        );
    }

    #[test]
    fn submenu_position_accounts_for_parent_scroll_offset() {
        let menu = MenuModel::new(vec![
            MenuItem::action(MenuActionId::new("first"), "First"),
            MenuItem::action(MenuActionId::new("second"), "Second"),
        ]);

        let (bounds, _) = fit_submenu_bounds(
            MenuRect::new(200.0, 20.0, 132.0, 80.0),
            &menu,
            1,
            (168.0, 40.0),
            MenuRect::new(0.0, 0.0, 800.0, 600.0),
            1.0,
            -20.0,
            MenuDirection::Right,
        );

        assert_eq!(bounds.y, 30.0);
    }
}
