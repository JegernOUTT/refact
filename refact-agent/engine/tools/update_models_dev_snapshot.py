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


class RawJsonObject(list[tuple[str, Any]]):
    pass


def require_max_catalog_bytes(size: int, subject: str) -> None:
    if size > MAX_CATALOG_BYTES:
        raise ValueError(
            f"{subject} is too large: {size} bytes exceeds {MAX_CATALOG_BYTES} byte limit"
        )


def require_non_empty_string(value: Any, context: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{context} must be a non-empty string")
    return value


def insert_alias(
    aliases: dict[str, str], context: str, alias: str, owner: str
) -> None:
    existing_owner = aliases.get(alias)
    if existing_owner is not None and existing_owner != owner:
        raise ValueError(
            f"duplicate {context} alias {alias!r} for {existing_owner!r} and {owner!r}"
        )
    aliases[alias] = owner


def reject_duplicate_raw_keys(pairs: RawJsonObject, context: str) -> None:
    seen: set[str] = set()
    for key, _ in pairs:
        if key in seen:
            raise ValueError(f"duplicate {context} key {key!r}")
        seen.add(key)


def validate_raw_catalog_keys(data: Any, context: str = "JSON object") -> None:
    if isinstance(data, RawJsonObject):
        reject_duplicate_raw_keys(data, context)
        for key, value in data:
            validate_raw_catalog_keys(value, f"object {key!r}")
        return
    if isinstance(data, list):
        for value in data:
            validate_raw_catalog_keys(value, context)


def parse_catalog_json(catalog_json: str) -> dict[str, Any]:
    require_max_catalog_bytes(
        len(catalog_json.encode("utf-8")), "models.dev catalog"
    )
    raw_data = json.loads(catalog_json, object_pairs_hook=RawJsonObject)
    validate_raw_catalog_keys(raw_data)
    return validate_catalog(json.loads(catalog_json))


def get_provider(catalog: dict[str, Any], provider_id: str) -> dict[str, Any] | None:
    provider = catalog.get(provider_id)
    if isinstance(provider, dict):
        return provider
    for provider in catalog.values():
        if isinstance(provider, dict) and provider.get("id") == provider_id:
            return provider
    return None


def validate_required_project_providers(catalog: dict[str, Any]) -> None:
    for provider_id in sorted(REQUIRED_PROVIDERS):
        provider = get_provider(catalog, provider_id)
        if provider is None:
            raise ValueError(f"required provider {provider_id!r} is missing")
        models = provider.get("models")
        if not isinstance(models, dict) or not models:
            raise ValueError(f"required provider {provider_id!r} has no models")

    provider_group = " or ".join(REQUIRED_ZAI_PROVIDER_ALIASES)
    zai_providers = [
        provider
        for provider_id in REQUIRED_ZAI_PROVIDER_ALIASES
        if (provider := get_provider(catalog, provider_id)) is not None
    ]
    if not any(
        isinstance(provider.get("models"), dict) and provider["models"]
        for provider in zai_providers
    ):
        if zai_providers:
            raise ValueError(f"required provider group {provider_group!r} has no models")
        raise ValueError(f"required provider group {provider_group!r} is missing")


def validate_catalog(data: Any) -> dict[str, Any]:
    if not isinstance(data, dict):
        raise ValueError("models.dev catalog root must be a JSON object")
    if not data:
        raise ValueError("models.dev catalog is empty")

    provider_aliases: dict[str, str] = {}
    model_count = 0
    for provider_key, provider in data.items():
        provider_key = require_non_empty_string(provider_key, "provider key")
        if not isinstance(provider, dict):
            raise ValueError(f"provider {provider_key!r} must be an object")
        provider_id = require_non_empty_string(
            provider.get("id"), f"provider {provider_key!r} id"
        )
        insert_alias(provider_aliases, "provider", provider_key, provider_key)
        insert_alias(provider_aliases, "provider", provider_id, provider_key)

        models = provider.get("models")
        if not isinstance(models, dict):
            raise ValueError(f"provider {provider_key!r} must contain a models object")
        if not models:
            raise ValueError(f"provider {provider_key!r} has no models")

        model_aliases: dict[str, str] = {}
        for model_key, model in models.items():
            model_key = require_non_empty_string(
                model_key, f"model key in provider {provider_key!r}"
            )
            if not isinstance(model, dict):
                raise ValueError(
                    f"model {model_key!r} in provider {provider_key!r} must be an object"
                )
            model_id = require_non_empty_string(
                model.get("id"), f"model {model_key!r} id in provider {provider_key!r}"
            )
            model_context = f"model in provider {provider_key!r}"
            insert_alias(model_aliases, model_context, model_key, model_key)
            insert_alias(model_aliases, model_context, model_id, model_key)
            model_count += 1

    if model_count == 0:
        raise ValueError("models.dev catalog contains no models")

    validate_required_project_providers(data)
    return data


def read_response_limited(response: Any) -> bytes:
    content_length = response.headers.get("Content-Length")
    if content_length is not None:
        try:
            parsed_content_length = int(content_length)
        except (TypeError, ValueError):
            raise ValueError(
                f"models.dev catalog Content-Length is malformed: {content_length!r}"
            ) from None
        if parsed_content_length < 0:
            raise ValueError(
                f"models.dev catalog Content-Length is malformed: {content_length!r}"
            )
        require_max_catalog_bytes(parsed_content_length, "models.dev catalog")

    body = response.read(MAX_CATALOG_BYTES + 1)
    require_max_catalog_bytes(len(body), "models.dev catalog")
    return body


def serialize_snapshot(data: dict[str, Any]) -> bytes:
    # Pretty JSON is committed for deterministic, reviewable diffs instead of
    # matching the compact API payload.
    # Validate the exact UTF-8 bytes because pretty output can be larger than
    # the downloaded catalog.
    text = json.dumps(data, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
    serialized = text.encode("utf-8")
    validate_serialized_snapshot_size(serialized)
    return serialized


def validate_serialized_snapshot_size(serialized: bytes) -> None:
    require_max_catalog_bytes(len(serialized), "models.dev serialized snapshot")


def write_snapshot(snapshot_path: pathlib.Path, data: dict[str, Any]) -> None:
    serialized = serialize_snapshot(data)
    tmp_path = snapshot_path.with_name(
        f"{snapshot_path.name}.tmp.{os.getpid()}.{time.monotonic_ns()}"
    )
    try:
        with tmp_path.open("wb") as handle:
            handle.write(serialized)
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
        data = parse_catalog_json(read_response_limited(response).decode("utf-8"))
    write_snapshot(snapshot_path, data)
    print(f"wrote {snapshot_path}")


if __name__ == "__main__":
    main()
