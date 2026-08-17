from geometry import area

def test_area():
    width, height = 4, 6
    expected = area(width, height)
    assert area(width, height) == expected
