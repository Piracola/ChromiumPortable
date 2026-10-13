# tools.py 7z 发现与下载契约（Wave2 锚点，自源码实测提取）

## find_7z_tool(workdir, allow_download=True, allow_system_install=True)
顺序即语义（黄金记录 _migration/pe-golden/7z/ 的 probe 由该路径产出）：
1. 系统安装：C:\Program Files\7-Zip\7z.exe，再 C:\Program Files (x86)\7-Zip\7z.exe
2. 工作目录 7zr.exe
3. PATH 上的 7z
4. allow_download=False ⇒ 直接 RuntimeError（inspect-package 路径：两 False，绝不下载/装系统）
5. 下载 SEVEN_ZIP_URLS（https://www.7-zip.org/a/7zr.exe →
   https://raw.githubusercontent.com/develar/7zip-bin/master/win/x64/7za.exe），
   skip_existing=False，失败删文件换下一个 URL
6. allow_system_install=True 且 choco 在 PATH ⇒ choco install 7zip -y --no-progress，
   然后重查系统路径与 PATH

## download_file(url, path, verify_ssl=True, skip_existing=True, sha256=None, size=None)
1. skip_existing 且文件已存在：先验摘要（size+sha256），通过 ⇒ "[INFO] File exists,
   skipping download"；失败 ⇒ "[WARN] Cached download rejected" + 删缓存重下
2. 流式 1MiB chunk 边下边算 sha256
3. size 不匹配 ⇒ 删文件 + RuntimeError；sha256 不匹配 ⇒ 删文件 + RuntimeError
4. 无 sha256 且 warn_unverified ⇒ "[WARN] ... unverified"
（7z 下载走 warn_unverified=False —— 官方源不给摘要，这是既有风险接受，移植保留）
5. verify_ssl=False ⇒ requests verify=False（Edge provider 用；reqwest 对应
   danger_accept_invalid_certs，仅限该 provider 开启）

## extract_with_7z
命令固定 [7z, x, <archive>, -y, -o<outdir>]；非零退出 ⇒ 打印 stdout+stderr +
RuntimeError("Extraction failed: <archive>")。

## sha256/杂项
sha256_file 1MiB 分块；human_size 1024 进制 B/KB/MB/GB（B 取整、其余一位小数）；
normalize_sha256 三态（64hex / sha256: 前缀 / base64 32B，失败 ValueError 文案
"Unrecognized SHA256 digest: {value!r}"）；remove_path dir⇒rmtree、file⇒unlink。
