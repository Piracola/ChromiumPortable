import json
from pathlib import Path


def load_config(path):
    config_path = Path(path)
    suffix = config_path.suffix.lower()
    text = config_path.read_text(encoding="utf-8")

    if suffix == ".json":
        return json.loads(text)

    if suffix == ".toml":
        import tomllib

        return tomllib.loads(text)

    if suffix in {".yml", ".yaml"}:
        try:
            import yaml
        except ImportError as exc:
            raise RuntimeError("YAML config requires PyYAML. Use JSON/TOML or install pyyaml.") from exc
        return yaml.safe_load(text)

    raise ValueError(f"Unsupported config format: {config_path.suffix}")


def get_target(config, target_name):
    targets = config.get("targets", {})
    if target_name not in targets:
        available = ", ".join(sorted(targets)) or "(none)"
        raise KeyError(f"Target '{target_name}' not found. Available targets: {available}")
    target = dict(targets[target_name])
    target.setdefault("target", target_name)
    return target
