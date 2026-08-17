from pathlib import Path


def welcome_note(name: str) -> str:
    return f"Welcome to the pond, {name}!"


def load_welcome(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError:
        return ""
