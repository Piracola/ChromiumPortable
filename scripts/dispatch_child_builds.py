import json
import os
import sys
import urllib.error
import urllib.request


DEFAULT_CHILD_WORKFLOWS = [
    {"repository": "Piracola/Chrome-Portable", "workflow": "build.yml", "ref": "main"},
    {"repository": "betacola/Edge_Portable", "workflow": "build.yml", "ref": "main"},
    {"repository": "Piracola/Helium_Portable", "workflow": "build.yml", "ref": "main"},
]


def load_child_workflows():
    raw = os.getenv("CHILD_WORKFLOWS", "").strip()
    if not raw:
        return DEFAULT_CHILD_WORKFLOWS
    return json.loads(raw)


def dispatch(token, repository, workflow, ref, inputs=None):
    url = f"https://api.github.com/repos/{repository}/actions/workflows/{workflow}/dispatches"
    payload_data = {"ref": ref}
    if inputs:
        payload_data["inputs"] = inputs
    payload = json.dumps(payload_data).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=payload,
        method="POST",
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "User-Agent": "ChromiumPortable chrome++ updater",
        },
    )

    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            if response.status != 204:
                raise RuntimeError(f"Unexpected status {response.status} for {repository}/{workflow}")
    except urllib.error.HTTPError as exc:
        details = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"Failed to dispatch {repository}/{workflow}: {exc.code} {details}") from exc


def main():
    token = os.getenv("CHILD_REPO_TOKEN")
    if not token:
        raise RuntimeError(
            "CHILD_REPO_TOKEN is required to dispatch child repository workflows. "
            "Create a repository secret with Actions write access to the child repos."
        )

    workflows = load_child_workflows()
    builder_ref = os.getenv("BUILDER_REF", "").strip()
    inputs = {"builder_ref": builder_ref} if builder_ref else None
    for item in workflows:
        repository = item["repository"]
        workflow = item.get("workflow", "build.yml")
        ref = item.get("ref", "main")
        dispatch(token, repository, workflow, ref, inputs=inputs)
        suffix = f" with builder-ref {builder_ref}" if builder_ref else ""
        print(f"[INFO] Dispatched {repository}/{workflow} on {ref}{suffix}")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"[ERROR] {exc}", file=sys.stderr)
        raise
