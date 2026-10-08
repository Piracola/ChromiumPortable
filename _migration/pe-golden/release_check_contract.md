# release.py check/render 契约（Wave3，自源码 L210-294 实测提取）

## check 决策树（UPDATE_NEEDED，CI 冻结契约的源头）
1. force_build（手动 dispatch）⇒ update_needed=True，"[INFO] Manual dispatch detected; forcing build."
2. 无 current_version ⇒ True，"No existing release version found; build is needed."
3. 资产缺失（asset_present=False）⇒ True，"Release asset is missing; rebuild is needed."
4. upstream != current 且 is_upgrade ⇒ True，"Version upgrade detected: A -> B"
5. 其余 ⇒ False，"No newer upstream version detected."
（文案逐字保留——Actions 日志肉眼对照用）

## create_new_release 组合逻辑
= not release_id OR (update_needed AND current_version AND
  should_create_new_release(policy, upstream, current))
minor_update = update_needed AND current_version AND not create_new_release
  AND upstream != current AND is_upgrade（仅在"留在既有 release"时原地改资产名）

## write_env 输出（check 命令）
恒写 UPDATE_NEEDED / UPSTREAM_VERSION / CREATE_NEW_RELEASE / MINOR_UPDATE
（"true"/"false" 小写串）；RELEASE_ID 与 RELEASE_TAG 条件追加（有值才写）。
env_json blob 由 write_env 统一产出（已移植，FIX-APPROVED）。

## render_release
模板缺省：tag "v{version}" / title "{display_name} {version}" /
body "自动构建的 {display_name} 便携版\n\n构建时间: {date}\n{display_name} 版本: {version}\n"
version 回退链：参数 > BUILT_VERSION > BROWSER_VERSION > UPSTREAM_VERSION
date 回退链：参数 > BUILD_DATE > 当天
assert_body_versions 单目标调用；body 写 build/release_body.md（UTF-8 无 BOM）；
write_env 写 RELEASE_TAG/RELEASE_TITLE/RELEASE_BODY_PATH。

## GitHub API 面（reqwest 手写范围）
latest_release / delete_release_asset / delete_target_assets /
find_latest_target_asset / download_release_asset（分页语义以 release.py 为准，
分页参数与 per_page 逐条对照——移植时读 L31-44 与 L325-415）。
