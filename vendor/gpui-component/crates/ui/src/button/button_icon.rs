use crate::{Icon, Sizable, Size, progress::ProgressCircle, spinner::Spinner};
use gpui::{
    App, IntoElement, Motion, ParentElement, RenderOnce, SharedString, Styled, Window, div,
    prelude::FluentBuilder,
};

/// Button icon which can be an Icon, Spinner, or Progress use for `icon` method of Button.
#[doc(hidden)]
#[derive(IntoElement)]
pub struct ButtonIcon {
    icon: ButtonIconVariant,
    loading_icon: Option<Icon>,
    loading: bool,
    size: Size,
    hover_animation: Option<(SharedString, f32, Motion)>,
}

impl<T> From<T> for ButtonIcon
where
    T: Into<ButtonIconVariant>,
{
    fn from(icon: T) -> Self {
        ButtonIcon::new(icon)
    }
}

impl ButtonIcon {
    /// Creates a new ButtonIcon with the given icon.
    pub fn new(icon: impl Into<ButtonIconVariant>) -> Self {
        Self {
            icon: icon.into(),
            loading_icon: None,
            loading: false,
            size: Size::Medium,
            hover_animation: None,
        }
    }

    pub(crate) fn loading_icon(mut self, icon: Option<Icon>) -> Self {
        self.loading_icon = icon;
        self
    }

    pub(crate) fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub(crate) fn hover_animation(
        mut self,
        group: impl Into<SharedString>,
        scale: f32,
        motion: Motion,
    ) -> Self {
        self.hover_animation = Some((group.into(), scale, motion));
        self
    }
}

impl Sizable for ButtonIcon {
    fn with_size(mut self, size: impl Into<crate::Size>) -> Self {
        self.size = size.into();
        self
    }
}

/// Button icon which can be an Icon, Spinner, Progress, or ProgressCircle use for `icon` method of Button.
#[doc(hidden)]
#[derive(IntoElement)]
pub enum ButtonIconVariant {
    Icon(Icon),
    Spinner(Spinner),
    Progress(ProgressCircle),
}

impl<T> From<T> for ButtonIconVariant
where
    T: Into<Icon>,
{
    fn from(icon: T) -> Self {
        Self::Icon(icon.into())
    }
}

impl From<Spinner> for ButtonIconVariant {
    fn from(spinner: Spinner) -> Self {
        Self::Spinner(spinner)
    }
}

impl From<ProgressCircle> for ButtonIconVariant {
    fn from(progress: ProgressCircle) -> Self {
        Self::Progress(progress)
    }
}

impl ButtonIconVariant {
    /// Returns true if the ButtonIconKind is an Icon.
    #[inline]
    pub(crate) fn is_spinner(&self) -> bool {
        matches!(self, Self::Spinner(_))
    }

    /// Returns true if the ButtonIconKind is a Progress or ProgressCircle.
    #[inline]
    pub(crate) fn is_progress(&self) -> bool {
        matches!(self, Self::Progress(_))
    }
}

impl Sizable for ButtonIconVariant {
    fn with_size(self, size: impl Into<crate::Size>) -> Self {
        match self {
            Self::Icon(icon) => Self::Icon(icon.with_size(size)),
            Self::Spinner(spinner) => Self::Spinner(spinner.with_size(size)),
            Self::Progress(progress) => Self::Progress(progress.with_size(size)),
        }
    }
}

impl ButtonIconVariant {
    pub(crate) fn hover_animation(
        self,
        group: SharedString,
        scale: f32,
        motion: Motion,
    ) -> Self {
        match self {
            Self::Icon(icon) => Self::Icon(icon.with_hover_animation(group, scale, motion)),
            other => other,
        }
    }
}

impl RenderOnce for ButtonIconVariant {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        match self {
            Self::Icon(icon) => icon.into_any_element(),
            Self::Spinner(spinner) => spinner.into_any_element(),
            Self::Progress(progress) => progress.into_any_element(),
        }
    }
}

impl RenderOnce for ButtonIcon {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        if self.loading {
            if self.icon.is_spinner() || self.icon.is_progress() {
                self.icon.with_size(self.size).into_any_element()
            } else {
                Spinner::new()
                    .when_some(self.loading_icon, |this, icon| this.icon(icon))
                    .with_size(self.size)
                    .into_any_element()
            }
        } else {
            let icon = self.icon.with_size(self.size);
            let icon = if let Some((group, scale, motion)) = self.hover_animation {
                icon.hover_animation(group, scale, motion)
            } else {
                icon
            };
            let icon_size = match self.size {
                Size::Size(value) => value,
                Size::XSmall => window.rem_size() * 0.75,
                Size::Small => window.rem_size() * 0.875,
                Size::Medium => window.rem_size(),
                Size::Large => window.rem_size() * 1.5,
            };
            div()
                .size(icon_size)
                .flex()
                .items_center()
                .justify_center()
                .child(icon)
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IconName;

    #[gpui::test]
    fn test_button_icon_builder(_cx: &mut gpui::TestAppContext) {
        let custom_icon = Icon::new(IconName::Loader);
        let icon = ButtonIcon::new(IconName::Plus)
            .loading(true)
            .loading_icon(Some(custom_icon))
            .large();

        assert!(icon.loading);
        assert!(icon.loading_icon.is_some());
        assert_eq!(icon.size, Size::Large);
    }

    #[gpui::test]
    fn test_button_icon_variant_types(_cx: &mut gpui::TestAppContext) {
        // Test Icon variant
        let icon_variant = ButtonIconVariant::Icon(Icon::new(IconName::Plus));
        assert!(!icon_variant.is_spinner());
        assert!(!icon_variant.is_progress());

        // Test Spinner variant
        let spinner_variant = ButtonIconVariant::Spinner(Spinner::new());
        assert!(spinner_variant.is_spinner());
        assert!(!spinner_variant.is_progress());

        // Test Progress variant
        let progress_variant = ButtonIconVariant::Progress(ProgressCircle::new(75));
        assert!(!progress_variant.is_spinner());
        assert!(progress_variant.is_progress());
    }
}
