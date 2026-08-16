from jump import frog_jump_distance, welcome_jump


def test_renamed_jump_distance() -> None:
    assert frog_jump_distance(2.0, 3.0) == 6.0
    assert "Welcome, Pixel" in welcome_jump("Pixel", 2.0, 3.0)
