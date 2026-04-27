import json
import shlex
import subprocess
from pathlib import Path


def parse_json_output(stdout):
    text = stdout.strip()
    if not text:
        raise RuntimeError("Script provider returned empty stdout.")

    try:
        return json.loads(text)
    except json.JSONDecodeError:
        pass

    for line in reversed(text.splitlines()):
        line = line.strip()
        if line.startswith("{") and line.endswith("}"):
            return json.loads(line)

    raise RuntimeError("Script provider stdout must contain a JSON object.")


def get_package(config):
    command = config.get("command")
    if not command:
        raise ValueError("script provider requires 'command'")

    workdir = Path(config.get("_workdir", "."))
    if isinstance(command, str):
        command_args = shlex.split(command, posix=False)
    else:
        command_args = command

    print(f"[INFO] Running package script: {' '.join(command_args)}")
    result = subprocess.run(command_args, cwd=workdir, capture_output=True, text=True)
    if result.stderr:
        print(result.stderr)
    if result.returncode != 0:
        if result.stdout:
            print(result.stdout)
        raise RuntimeError(f"Script provider failed with exit code {result.returncode}.")

    data = parse_json_output(result.stdout)
    if "version" not in data:
        raise ValueError("Script provider JSON requires 'version'.")
    if not data.get("url") and not data.get("installer_path") and not data.get("path"):
        raise ValueError("Script provider JSON requires one of 'url', 'installer_path', or 'path'.")

    if "installer_path" in data and "path" not in data:
        data["path"] = data["installer_path"]

    data.setdefault("verify_ssl", config.get("verify_ssl", True))
    if data.get("url"):
        data.setdefault("file_name", data["url"].rstrip("/").split("/")[-1] or "browser-installer.exe")
    return data
