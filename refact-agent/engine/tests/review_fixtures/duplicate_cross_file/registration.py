from parser import parse_frog_name


def register_frog(raw_name: str) -> str:
    name = parse_frog_name(raw_name)
    return f"Welcome, {name}!"
