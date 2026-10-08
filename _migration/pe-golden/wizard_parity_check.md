# wizard.py 退役前的功能对齐核查（round 43）

向导三菜单（wizard.py:363-365）：one（本机安装包）/ folder（整目录）/ online（在线目标）。
GUI 的 vars.mode 恰为 "one"|"folder"|"online"（gui_core.py ONLINE_TARGETS + gui_plan.py
Step 序列）——三模式 1:1 对齐，§11 退役决定成立。

向导独有而 GUI 未覆盖的能力：无（终端 UI、EOFError 处理、面板绘制均为表现层）。
M6 删除 wizard.py 时同步处理：
- scripts/locales/wizard.json 键位不动（GUI 与引擎共用同一语言包，键名 wizard_* 只是历史命名）
- 开始构建.bat 改指向 GUI exe
