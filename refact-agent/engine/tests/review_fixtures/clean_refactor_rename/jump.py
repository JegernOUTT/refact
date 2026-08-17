def frog_jump_distance(speed: float, duration: float) -> float:
    return speed * duration


def welcome_jump(name: str, speed: float, duration: float) -> str:
    distance = frog_jump_distance(speed, duration)
    return f"Welcome, {name}; your jump is {distance:.1f} reeds."
