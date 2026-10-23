use std::{fs, path::PathBuf, time::Duration};

use gpui::{
    App, AppContext, Context, Render, Window, WindowBounds, WindowOptions, div, prelude::*, px,
    size,
};
use gpui_component::{
    ActiveTheme, Root,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

use super::installer::{self, InstallJob, UPDATE_ARGUMENT};

pub(crate) fn run_if_requested() -> bool {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new(UPDATE_ARGUMENT)) {
        return false;
    }
    let result = arguments
        .next()
        .ok_or_else(|| "缺少更新任务参数".to_owned())
        .and_then(|path| {
            let path = PathBuf::from(path);
            let directory = path.parent().ok_or("更新任务目录无效")?.to_path_buf();
            let bytes = fs::read(&path).map_err(|e| format!("读取更新任务失败：{e}"))?;
            let job: InstallJob =
                serde_json::from_slice(&bytes).map_err(|e| format!("更新任务格式无效：{e}"))?;
            Ok((job, directory))
        });
    match result {
        Ok((job, directory)) => run(job, directory),
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title("QcClip 更新失败")
                .set_description(error)
                .set_level(rfd::MessageLevel::Error)
                .show();
        }
    }
    true
}

struct InstallerView {
    message: String,
    fraction: Option<f32>,
    failed: bool,
    done: bool,
    executable: PathBuf,
    directory: PathBuf,
}

enum Event {
    Progress(String, Option<f32>),
    Finished(Result<(), String>),
}

fn run(job: InstallJob, directory: PathBuf) {
    gpui_platform::application()
        .with_assets(crate::assets::Assets::new())
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            cx.on_window_closed(|cx, _| {
                cx.quit();
            })
            .detach();
            let result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::centered(size(px(480.0), px(240.0)), cx)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("QcClip 自动更新".into()),
                        ..Default::default()
                    }),
                    is_resizable: false,
                    ..Default::default()
                },
                move |window, cx| {
                    let view = cx.new(|cx: &mut Context<InstallerView>| {
                        let (sender, receiver) = async_channel::unbounded();
                        let worker_directory = directory.clone();
                        let executable = job.executable.clone();
                        // 首帧成功绘制后才启动握手，主程序不会因窗口初始化失败而退出。
                        window.on_next_frame(move |_, _| {
                            std::thread::spawn(move || {
                                let result = installer::apply(
                                    &job,
                                    &worker_directory,
                                    |message, fraction| {
                                        let _ = sender
                                            .send_blocking(Event::Progress(message, fraction));
                                    },
                                );
                                if let Err(error) = &result {
                                    let _ = fs::write(worker_directory.join("error.txt"), error);
                                }
                                let _ = sender.send_blocking(Event::Finished(result));
                            });
                        });
                        cx.spawn(async move |view, cx| {
                            while let Ok(event) = receiver.recv().await {
                                let finished = matches!(event, Event::Finished(Ok(())));
                                if view
                                    .update(cx, |view, cx| {
                                        match event {
                                            Event::Progress(message, fraction) => {
                                                view.message = message;
                                                view.fraction = fraction;
                                            }
                                            Event::Finished(result) => {
                                                view.done = true;
                                                if let Err(error) = result {
                                                    view.failed = true;
                                                    view.message = error;
                                                }
                                            }
                                        }
                                        cx.notify();
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                                if finished {
                                    cx.background_executor().timer(Duration::from_secs(2)).await;
                                    cx.update(|cx| cx.quit());
                                    break;
                                }
                            }
                        })
                        .detach();
                        InstallerView::new(executable, directory)
                    });
                    let weak = view.downgrade();
                    window.on_window_should_close(cx, move |_, cx| {
                        weak.upgrade().is_some_and(|view| view.read(cx).done)
                    });
                    cx.new(|cx| Root::new(view, window, cx))
                },
            );
            if let Ok(handle) = &result {
                let _ = handle.update(cx, |_, window, _| {
                    window.set_visible(true);
                    window.activate_window();
                });
            }
            if let Err(error) = result {
                rfd::MessageDialog::new()
                    .set_title("QcClip 更新失败")
                    .set_description(format!("无法打开更新进度窗口：{error}"))
                    .show();
                cx.quit();
            }
            cx.activate(true);
        });
}

impl InstallerView {
    fn new(executable: PathBuf, directory: PathBuf) -> Self {
        Self {
            message: "正在校验更新文件…".into(),
            fraction: None,
            failed: false,
            done: false,
            executable,
            directory,
        }
    }
}

impl Render for InstallerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().colors;
        v_flex()
            .size_full()
            .p_6()
            .gap_4()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(div().text_lg().child(if self.failed {
                "更新未完成"
            } else {
                "正在更新 QcClip"
            }))
            .child(
                div()
                    .id("update-message")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .text_sm()
                    .child(self.message.clone()),
            )
            .when_some(self.fraction, |view, fraction| {
                view.child(
                    div()
                        .w_full()
                        .h(px(6.0))
                        .rounded_md()
                        .bg(theme.border)
                        .child(
                            div()
                                .h_full()
                                .w(gpui::relative(fraction))
                                .rounded_md()
                                .bg(theme.primary),
                        ),
                )
            })
            .when(!self.done && self.fraction.is_none(), |view| {
                view.child(
                    h_flex()
                        .gap_2()
                        .child(gpui_component::spinner::Spinner::new())
                        .child(div().text_sm().child("更新进行中，请稍候…")),
                )
            })
            .when(self.failed, |view| {
                view.child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("restart-qcclip")
                                .primary()
                                .label("重新打开 QcClip")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    match installer::restart(&view.executable) {
                                        Ok(()) => cx.quit(),
                                        Err(error) => {
                                            view.message = error;
                                            cx.notify();
                                        }
                                    }
                                })),
                        )
                        .child(
                            Button::new("show-update-error")
                                .label("查看错误详情")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    let _ = crate::platform::shell_open::open_with_default_program(
                                        &view.directory.join("error.txt"),
                                    );
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("close-updater")
                                .label("关闭")
                                .on_click(|_, _, cx| cx.quit()),
                        ),
                )
            })
    }
}
