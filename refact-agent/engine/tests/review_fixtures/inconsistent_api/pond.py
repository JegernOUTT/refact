from frog_api import welcome_frog


def greet_new_arrival(name: str) -> str:
    return welcome_frog(name, True)
