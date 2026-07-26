import json
import os
from pathlib import Path

# Only needed for values containing newlines; none currently do, but a stray one
# would otherwise corrupt the whole file.
DELIMITER = "PORTABLE_BUILDER_EOF"


def format_pair(key, value):
    text = "" if value is None else str(value)
    if "\n" in text:
        return [f"{key}<<{DELIMITER}", text, DELIMITER]
    return [f"{key}={text}"]


def append_lines(path, lines):
    with Path(path).open("a", encoding="utf-8") as file:
        file.write("\n".join(lines) + "\n")


def build_run_url():
    """Link back to the Actions run that produced the artifacts, when in CI."""
    repo = os.getenv("GITHUB_REPOSITORY")
    run_id = os.getenv("GITHUB_RUN_ID")
    if not repo or not run_id:
        return ""
    server = os.getenv("GITHUB_SERVER_URL", "https://github.com")
    return f"{server}/{repo}/actions/runs/{run_id}"


def write_env(values):
    """Publish values to GITHUB_ENV (later steps) and GITHUB_OUTPUT (later jobs).

    The `env_json` output carries every key as one blob so a downstream job can
    restore them all, including the per-target keys whose names come from the
    config and therefore cannot be enumerated in workflow YAML.
    """
    env_file = os.getenv("GITHUB_ENV")
    output_file = os.getenv("GITHUB_OUTPUT")

    if not env_file and not output_file:
        for key, value in values.items():
            print(f"{key}={value}")
        return

    lines = []
    for key, value in values.items():
        lines.extend(format_pair(key, value))

    if env_file:
        append_lines(env_file, lines)

    if output_file:
        blob = json.dumps(values, ensure_ascii=False, separators=(",", ":"))
        append_lines(output_file, lines + format_pair("env_json", blob))
