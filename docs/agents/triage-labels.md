# 分诊标签

这些技能都用五个规范分诊角色来表述。这份文件把这些角色映射到这个仓库的议题跟踪器实际使用的标签字符串。

| mattpocock/skills 里的标签 | 我们跟踪器里的标签 | 含义 | 现状 |
| --- | --- | --- | --- |
| `needs-triage` | `needs-triage` | 维护者需要评估这条议题 | 已在 `ExLei/QcClip` 创建 |
| `needs-info` | `needs-info` | 等待报告者补充更多信息 | 已在 `ExLei/QcClip` 创建 |
| `ready-for-agent` | `ready-for-agent` | 已经完整说明，可以交给 AFK 代理 | 已在 `ExLei/QcClip` 创建 |
| `ready-for-human` | `ready-for-human` | 需要由人来实现 | 已在 `ExLei/QcClip` 创建 |
| `wontfix` | `wontfix` | 不打算处理 | 已有（fork 时继承） |

当某个技能提到某个角色时（例如「套用 AFK 就绪的分诊标签」），就使用这张表里对应的标签字符串。

## 标签都在哪里

议题跟踪器是 `ExLei/QcClip`（本仓库 `origin`），本账号在那里是 admin，标签已经建好，直接按名字用、直接贴。上游 `mosheng1/QcClip` 的标签只读，不要在那边改。

重建同样的标签（换仓库或误删时）：

```bash
gh label create needs-triage    -R ExLei/QcClip --color fbca04 --description "维护者需要评估这条议题"
gh label create needs-info      -R ExLei/QcClip --color f9d0c4 --description "等待报告者补充更多信息"
gh label create ready-for-agent -R ExLei/QcClip --color 0e8a16 --description "已经完整说明，可以交给 AFK 代理"
gh label create ready-for-human -R ExLei/QcClip --color 1d76db --description "需要由人来实现"
```

`/wayfinder` 另外需要五个标签，也已建好：`wayfinder:map`、`wayfinder:research`、`wayfinder:prototype`、`wayfinder:grilling`、`wayfinder:task`。

把「我们跟踪器里的标签」那一列改成你实际使用的词汇。
