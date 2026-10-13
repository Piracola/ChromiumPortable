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


---

## 补遗（L135-243 实测）：check_targets / build_selected_targets 循环语义

1. check_targets：
   - validate_target_asset_matchers 先行（重叠即崩）。
   - 逐 target：force_build(workflow_dispatch) or 无 current or 无资产 or (版本变化且 is_upgrade)。
   - 逐 target 打印："[INFO] {name}: upstream=X current=Y asset_present=Z update=W"。
   - tag_target：config.release.tag_target 或**首个 target**；create policy 取 config 级
     或 tag_target 级（两级回退）。
   - create_new_release = 无 release_id OR (tag_target 有 update AND should_create...)。
   - 输出：UPDATE_NEEDED=any(updates)、CREATE_NEW_RELEASE、MINOR_UPDATE、
     RELEASE_ID/RELEASE_TAG 条件、**逐 target** {prefix}_UPDATE 与 UPSTREAM_{prefix}
     （env_prefix 覆盖语义在此再次生效）。
2. build_selected_targets：
   - 逐 target 读 {prefix}_UPDATE（环境变量跨 job 通信！）决定跳过；
     workflow_dispatch 强制全建；跳过打印 "[INFO] Skipping {name}; no update required."
   - 构建后写 {prefix}_VERSION/_BUILD_VERSION/_PACKAGE_VERSION/_ARCHIVE/_SHA256/_SIZE
     （六键——asset_facts 的消费端就是它们）。
   - 末尾 ensure_shared_release_assets（CREATE_NEW_RELEASE=true 时把 assets 挂到新 release）。
3. Rust 移植要点：{prefix}_UPDATE 经 GITHUB_ENV 在两个命令间传递——check 与 build 是
   分开的 CI 步骤，Rust 端必须真的读写该环境变量而非进程内状态。


---

## 补遗 2（L243-350 实测）：ensure_shared_release_assets / render_multi / update_multi

1. ensure_shared_release_assets：只对**本次未重建**（{prefix}_UPDATE != true）的 target
   生效；从既有 release 拉回最新资产（download_release_asset）到 build/assets（重用已存在
   文件），再写 {prefix}_ARCHIVE/_SHA256/_SIZE——release 说明里未重建渠道也有校验和行。
2. render_multi_release：config 顶层 release 块**缺失** ⇒ 退化为单 target render_release；
   版本回退链 {prefix}_PACKAGE_VERSION > {prefix}_VERSION > UPSTREAM_{prefix}；
   tag/title/body 缺省 "v{date}"/"{date}"/""（与单目标不同！）；assert_body_versions
   覆盖全部 targets。
3. update_multi_release：只为**本次重建**的 target 删旧资产（UPDATE=true 才 delete）；
   PATCH body 语义与单目标一致（MINOR_UPDATE 才带 name+tag_name）。
   注：资产上传本身不在 Python 引擎里——由 workflow 的 softprops/action-gh-release
   从 build/assets/*.7z 上传。Rust 引擎保持同样边界（M3 不实现 upload）。
