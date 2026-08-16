from frog_api import welcome_frog


def greet_returning_frog(name: str) -> str:
    return welcome_frog(name, excited=True)
