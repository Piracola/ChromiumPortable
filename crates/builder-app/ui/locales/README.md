# ui/locales

构建/开发期把仓库根的 "scripts/locales/wizard.json" 复制到本目录，供 Tauri 以
include_str! 内嵌（离线原则）。

"scripts/locales/wizard.json" 仍是**单一事实来源（single source of truth）**：
引擎（crates/portable-builder/src/i18n.rs）与图形界面（crates/builder-app）共用
同一份文件，只支持 zh-CN / en。

要改文案：改 scripts/locales/wizard.json，然后重新复制到本目录，
否则下次复制会覆盖掉你的改动。
