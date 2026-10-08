# multi.py 语义锚点（Wave3，自源码 L14-133 实测提取）

1. env_name: [^A-Za-z0-9]+ → _，strip('_')，upper。target 名直接当环境变量前缀用。
2. split_targets: 逗号分割 + strip + 滤空（不去重、不排序——保序）。
3. asset_facts（渲染 check 行的"本次产物 or 既有资产"回退）：
   - env_prefix 可被 target.env_prefix 覆盖，否则 env_name(target_name)；
   - 读 {prefix}_ARCHIVE / {prefix}_SHA256 / {prefix}_SIZE；
   - digest 为空且 release_id 存在 ⇒ 查 release 最新资产回填（name/size/digest），
     digest 形如 "sha256:<hex>"，取冒号后段；
   - size 经 human_size 渲染，空则 "-"（build_flat_context L81: value or "-"）。
4. build_flat_context: 前缀小写（env_name(target).lower()）拼进模板上下文——
   多目标 release body 的 {chrome_version} / {chrome_sha256} 等占位符来自这里。
5. validate_target_asset_matchers：用干跑样本（123.456.789.0 / 2099-12-31）渲染各
   target 的 archive_name，再互测对方 matcher；**发现重叠 = 直接 RuntimeError**
   （防一个 target 的 release 把另一个 target 的资产删掉的静默事故）。
   这条在 Rust 移植里必须有对应黄金用例（conflict 场景 + 通过场景）。

以上进 Wave3 简报；黄金对照补入 multi_reference（M3 前完成）。
