def welcome_frog(name: str, *, excited: bool) -> str:
    suffix = "!" if excited else "."
    return f"Welcome, {name}{suffix}"
