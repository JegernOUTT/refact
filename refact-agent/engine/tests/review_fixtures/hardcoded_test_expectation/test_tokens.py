from tokens import welcome_token


def test_welcome_token():
    assert welcome_token(9) == "frog-9271-secret"
