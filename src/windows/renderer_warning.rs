//! 渲染器落到软件渲染（CPU 光栅化）时提示用户。
//!
//! 使用系统对话框而不是应用通知，避免用户关闭通知后连“不再提示”都看不到。

use gpui::{AnyWindowHandle, App, PromptButton, PromptLevel};

use crate::platform::system_info::{AdapterInfo, FeatureLevel};

/// 软件渲染的原因，决定给用户的建议。
#[derive(Clone, Debug, PartialEq, Eq)]
enum SoftwareRenderingCause {
    /// 没有任何硬件显卡，通常是显卡驱动未安装。
    NoHardwareAdapter,
    /// 硬件显卡都达不到 D3D 11.0，显卡或其驱动过旧。
    UnsupportedAdapter { name: String },
    /// 有满足要求的硬件显卡却未能启用，多半是驱动异常。
    AdapterUnavailable { name: String },
}

fn classify(adapters: &[AdapterInfo]) -> SoftwareRenderingCause {
    let hardware: Vec<&AdapterInfo> = adapters
        .iter()
        .filter(|adapter| !adapter.software)
        .collect();
    let Some(first) = hardware.first() else {
        return SoftwareRenderingCause::NoHardwareAdapter;
    };
    let capable = hardware.iter().find(|adapter| {
        adapter
            .feature_level
            .as_ref()
            .is_ok_and(|level| *level >= FeatureLevel::RENDERER_MINIMUM)
    });
    match capable {
        Some(adapter) => SoftwareRenderingCause::AdapterUnavailable {
            name: adapter.name.clone(),
        },
        None => SoftwareRenderingCause::UnsupportedAdapter {
            name: first.name.clone(),
        },
    }
}

/// 在主窗口创建后调用：记录渲染器显卡，软件渲染时按需弹出提示。
pub(crate) fn check(window: AnyWindowHandle, cx: &mut App) {
    let specs = match window.update(cx, |_, window, _| window.gpu_specs()) {
        Ok(Some(specs)) => specs,
        Ok(None) => {
            tracing::warn!("无法读取渲染器显卡信息");
            return;
        }
        Err(error) => {
            tracing::warn!(%error, "无法读取主窗口渲染器信息");
            return;
        }
    };
    if !specs.is_software_emulated {
        tracing::info!(
            device = %specs.device_name, vendor = %specs.driver_name, driver = %specs.driver_info,
            "渲染器显卡"
        );
        return;
    }
    tracing::warn!(
        device = %specs.device_name, vendor = %specs.driver_name, driver = %specs.driver_info,
        "渲染器正在使用软件渲染（CPU 光栅化），界面绘制会显著占用 CPU"
    );
    #[cfg(target_os = "windows")]
    {
        if crate::state_runtime::state(cx).software_rendering_warning_dismissed {
            return;
        }
        // 远程桌面本身就常走软件渲染，更新驱动无济于事，不打扰用户。
        if crate::platform::system_info::is_remote_session() {
            tracing::info!("当前为远程桌面会话，跳过软件渲染提示");
            return;
        }
        cx.spawn(async move |cx| {
            // 显卡探测需要创建 D3D 设备，放到后台执行。
            let adapters = cx
                .background_executor()
                .spawn(async { crate::platform::system_info::adapters() })
                .await;
            let cause = classify(&adapters);
            tracing::info!(?cause, "软件渲染原因");
            let answer = cx.update(|cx| {
                let translator = crate::i18n::translator(cx);
                let suggestion = match &cause {
                    SoftwareRenderingCause::NoHardwareAdapter => {
                        translator.translate("renderer_warning.no_hardware_adapter")
                    }
                    SoftwareRenderingCause::UnsupportedAdapter { name } => translator
                        .translate("renderer_warning.unsupported_adapter")
                        .replace("{gpu}", name),
                    SoftwareRenderingCause::AdapterUnavailable { name } => translator
                        .translate("renderer_warning.adapter_unavailable")
                        .replace("{gpu}", name),
                };
                let detail = format!(
                    "{}\n\n{}",
                    translator.translate("renderer_warning.message"),
                    suggestion
                );
                window
                    .update(cx, |_, window, cx| {
                        window.prompt(
                            PromptLevel::Warning,
                            &translator.translate("renderer_warning.title"),
                            Some(&detail),
                            &[
                                PromptButton::ok(translator.translate("renderer_warning.continue")),
                                PromptButton::new(translator.translate("renderer_warning.never")),
                            ],
                            cx,
                        )
                    })
                    .ok()
            });
            let Some(answer) = answer else {
                return;
            };
            if answer.await != Ok(1) {
                return;
            }
            cx.update(|cx| {
                let mut state = crate::state_runtime::state(cx);
                state.software_rendering_warning_dismissed = true;
                if let Err(error) = crate::state_runtime::set_state(state, cx) {
                    tracing::warn!("保存软件渲染提示状态失败：{error}");
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{SoftwareRenderingCause, classify};
    use crate::platform::system_info::{AdapterInfo, FeatureLevel};

    fn adapter(name: &str, software: bool, level: Option<(u8, u8)>) -> AdapterInfo {
        AdapterInfo {
            name: name.into(),
            vendor_id: 0,
            device_id: 0,
            dedicated_video_mib: 0,
            shared_system_mib: 0,
            software,
            driver_version: None,
            feature_level: level
                .map(|(major, minor)| FeatureLevel { major, minor })
                .ok_or_else(|| "failed".into()),
            enumerated: 1,
        }
    }

    const WARP: (&str, bool, Option<(u8, u8)>) =
        ("Microsoft Basic Render Driver", true, Some((11, 1)));

    #[test]
    fn only_warp_means_driver_missing() {
        let adapters = [adapter(WARP.0, WARP.1, WARP.2)];
        assert_eq!(
            classify(&adapters),
            SoftwareRenderingCause::NoHardwareAdapter
        );
        assert_eq!(classify(&[]), SoftwareRenderingCause::NoHardwareAdapter);
    }

    #[test]
    fn hardware_below_d3d11_is_unsupported() {
        let adapters = [
            adapter("Intel HD Graphics 3000", false, Some((10, 1))),
            adapter("Broken GPU", false, None),
            adapter(WARP.0, WARP.1, WARP.2),
        ];
        assert_eq!(
            classify(&adapters),
            SoftwareRenderingCause::UnsupportedAdapter {
                name: "Intel HD Graphics 3000".into()
            }
        );
    }

    #[test]
    fn capable_hardware_points_to_driver_problem() {
        let adapters = [
            adapter("Old GPU", false, Some((10, 0))),
            adapter("Intel UHD Graphics 620", false, Some((11, 0))),
            adapter(WARP.0, WARP.1, WARP.2),
        ];
        assert_eq!(
            classify(&adapters),
            SoftwareRenderingCause::AdapterUnavailable {
                name: "Intel UHD Graphics 620".into()
            }
        );
    }
}
