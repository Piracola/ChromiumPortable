# discovery.py 评分算法锚点（Wave3，自源码 L120-262 实测提取；黄金对照 discovery_reference.json）

## _score_candidate 计分表（逐项分值是行为契约，不得增删）
+15 架构匹配 desired_arch | -100 架构不匹配
+120 文件名 ∈ KNOWN_BROWSER_EXES | -180 stem 含 REJECT_NAME_TOKENS | -70 父路径后3段含 REJECT_PATH_TOKENS
-140 版本信息(ProductName/FileDescription/OriginalFilename)含 installer/setup/update/helper/crash/service
+25 元数据含 browser/chromium/chrome/vivaldi/opera/brave/thorium | +15 OriginalFilename 与文件名一致
+35 同目录含 CHROMIUM_MARKERS | +45 数字版本子目录（前4个）里含 markers | +20 父目录即版本目录且有 markers
+55 导入表含 *_elf.dll（chrome/vivaldi/opera）| -15 导入表为空
read_pe_machine 失败 ⇒ 整个候选 None（不是 0 分）。
reasons 是**中文**字符串——inspect-package --json 黄金对照会逐字节比对它们。

## 排序与选择
排序键：(-score, path 部段数, name.casefold())。
select_browser_candidate：minimum_score=80 且必须有 chromium_evidence，否则 RuntimeError
（错误信息列前 8 候选名+分数）；次优 ≥ 最佳-15（ambiguity_margin）⇒ RuntimeError
（歧义保护，错误信息列并列候选）。

## extract_package_layers
max_depth=4, max_archives=16；目录输入直接 kind=directory；先 remove_path 再解包。
嵌套层判定：archive 后缀 / 名称含 chrome.7z 等优先（黄金记录里 brave 三层链即此）。

## 版本提取
ProductVersion > FileVersion > ""；product = ProductName > FileDescription > stem。
发现 0.0.0.0 时回退 package.version（builder.py prepare_package 侧）。

Rust 移植注意：
- reasons 中文文案逐字保留（M2/M3 黄金对照包含它）；
- casefold ≈ Rust to_lowercase（ASCII 范围等效）；
- path.parts[-3:] 用 Path::components().rev().take(3)；
- Windows 大小写不敏感文件系统上的遍历顺序 Python 依 rglob 序，Rust walkdir 需排
  序后处理或证明与排序键无关（排序键已含 name.casefold 终判，但同分同名时 parts
  长度前必须稳定——用 BTreeMap 收集或显式 sort）。
