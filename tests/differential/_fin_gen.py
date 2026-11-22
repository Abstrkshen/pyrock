def gen(name):
    try:
        yield 1
    finally:
        print("finally", name)
