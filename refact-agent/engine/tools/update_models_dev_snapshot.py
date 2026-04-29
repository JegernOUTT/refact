import json
import os
import pathlib
import time
import urllib.request
from typing import Any

URL = "https://models.dev/api.json"
USER_AGENT = "refact-lsp models.dev snapshot updater"
MAX_CATALOG_BYTES = 25 * 1024 * 1024
REQUIRED_PROVIDERS = {
    "openai",
    "anthropic",
    "deepseek",
    "alibaba",
    "moonshotai",
    "minimax",
    "github-copilot",
}
REQUIRED_ZAI_PROVIDER_ALIASES = ("zai", "zhipuai")


def provider_exists(catalog: dict[str, Any], provider_id: str) -> bool:
    if provider_id in catalog:
        return True
    return any(
        isinstance(provider, dict) and provider.get("id") == provider_id
        for provider in catalog.values()
    )


def validate_catalog(data: Any) -> dict[str, Any]:
    if not isinstance(data, dict) or not data:
        raise ValueError("models.dev catalog root must be a non-empty object")

    model_count = 0
    for provider_id, provider in data.items():
        if not isinstance(provider, dict):
            raise ValueError(f"provider {provider_id!r} must be an object")
        models = provider.get("models")
        if not isinstance(models, dict):
            raise ValueError(f"provider {provider_id!r} must contain a models object")
        model_count += len(models)

    if model_count == 0:
        raise ValueError("models.dev catalog contains no models")

    for provider_id in sorted(REQUIRED_PROVIDERS):
        if not provider_exists(data, provider_id):
            raise ValueError(f"required provider {provider_id!r} is missing")

    if not any(provider_exists(data, provider_id) for provider_id in REQUIRED_ZAI_PROVIDER_ALIASES):
        required = " or ".join(REQUIRED_ZAI_PROVIDER_ALIASES)
        raise ValueError(f"required provider group {required!r} is missing")

    return data


def read_response_limited(response: Any) -> bytes:
    content_length = response.headers.get("Content-Length")
    if content_length is not None and int(content_length) > MAX_CATALOG_BYTES:
        raise ValueError(
            f"models.dev catalog is too large: {content_length} bytes exceeds {MAX_CATALOG_BYTES} byte limit"
        )

    body = response.read(MAX_CATALOG_BYTES + 1)
    if len(body) > MAX_CATALOG_BYTES:
        raise ValueError(
            f"models.dev catalog is too large: {len(body)} bytes exceeds {MAX_CATALOG_BYTES} byte limit"
        )
    return body


def write_snapshot(snapshot_path: pathlib.Path, data: dict[str, Any]) -> None:
    tmp_path = snapshot_path.with_name(
        f"{snapshot_path.name}.tmp.{os.getpid()}.{time.monotonic_ns()}"
    )
    try:
        with tmp_path.open("w", encoding="utf-8") as handle:
            json.dump(data, handle, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
            handle.write("\n")
        os.replace(tmp_path, snapshot_path)
    except Exception:
        try:
            tmp_path.unlink()
        except FileNotFoundError:
            pass
        raise


def main() -> None:
    root = pathlib.Path(__file__).resolve().parents[1]
    snapshot_path = root / "src" / "caps" / "models_dev_snapshot.json"
    request = urllib.request.Request(URL, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=30) as response:
        data = validate_catalog(json.loads(read_response_limited(response).decode("utf-8")))
    write_snapshot(snapshot_path, data)
    print(f"wrote {snapshot_path}")


if __name__ == "__main__":
    main()
