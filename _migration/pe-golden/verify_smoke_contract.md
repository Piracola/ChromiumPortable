# verify.rs smoke 判决契约（Wave3 移植锚点，自 verify.py 实测提取）

1. 判据链：解包后 (a) assert_portable_version_import（导入表相对 version.dll）
   (b) assert_no_setdll_backup——只查 <exe>~ 这一个精确路径，不查通配（防误报）
   (c) assert_no_forbidden_files (d) smoke_test。
2. smoke_test 顺序：先断言 Data 目录**不存在**（已带 Data 的包 = 泄漏用户配置），
   再 --version 短跑（120s），再 DEFAULT_SMOKE_ARGS 长跑（180s），
   然后 **轮询**（wait_for_directory，0.5s 间隔，60s 超时）等 Data 目录出现。
3. 关键语义（AGENTS.md 教训的代码化）：
   - msedge.exe 是 launcher stub，退出码本身几乎无意义——真正的证明是
     Chrome++ 把 profile 重定向进了便携 Data 目录；
   - launcher 退出远早于 detached children 建好 profile，所以必须**轮询**而非单次检查；
   - Chromium 是 GUI 子系统二进制，capture 的管道恒空，退出码即"patched PE 成功
     加载 DLL"的信号。
4. 全部超时可被 target 覆盖：smoke_data_dir(=Data)/smoke_timeout(120/180)/
   smoke_data_timeout(60)/smoke_args(DEFAULT_SMOKE_ARGS)。
5. cleanup：浏览器子进程短暂存活持有 DLL 句柄 → 删除重试 4 次 × 3s。
6. version.txt 存在时只打印不比对（Helium 包版本与 Chromium 版本有意不同）。

以上每条在 Rust 移植（verify.rs）中必须一一对应；M3 门禁 smoke 用例以本文件为验收清单。


---

## 补遗（L44-128 实测）：档案定位与可执行文件发现

1. find_target_archive：assets_dir.glob("*.7z") 中 regex **fullmatch** archive_name
   （不是 search）；无匹配 ⇒ FileNotFoundError（错误信息列出全部现存档案名）；
   多个匹配 ⇒ **mtime 最新**者。
2. locate_executable：auto 布局走 analyze_extracted_app 重新静态分析；其余布局
   find_version_dir(app_root) + exe_name，normpath 后必须存在，否则
   FileNotFoundError("Browser executable not found in archive: ...")。
3. run_browser 超时 ⇒ RuntimeError "Browser did not exit within {timeout}s"；
   非零退出 ⇒ 打印 stdout/stderr 前 2000 字符 + RuntimeError 带码。
   （语义再次强调：零退出码无证明力，Data 目录出现才是断言。）
4. describe_tree：排序后、深度 ≤2 的相对路径清单，用于失败信息的可诊断性——
   Rust 移植保留该输出形态（M3 对照验收的失败分支也要像样）。
