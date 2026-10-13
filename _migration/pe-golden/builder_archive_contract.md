# builder.py archive/inject 语义锚点（Wave3，自源码 L400-517 实测提取）

## archive_target（M3 判据②的直接实现）
1. 7z 打包命令固定为：7z a -t7z -mx=9 <abs_archive_path> <items>，cwd=release_dir。
   - mx=9 是体积优化的既有决策，移植不得降级为默认压缩。
   - source_dir=None ⇒ items=["*"]（整目录）；否则 items=[source_dir.name]（单目录名，
     相对路径语义依赖 cwd）。
2. 版本/日期回退链（移植时逐级保留）：
   version = 参数 or BUILT_VERSION or BROWSER_VERSION
   package_version = 参数 or PACKAGE_VERSION or UPSTREAM_VERSION or version
   build_date = 参数 or BUILD_DATE or 当天 %Y-%m-%d
3. archive_name 模板缺省 "{display_name}_Portable_{version}_{date}.7z"；
   build-package 路径会覆盖为 "{resolved}_Portable_{version}_{date}.7z"。
4. 先 remove_path(archive_path) 再打包（防 7z 追加进旧档）。

## build_package_file（GUI 与 build-package 子命令的共同入口）
1. target_id = "auto_" + sanitize(stem.casefold())（[^a-z0-9]+→_，strip('_')，空则 auto_package）。
2. 合成 target：layout=auto、allow_7z_*=False（静态路径绝不下载/装 7z）、
   ini_location/version_dll_location=app_root。
3. 先以 0.0.0.0 占位跑 prepare_package 拿 discovery，再回填 version/product，
   output_dir = _safe_output_name(output_dir or product)（sanitize: [<>:"/\\|?*]+→_，
   strip(' .')，空则 "Browser"）。
4. finally 移除 build/temp/<target_id>（不留第二份解包拷贝）。

## inject_dll（L358-399 前段已读）
1. setdll /t <exe>（verify_architecture 开启时）→ setdll /d:<rel_dll> <exe>，
   rel 是 version.dll 相对 exe 目录的相对路径（os.path.relpath，跨盘符失败退绝对路径）。
2. cwd = exe 所在目录。setdll 输出 OEM 码页（风险 R4）。
3. verify 阶段 assert_no_setdll_backup 只查 <exe>~ 精确路径（见 verify_smoke_contract.md）。

每条在 Rust（builder.rs / tools.rs）中一一对应；M3 门禁以本文件+黄金 JSON 为验收清单。


---

## 补遗（L305-399 实测）：inject_dll / finalize / _complete_build

1. inject_dll：
   - exe 定位三级：staged.executable（auto 布局）→ version_dir/exe_name → **rglob 同名文件取
     相对深度最浅者**（"Browser executable not found: ..." 失败文案）。
   - verify_architecture 默认 True ⇒ 先 setdll /t <exe>（只打印不判失败）。
   - /d: 参数是 version.dll **相对 exe 目录**的 relpath；跨盘 ValueError ⇒ 回退绝对路径。
   - 非零退出 ⇒ RuntimeError("DLL injection failed.")（stderr 打印）。
   - 注入后立即 assert_portable_version_import（导入表自检闭环）。
   - setdll 备份 <exe>~ 删除（"Removing setdll backup" 打印）；remove_setdll 默认 True ⇒
     setdll 工具本体也删除。
2. finalize：build/release/<output_dir>/ ← move stage；写 version.txt（UTF-8，内容=staged
   version 无尾换行）；start_script（默认 "开始.bat"）若在 workdir 存在则**复制到 release_dir
   根**（不是 app_dir 内）。
3. _complete_build：版本不一致 ⇒ [WARN] 打印后**以目录版本为准**；env 回写五键
   BUILT_VERSION/BROWSER_VERSION/PACKAGE_VERSION/UPSTREAM_VERSION/OUTPUT_DIR +
   条件 CHROME_PLUS_VERSION（github_env.write_env 非落盘路径 → 实际走 stdout 打印——
   子进程模式下由 GUI 捕获，CI 模式下 GITHUB_ENV 存在时落盘）。
