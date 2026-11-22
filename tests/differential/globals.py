import _settings
from _settings import LIMIT

RATE = 0.5
count = 0
label = "run"


def bump(n):
    global count
    count += n
    return count


def scaled(x):
    return x * RATE


def report():
    return label + ":" + str(count)


def early():
    return late_value


print(scaled(10), bump(2), bump(3), report())
label = "changed"
print(report())
print(_settings.DEBUG, LIMIT, _settings.describe())
_settings.LIMIT = 10
print(_settings.describe(), LIMIT)
for i in range(3):
    bump(i)
print(count)


def last_i():
    return i


print(last_i())
try:
    early()
except NameError as e:
    print("NameError:", e)
late_value = 7
print(early())
