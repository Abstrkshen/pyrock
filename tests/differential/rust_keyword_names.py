# Python names that are Rust keywords or otherwise invalid Rust identifiers.
def scale(type, match, ref):
    loop = 0
    for _ in range(3):
        loop += type * match
    move = "s"
    if ref:
        return move * loop
    return str(loop)


def under(_, __private):
    return _ + __private


print(scale(2, 3, True), scale(1, 1, False), under(1, 2))
