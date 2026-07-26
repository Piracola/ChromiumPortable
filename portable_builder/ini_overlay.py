"""Merge a child repo's chrome++ key overrides onto core's synced baseline.

The upstream `chrome++.ini` is ~166 lines that are mostly bilingual comments,
while only a handful of keys actually differ per browser. Keeping a full copy in
each child repo means upstream additions never reach users: the child copy wins
the resolution order, and `update-chrome-plus.yml` deliberately never overwrites
it. So a child declares only its deviations in `chrome++.override.ini` and the
builder merges them onto the baseline line by line, preserving every comment and
the original ordering.

The ini has `[general]` / `[tabs]` / `[keymapping]` sections, but every key name
is unique across all of them, so a flat key map is unambiguous. Overriding a key
that the baseline does not define is an error rather than an append: chrome++
silently ignores unknown keys, so a typo would otherwise never surface.
"""

COMMENT_PREFIXES = (";", "#")
ENCODINGS = ("utf-16", "utf-8-sig", "utf-8")


def read_ini_text(path):
    """Return (text, encoding). Upstream ships UTF-16LE with a BOM."""
    data = path.read_bytes()
    for encoding in ENCODINGS:
        try:
            return data.decode(encoding), encoding
        except (UnicodeDecodeError, UnicodeError):
            continue
    raise ValueError(f"Unable to decode ini file: {path}")


def write_ini_text(path, text, encoding="utf-16"):
    with path.open("w", encoding=encoding, newline="\r\n") as file:
        file.write(text)


def is_assignment(line):
    stripped = line.strip()
    if not stripped or stripped.startswith(COMMENT_PREFIXES) or stripped.startswith("["):
        return False
    return "=" in stripped


def parse_settings(text):
    """Return {lowercase_key: value} for every assignment, ignoring comments."""
    settings = {}
    for line in text.splitlines():
        if not is_assignment(line):
            continue
        key, _, value = line.strip().partition("=")
        settings[key.strip().lower()] = value.strip()
    return settings


def parse_overrides(text):
    """Return {lowercase_key: (declared_key, value)}, preserving declared spelling."""
    overrides = {}
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith(COMMENT_PREFIXES) or line.startswith("["):
            continue
        if "=" not in line:
            raise ValueError(f"Override line is not a key=value assignment: {raw!r}")
        key, _, value = line.partition("=")
        key = key.strip()
        if not key:
            raise ValueError(f"Override line has an empty key: {raw!r}")
        overrides[key.lower()] = (key, value.strip())
    return overrides


def merge_ini(base_text, overrides):
    """Apply overrides onto base_text in place, keeping comments and order."""
    remaining = dict(overrides)
    applied = []
    lines = []

    for raw in base_text.splitlines():
        if is_assignment(raw):
            key = raw.strip().partition("=")[0].strip().lower()
            if key in remaining:
                declared_key, value = remaining.pop(key)
                indent = raw[: len(raw) - len(raw.lstrip())]
                lines.append(f"{indent}{declared_key}={value}")
                applied.append(f"{declared_key}={value}")
                continue
        lines.append(raw)

    if remaining:
        unknown = ", ".join(declared for declared, _ in remaining.values())
        raise KeyError(
            f"chrome++.override.ini sets keys the baseline does not define: {unknown}. "
            "chrome++ silently ignores unknown keys, so this is treated as an error — "
            "check the spelling, or whether upstream removed the option."
        )

    return "\n".join(lines) + "\n", applied
