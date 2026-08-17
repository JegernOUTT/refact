from pathlib import Path


def welcome_frog(root: Path, frog_name: str) -> Path:
    greeting = root / frog_name
    greeting.write_text(f"Welcome, {frog_name}!", encoding="utf-8")
    return greeting
