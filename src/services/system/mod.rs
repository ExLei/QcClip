pub(crate) mod win_v_hotkey;

#[cfg(target_os = "windows")]
pub(crate) fn check_win_v_on_startup(cx: &mut gpui::App) {
    use gpui::{AppContext as _, PromptButton, PromptLevel};

    let Some(binding) = crate::config_runtime::toggle_main_window_shortcut(cx) else {
        return;
    };
    if !crate::hotkey::is_win_v_binding(&binding) || win_v_hotkey::is_win_v_hotkey_disabled() {
        return;
    }

    let Some(window) = crate::windows::main::MainWindowService::window_handle(cx) else {
        tracing::error!("启动时检查 Win+V 失败：主窗口尚未就绪");
        return;
    };
    let translator = crate::i18n::translator(cx);
    let title = translator.translate("settings.shortcuts.win_v_replace.title");
    let detail = translator.translate("settings.shortcuts.win_v_replace.detail");
    let cancel = PromptButton::cancel(
        translator.translate("settings.shortcuts.win_v_replace.cancel"),
    );
    let confirm =
        PromptButton::ok(translator.translate("settings.shortcuts.win_v_replace.confirm"));
    let answer = match window.update(cx, |_, window, cx| {
        window.prompt(
            PromptLevel::Warning,
            &title,
            Some(&detail),
            &[cancel, confirm],
            cx,
        )
    }) {
        Ok(answer) => answer,
        Err(error) => {
            tracing::error!("启动时打开 Win+V 替换提示失败：{error}");
            return;
        }
    };

    cx.spawn(async move |cx| {
        if answer.await.unwrap_or(0) != 1 {
            return;
        }
        let result = cx
            .background_spawn(async { win_v_hotkey::disable_win_v_hotkey() })
            .await;
        if let Err(error) = result {
            tracing::error!("启动时替换系统 Win+V 失败：{error}");
        }
    })
    .detach();
}
