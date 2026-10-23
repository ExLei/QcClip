# 版本更新检查

应用优先读取 `https://api.github.com/repos/mosheng1/QcClip/releases`，包括正式版与预发布版，排除草稿，并按版本号选择最新版本。GitHub 请求失败或超时后，会立即改用 `https://gitee.com/api/v5/repos/mk-mo-sheng/QcClip/releases`；两个来源都失败时才报告检查失败。发现的版本会打开实际成功来源的发布页，不下载或安装。

版本比较采用 SemVer 优先级，支持 `v` / `V` 前缀、`alpha`、`beta`、`rc`、`nightly`、`preview` 等后缀。紧凑后缀如 `beta2`、`beta10` 会规范化为 `beta.2`、`beta.10` 后比较，数字按数值排序；同一基础版本的正式版高于预发布版，`+build` 元数据不影响优先级。无法识别的标签会跳过。GitHub 标签中的未知后缀按 SemVer 字符排序，不根据发布时间推测版本顺序。

默认每天检查，可选择每3天、每周或永不更新。成功检查的时间、最新版本及已通知版本保存在数据目录的 `config.updates.json`；检查失败保留已有更新提示，自动检查1小时后重试。手动检查不受频率和重试等待限制。自动通知同一版本只弹一次。

“发现新版本提醒”统一放在“常规 → 通知偏好”，沿用原有更新通知配置值，只控制自动检查发现新版本时的弹窗；关闭后更新标识仍然显示。手动检查始终在关于页显示结果，其普通结果弹窗受“操作结果提示”控制，失败弹窗受“警告与错误提示”控制，所有弹窗仍受全局通知总开关控制。关于页保留检查频率、代理和手动检查入口。

代理设置支持跟随系统、直连、自定义 HTTP 代理，HTTPS 更新接口通过 HTTP CONNECT 隧道访问。跟随系统优先使用代理环境变量，Windows 上没有代理环境变量时读取系统手动代理；PAC 自动代理脚本不执行，可改用自定义代理。选择自定义代理后填写地址并点击“保存代理”生效。

GitHub 未认证 API 的额度按网络出口 IP 计算，共用代理节点可能耗尽额度，而直连仍可正常检查。额度耗尽时会继续尝试 Gitee；两个来源都不可用时显示失败原因。代理设置会同时用于 GitHub 和 Gitee，不会自动绕过用户选择的代理。

## 本地测试

需要 Python 3。第一个 PowerShell 窗口启动服务：

```powershell
py -3 scripts/update_test_server.py --scenario update --version v1.0.0-beta.2
```

第二个 PowerShell 窗口设置临时环境变量并启动 debug 包：

```powershell
$env:QCCLIP_UPDATE_TEST_URL = 'http://127.0.0.1:18765/releases'
cargo run
Remove-Item Env:QCCLIP_UPDATE_TEST_URL
```

只有 `debug_assertions` 编译开启时读取该变量，且只接受本机 HTTP 地址。release 编译完全排除环境变量入口，始终请求 GitHub。debug 本地测试使用独立的 `config.test-updates.json`，不会污染正式检查记录。

可用场景：`update`（发现更新）、`current`（当前版本）、`empty`（空列表）、`prerelease`（测试版）、`mixed`（版本乱序、草稿和无效标签）、`paginated`（多页）、`not-found`（尚未建立仓库）、`rate-limit`（限流）、`error`（服务错误）、`invalid`（数据格式错误）。`--delay 3` 可观察检查中的状态，`--delay 25` 可测试超时。

服务运行时可切换场景，再从托盘或关于页点击“检查更新”：

```powershell
Invoke-RestMethod 'http://127.0.0.1:18765/control?scenario=mixed'
Invoke-RestMethod 'http://127.0.0.1:18765/control?scenario=update&version=v1.0.0-rc2'
```

发现更新后检查托盘底部绿色居中项、设置侧边栏“更新”标识、关于页版本对比、主窗口图标上的绿色向上箭头、全局通知中的发布页按钮。测试场景的跳转仍使用真实 GitHub 发布页，尚未发布的模拟标签可能显示 GitHub 404。

```powershell
py -3 scripts/update_test_server.py --self-test
cargo test --test update_check
```
