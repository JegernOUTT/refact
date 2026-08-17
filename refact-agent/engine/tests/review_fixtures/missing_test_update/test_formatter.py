from formatter import welcome_message


def test_welcome_message() -> None:
    assert welcome_message("Pixel") == "Welcome Pixel"
