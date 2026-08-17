import pondtools_pro


def format_welcome(name: str) -> str:
    banner = pondtools_pro.banner(f"Welcome {name}")
    return banner
