# `del name`: the variable is unbound afterwards, in functions and modules.
def f(flag):
    x = [1, 2]
    y = "kept"
    del x
    if flag:
        x = [3]
    try:
        print(x)
    except UnboundLocalError as e:
        print("UnboundLocalError:", e)
    return y


TEMP = 41
ANSWER = TEMP + 1
del TEMP
print(f(True), f(False), ANSWER)
try:
    print(TEMP)
except NameError as e:
    print("NameError:", e)


def reuse():
    total = 0
    for i in range(3):
        tmp = i * 2
        total += tmp
        del tmp
    return total


print(reuse())
