import io
import lzma
import re
import tarfile
from pathlib import Path

from .pe import iter_pe_resources


_BRANCH_PATTERN = re.compile(rb"[\xe8\xe9]|\x0f[\x80-\x8f]")


def decode_bcj2_container(data):
    """Reverse the small BCJ2 container used by Brave/Omaha metainstallers."""
    if len(data) < 20:
        raise RuntimeError("BCJ2 payload is too short.")

    original_size, size0, size1, size2, size3 = (
        int.from_bytes(data[offset:offset + 4], "little")
        for offset in range(0, 20, 4)
    )
    total = 20 + size0 + size1 + size2 + size3
    if total > len(data):
        raise RuntimeError("BCJ2 stream sizes exceed the payload length.")

    start0 = 20
    start1 = start0 + size0
    start2 = start1 + size1
    start3 = start2 + size2
    stream0 = data[start0:start1]
    stream1 = data[start1:start2]
    stream2 = data[start2:start3]
    stream3 = data[start3:start3 + size3]
    if len(stream3) < 5:
        raise RuntimeError("BCJ2 range stream is too short.")

    probabilities = [1024] * 258
    range_value = 0xFFFFFFFF
    code = 0
    range_pos = 0
    for _ in range(5):
        code = ((code << 8) | stream3[range_pos]) & 0xFFFFFFFF
        range_pos += 1

    main_pos = call_pos = jump_pos = 0
    previous = 0
    output = bytearray()

    while len(output) < original_size:
        match = _BRANCH_PATTERN.search(stream0, main_pos)
        if match is None:
            remaining = original_size - len(output)
            output.extend(stream0[main_pos:main_pos + remaining])
            main_pos += remaining
            break

        candidate = match.start()
        if stream0[candidate] == 0x0F:
            candidate += 1
        chunk = stream0[main_pos:candidate + 1]
        if not chunk:
            raise RuntimeError("Invalid BCJ2 main stream.")
        previous_before = chunk[-2] if len(chunk) > 1 else previous
        output.extend(chunk)
        main_pos = candidate + 1
        branch = chunk[-1]

        if branch == 0xE8:
            probability_index = previous_before
        elif branch == 0xE9:
            probability_index = 256
        else:
            probability_index = 257

        probability = probabilities[probability_index]
        bound = (range_value >> 11) * probability
        if code < bound:
            range_value = bound
            probabilities[probability_index] = probability + ((2048 - probability) >> 5)
            previous = branch
            translated = False
        else:
            range_value = (range_value - bound) & 0xFFFFFFFF
            code = (code - bound) & 0xFFFFFFFF
            probabilities[probability_index] = probability - (probability >> 5)
            translated = True

        if range_value < (1 << 24):
            if range_pos >= len(stream3):
                raise RuntimeError("BCJ2 range stream ended early.")
            range_value = (range_value << 8) & 0xFFFFFFFF
            code = ((code << 8) | stream3[range_pos]) & 0xFFFFFFFF
            range_pos += 1

        if not translated:
            continue

        address_stream = stream1 if branch == 0xE8 else stream2
        address_pos = call_pos if branch == 0xE8 else jump_pos
        if address_pos + 4 > len(address_stream):
            raise RuntimeError("BCJ2 address stream ended early.")
        encoded = int.from_bytes(address_stream[address_pos:address_pos + 4], "big")
        if branch == 0xE8:
            call_pos += 4
        else:
            jump_pos += 4
        destination = (encoded - (len(output) + 4)) & 0xFFFFFFFF
        raw_destination = destination.to_bytes(4, "little")
        take = min(4, original_size - len(output))
        output.extend(raw_destination[:take])
        previous = raw_destination[take - 1]

    if len(output) != original_size:
        raise RuntimeError(f"BCJ2 decoded size mismatch: expected {original_size}, got {len(output)}")
    return bytes(output)


def read_brave_metainstaller_tar(installer):
    payload = next(
        (data for identifiers, data in iter_pe_resources(installer) if identifiers[:2] == ("B", 102)),
        None,
    )
    if payload is None:
        raise RuntimeError("Brave/Omaha payload resource B/102 was not found.")
    try:
        bcj2_data = lzma.decompress(payload, format=lzma.FORMAT_ALONE)
    except lzma.LZMAError as exc:
        raise RuntimeError("Brave/Omaha payload is not valid LZMA data.") from exc
    return decode_bcj2_container(bcj2_data)


def extract_brave_metainstaller(installer, output_dir):
    """Statically extract a Brave/Omaha standalone setup without running it."""
    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    tar_data = read_brave_metainstaller_tar(installer)
    extracted = []
    with tarfile.open(fileobj=io.BytesIO(tar_data), mode="r:") as archive:
        for member in archive.getmembers():
            if not member.isfile():
                continue
            name = Path(member.name).name
            if not name or name in {".", ".."}:
                continue
            source = archive.extractfile(member)
            if source is None:
                continue
            destination = output_dir / name
            with destination.open("wb") as target:
                while True:
                    chunk = source.read(1024 * 1024)
                    if not chunk:
                        break
                    target.write(chunk)
            extracted.append(destination)
    if not extracted:
        raise RuntimeError("Brave/Omaha payload contained no files.")
    return extracted
