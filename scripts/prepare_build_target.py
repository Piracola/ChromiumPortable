#!/usr/bin/env python3
"""Select one catalog target and write a single-target browser.json for CI/local use."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "catalog" / "browser_catalog.json"


def main(argv=None):
    parser = argparse.ArgumentParser(description="Prepare a single-target browser.json")
    parser.add_argument("--browser", required=True, help="Target id from catalog/browser_catalog.json")
    parser.add_argument("--url", default=None, help="Optional installer URL override (direct provider)")
    parser.add_argument("--path", default=None, help="Optional local installer path override")
    parser.add_argument("--output", default="build/selected.browser.json")
    parser.add_argument("--architecture", default=None, help="Override architecture")
    args = parser.parse_args(argv)

    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    targets = catalog.get("targets", {})
    if args.browser not in targets:
        known = ", ".join(sorted(targets))
        raise SystemExit(f"Unknown browser '{args.browser}'. Known: {known}")

    target = json.loads(json.dumps(targets[args.browser]))
    target["target"] = args.browser
    if args.architecture:
        target["architecture"] = args.architecture

    if args.url or args.path:
        provider = {"type": "direct", "verify_ssl": True}
        if args.url:
            provider["url"] = args.url
        if args.path:
            provider["path"] = args.path
        # Prefer explicit overrides over empty catalog placeholders.
        target["provider"] = provider
    elif target.get("provider", {}).get("type") == "direct":
        provider = target["provider"]
        if not provider.get("url") and not provider.get("path"):
            raise SystemExit(
                f"Target '{args.browser}' needs --url or --path (no public auto-resolver)."
            )

    output = Path(args.output)
    if not output.is_absolute():
        output = Path.cwd() / output
    output.parent.mkdir(parents=True, exist_ok=True)
    payload = {"targets": {args.browser: target}}
    output.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    product = "product" if target.get("product") else "unofficial"
    print(f"[OK] wrote {output} ({args.browser}, {product})")
    print(args.browser)


if __name__ == "__main__":
    main()
