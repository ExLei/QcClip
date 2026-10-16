1. 全部界面图标在iconsall目录里查找，之后复制到assets\icons目录下进行使用，或者从https://tabler.io/icons进行下载使用

2. git提交前缀规范
| 前缀       | 用途描述                                                  |
| ---------- | --------------------------------------------------------- |
| `feat`     | 新增功能（feature）                                       |
| `fix`      | 修复 bug                                                  |
| `docs`     | 文档更新（documentation）                                 |
| `style`    | 代码风格调整（不影响代码逻辑，如空格、格式化等）          |
| `refactor` | 代码重构（既不是新增功能，也不是修复 bug）                |
| `perf`     | 性能优化（performance）                                   |
| `test`     | 增加或修改测试用例                                        |
| `chore`    | 构建过程或辅助工具的变动（如依赖更新、配置文件修改等）    |
| `build`    | 影响构建系统的更改（如 webpack、gulp 等）                 |
| `ci`       | 持续集成相关的更改（如 Travis、GitHub Actions 等）        |
| `revert`   | 回滚之前的提交                                            |
| `merge`    | 合并分支或代码                                            |
| `update`   | 普通更新（非功能或 bug 修复，如依赖版本更新、注释修改等） |
| `init`     | 初始化项目或模块                                          |
| `config`   | 配置文件修改                                              |
| `release`  | 发布版本                                                  |
| `optimize` | 优化代码或逻辑（非性能优化）                              |
| `typo`     | 修复拼写错误                                              |
| `wip`      | 工作进行中（Work In Progress，通常用于临时提交）          |
| `break`    | 重大变更（破坏性修改，可能不向下兼容）                    |

3.git提交前规范
- 提交前在项目根目录运行 `cargo fmt`
- 仅格式化改动使用 `style` 前缀单独提交
