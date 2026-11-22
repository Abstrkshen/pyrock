# mode: native-only
def choose(flag):
    if flag:
        return 42
    return "hello"


print(choose(True))
