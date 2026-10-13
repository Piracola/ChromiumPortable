# cli.py 分发尾段契约（round 41，L202-263+ 实测）

1. research-packages：不要求 --target；枚举后缀集排序（casefold）、编号目录
   {index:02d}-{stem}；--json 输出 combined={"recognized": [...], "failed": [...]}
   （indent=2、ensure_ascii=False）；有失败 ⇒ 非零退出 + RuntimeError
   "{N} package(s) could not be identified"；无 --json 时逐包 [i/total] + print_report
   或 [FAIL]。
2. 目标类命令（check/build/archive/verify/render-release/update-release）：
   --target 缺失 ⇒ parser.error（exit 2）；check-targets/build-targets 等 targets 后缀
   命令 target=None + split_targets(--target 逗号串)——**verify-targets 用 --target
   传逗号列表，不是位置参数**（main.rs 骨架需复核——当前 VerifyTargets 用的是
   --target? 检查：骨架里 VerifyTargets 有 no_smoke 但 targets 走全局 --target，正确）。
3. resolve_builder_dir 对所有目标类命令统一调用。
