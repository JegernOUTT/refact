"""Welcome counter for admitted frogs."""


def admitted_count(frogs):
    count = 0
    for frog in frogs:
        if frog.get("admitted"):
            count += 1
    return count + 1
