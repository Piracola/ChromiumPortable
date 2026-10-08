# review 移交项（Wave1 review 链产出，Wave2/4 发单时必须传达）

1. （review-B2 Finding 1）wizard.json 新占位符形状：Rust formatter 是 str.format 严格子集
   （仅 {name} + {{}} 转义）。任何新键引入 {x!r}/{x:>5}/{x.y}/{x[0]} 等形状必须同步
   扩展 Rust formatter + 新增 golden，否则静默回退模板。当前 9 种简单形状（见 review-B2 报告）。
2. （review-B2 Finding 6）跨波耦合：crate 测试活读 scripts/locales/wizard.json 与
   config_i18n_reference.json（168 计数 + 全量 en 键表断言）。任何 wave 改 wizard.json
   键集必须同步再生黄金，否则 51/51 会跨波破裂。
3. （review-A Finding 2，已入 doc §4.6）versions 全角数字偏差：unreachable，勿"修复"。
4. （review-E Finding 4）done 事件：runner 队列协议是 ("done", code, elapsed)，但
   gui.py:918 _emit 层会富化 result+state——Wave4 的 emit 层照抄富化。
5. （review-E Finding 5）log tag：None 而非 ""——Wave4 JS 侧 falsy 判断两者等价，但
   移植时对齐 None 语义。

6. （review-W2C Finding 3）行尾归一：Python write_text 在 Windows 产 CRLF、Rust fs::write 产 LF
   （selected.browser.json 849 vs 820 字节，SHA256 不同、解析后等价）。M3 的字节对比门禁
   在 Windows 上哈希前必须 CRLF→LF 归一化，或改用解析后 JSON 比较。
7. （review-W2C Finding 1 澄清）prepare_build_target.py:31 确实注入 target["target"]；
   prepare_target_reference.json 的 chrome_stable_full 是注入前的 catalog 快照——
   M3 对照时须先剥 "target" 键或直接与脚本实跑输出比。

8. （review-Wave2 Finding 1 MEDIUM）download_file 的 reqwest blocking Client::timeout(120s)
   是**总时限**（Python requests timeout=120 是每 socket 操作）——慢链路下载 149MB 安装包
   会中途中止。M3 前修复：去掉总时限或改用大 idle/read timeout（契约="无总限，120s/读"）。
9. （review-Wave2 Finding 2 LOW-MED）digest 为 "sha256:"（前缀后空）时 normalize 返回
   Ok(None)，download_file 两处 .expect("non-empty digest normalizes to Some") 会 PANIC
   ——Python 是 RuntimeError。M3 前改为显式错误。
10. （review-Wave2 Finding 7/8 Note）pelite imports() 对损坏导入表整体失败（Python 跳过
   坏 RVA 继续走）；资源树 >3 层损坏时 Rust 产出 ['0','0','0']（Python 给真实前三标识）。
   有效 PE 不受影响（7/7 黄金）。损坏样本场景在 M3 对照时知晓即可。
11. （review-Wave2 Finding 11）providers/mod.rs 的 get_package 分发还是 M0 桩（未知类型
   KeyError 文案未移植）——Wave3 接 config→provider 分发时必须补上，commit 信息
   "mod dispatch already wired" 不准确。
12. （review-Wave2 hygiene）pe.rs assert 测试每次运行在 %TEMP% 泄漏 pe_assert_{pid} 目录
   （已积累 37 个）——加清理。
