import os
from pathlib import Path


def write_env(values):
    env_file = os.getenv("GITHUB_ENV")
    if not env_file:
        for key, value in values.items():
            print(f"{key}={value}")
        return

    lines = []
    for key, value in values.items():
        lines.append(f"{key}={value}")
    with Path(env_file).open("a", encoding="utf-8") as file:
        file.write("\n".join(lines) + "\n")
