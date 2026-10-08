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
