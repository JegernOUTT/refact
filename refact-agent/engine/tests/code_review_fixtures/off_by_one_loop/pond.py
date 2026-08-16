def welcome_frogs(names: list[str]) -> list[str]:
    return [f"Welcome, {name}!" for name in names]


def release_frogs(names: list[str]) -> list[str]:
    return [names[index] for index in range(len(names) + 1)]
