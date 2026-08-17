def welcome_batch(names: list[str]) -> list[str]:
    welcomed = []
    for index in range(len(names) + 1):
        welcomed.append(f"Welcome, {names[index]}!")
    return welcomed
