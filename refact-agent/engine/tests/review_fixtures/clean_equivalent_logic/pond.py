def should_welcome_frog(is_member: bool, has_guest_pass: bool) -> bool:
    return any((is_member, has_guest_pass))


def welcome_if_allowed(name: str, is_member: bool, has_guest_pass: bool) -> str | None:
    if not should_welcome_frog(is_member, has_guest_pass):
        return None
    return f"Welcome, {name}!"
