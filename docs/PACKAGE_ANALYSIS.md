# 常见浏览器安装包结构

本文记录智能结构解析器的样本分析结果。分析过程只读取安装包、列出归档内容并检查 PE 元数据，没有运行或安装任何浏览器。

## 样本结果

| 浏览器 | 安装包结构 | 可识别的主程序 | 结论 |
| --- | --- | --- | --- |
| 360 极速浏览器 X | 外层 EXE 包含 `chrome.7z`，内部为 `Chrome-bin/版本号` | `Chrome-bin/360chromex.exe` | 可自动识别，但不能按文件大小选择；包内还有体积更大的兼容组件和多个安装、更新程序 |
| Thorium | 外层 EXE 包含 `chrome.7z`，内部为 `Chrome-bin/版本号` | `Chrome-bin/thorium.exe` | 可自动识别 |
| Vivaldi | 外层 EXE 包含 `vivaldi.7z`，内部为 `Vivaldi-bin/版本号` | `Vivaldi-bin/vivaldi.exe` | 可自动识别 |
| Opera | 外层归档直接包含完整浏览器文件，没有数字版本目录 | `opera.exe` | 可自动识别，但解析器必须支持平铺结构，并排除 installer、autoupdate、crashreporter 等辅助程序 |
| Brave | 外层 Brave Update 安装器的资源中有 LZMA/BCJ2 bundle，bundle 内再有 `brave_installer` 和 `chrome.7z` | `Chrome-bin/brave.exe` | 已通过专用 Brave/Omaha 静态解包适配器自动识别 |

样本主程序均为 x64 PE，并具有正常导入表。自动流程已经在解包副本上完成 Chrome++ 注入并重新检查导入表；没有运行安装包或浏览器。浏览器实际启动后的兼容性和便携数据目录行为仍需在隔离环境或发布验证中确认。

## 当前解析规则

1. 先读取安装包的产品名、版本和公司信息，识别浏览器及安装器类型。
2. 支持安装包内再包含一层 `chrome.7z`、`vivaldi.7z` 等归档。
3. 同时支持“主程序在版本目录上一层”和“所有文件平铺在同一层”两种结构。
4. 结合 EXE 的产品名、原始文件名、所在位置和周围的 Chromium 资源判断主程序。
5. 排除名称包含 installer、setup、update、helper、crashreporter、proxy、driver、shell 等含义的辅助程序。
6. 不以文件大小作为主要依据。360 样本中体积最大的 EXE 不是主浏览器。
7. 出现多个可信候选时停止并输出候选列表，不自动猜测。
8. Brave/Omaha 更新器使用独立的资源解包适配器；适配器只读取和解包数据，不运行安装程序。

## 当前样本中的明确目标

```text
360       Chrome-bin/360chromex.exe
Thorium   Chrome-bin/thorium.exe
Vivaldi   Vivaldi-bin/vivaldi.exe
Opera     opera.exe
Brave     Chrome-bin/brave.exe
```

这些结构应作为自动解析器的首批回归样本。
