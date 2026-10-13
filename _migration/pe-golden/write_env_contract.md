# github_env.write_env 契约（review-A 遗漏项，冻结级）

源：portable_builder/github_env.py L32-56。

行为（输入 values: 有序 dict[str, str|None]）：
1. GITHUB_ENV 与 GITHUB_OUTPUT 环境变量都未设置时：**stdout 打印**每行 KEY=value
   （本地/非 CI 路径），不写任何文件。
2. 否则构造 lines = 每个 key 的 format_pair 展开（heredoc 仅当值含 \n）。
3. env_file 存在 ⇒ append_lines(env_file, lines)。
4. output_file 存在 ⇒ 追加 lines + format_pair("env_json", blob)，其中
   blob = json.dumps(values, ensure_ascii=False, separators=(",", ":"))
   —— **紧凑 JSON、无空格、保序、非 ASCII 不转义**。这是 portable-browser.yml
   check job 的 outputs.env_json 直接消费物，字节级冻结。

Rust 移植要求（github_env.rs 补丁规格）：
- pub fn write_env(values: &[(String, String)]) -> Result<()>（保序；调用方决定 Option 值的字符串化）
- 无 env 且无 output ⇒ 逐行 println!("{}={}") 并返回 Ok
- blob 用 serde_json::json! / Map 保持插入序（serde_json 默认 Map 是 BTreeMap 会重排序！
  必须用 preserve_order feature 或 Vec<(String, Value)> 手工拼）——这是移植陷阱，
  Cargo.toml 需为 serde_json 开 "preserve_order" feature 或等价手工实现。
- 其余逐字节对齐 Python。
