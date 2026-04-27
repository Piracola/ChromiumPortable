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
