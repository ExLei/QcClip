# 符号数据

本目录包含 QcClip 符号面板使用的 Unicode 官方数据。

- `UnicodeData.txt`：Unicode 17.0 字符名称、类别和编码信息。
- `Blocks.txt`：Unicode 17.0 区块范围，供后续分类扩展使用。

符号分类采用 QuickClipboard 的范围设计，再由 QcClip 在运行时进行过滤和去重：

- 标点
- 箭头
- 数学
- 货币
- 几何图形
- 方框制图
- 杂项符号
- 技术符号
- 字母类符号
- 数字符号

过滤控制字符、格式字符、代理区、私用区、未分配字符、空白字符以及重复字符。

来源：

- https://www.unicode.org/Public/UCD/latest/ucd/UnicodeData.txt
- https://www.unicode.org/Public/UCD/latest/ucd/Blocks.txt

Unicode 数据按照 Unicode 官方条款使用。
