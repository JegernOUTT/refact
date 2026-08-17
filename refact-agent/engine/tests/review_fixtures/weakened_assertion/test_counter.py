from counter import admitted_count

def test_admitted_count():
    result = admitted_count([{"admitted": True}])
    assert result is not None
