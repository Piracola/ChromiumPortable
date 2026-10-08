# locales/

这个目录在**构建 / 开发流程**中由脚本填充：把仓库唯一的语言来源
"scripts/locales/wizard.json" 复制到本目录，供 Tauri 以 include_str! 内嵌
或作为 resource 打包。

"scripts/locales/wizard.json" 现在仍是**单一事实来源（single source of truth）**：
引擎（portable_builder/i18n.py）、向导（scripts/wizard.py）与图形界面
（scripts/gui.py / scripts/gui_core.py）共用同一份文件，只支持 zh-CN / en
两种语言。本目录不放手工编辑的副本——要改文案请改
scripts/locales/wizard.json，否则下次复制会覆盖掉你的改动。

迁移文档：docs/MIGRATION_RUST_TAURI.md §6.5。
