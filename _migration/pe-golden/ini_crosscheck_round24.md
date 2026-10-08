# ini golden cross-check（round 24，父代理）

Python 导出（ini_reference.json）与 Wave1-C 声明的 Rust real_fixtures 测试输出对照：
- applied keys 序列：完全一致（suppress_false_upgrade_notification=1 →
  wheel_tab_when_press_rbutton=0 → open_url_new_tab=1 → open_bookmark_new_tab=1）
- 编码：baseline 与 defaults 均为 utf-16（C 的 Utf16 默认判定一致）
- 行数：基线 173 行（含尾空行 16088 字节），合并后 173 行（原地改写，不增行）
- C 声明的改写行号 114/147/148/149 与 defaults 的 4 键数量吻合

结论：Rust real_fixtures 与 Python 黄金在 applied 顺序与编码维度一致；
字节级恒等由 C 的 no-op 回写测试覆盖（review-C 正在独立复现）。
