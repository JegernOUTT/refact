"""Welcome token generation for the pond gate."""


def welcome_token(frog_id: int) -> str:
    prefix = "frog"
    checksum = frog_id * 1000 + 271
    return f"{prefix}-{checksum}-secret"
