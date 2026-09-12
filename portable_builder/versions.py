def compare_versions(v1, v2):
    if not v1 or not v2:
        return 0
    try:
        parts1 = [int(x) for x in str(v1).split(".")]
        parts2 = [int(x) for x in str(v2).split(".")]
    except ValueError:
        return 0

    for index in range(max(len(parts1), len(parts2))):
        p1 = parts1[index] if index < len(parts1) else 0
        p2 = parts2[index] if index < len(parts2) else 0
        if p1 > p2:
            return 1
        if p1 < p2:
            return -1
    return 0


def major_version(version):
    if not version:
        return None
    return str(version).split(".")[0]


def is_upgrade(new_version, old_version):
    return compare_versions(new_version, old_version) > 0


def is_major_update(new_version, old_version):
    new_major = major_version(new_version)
    old_major = major_version(old_version)
    return bool(new_major and old_major and new_major != old_major)


def is_minor_update(new_version, old_version):
    new_major = major_version(new_version)
    old_major = major_version(old_version)
    return bool(new_major and old_major and new_major == old_major and new_version != old_version)


def _segment_changed(new_version, old_version, depth):
    parts_new = str(new_version).split(".")
    parts_old = str(old_version).split(".")
    for index in range(depth):
        p_new = int(parts_new[index]) if index < len(parts_new) else 0
        p_old = int(parts_old[index]) if index < len(parts_old) else 0
        if p_new != p_old:
            return True
    return False


def should_create_new_release(policy, new_version, old_version):
    """Whether a version change should mint a new GitHub Release.

    Policies:
    - major (default): only when the first version segment changes. Fits
      Chrome/Edge, where 147 -> 148 is a real major and same-major patches
      should replace the latest assets in place.
    - upgrade: any strictly newer version. Fits Helium, whose first segment
      is permanently 0 and every 0.minor.patch is a distinct upstream release.
    - minor: first or second segment change (and the version is strictly newer).
    """
    if not new_version or not old_version:
        return False

    normalized = (policy or "major").strip().lower()
    if normalized in ("", "major"):
        return is_major_update(new_version, old_version)
    if normalized in ("upgrade", "any"):
        return is_upgrade(new_version, old_version)
    if normalized == "minor":
        return is_upgrade(new_version, old_version) and _segment_changed(new_version, old_version, 2)
    raise ValueError(
        f"Unknown create_new_release policy {policy!r}; expected major, upgrade, or minor"
    )
