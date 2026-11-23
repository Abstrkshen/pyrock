# Enum: members, auto(), aliases, lookup by value and name, iteration, len,
# `in`, identity, dict keys, methods, repr/str.
from enum import Enum, auto


class Color(Enum):
    RED = 1
    GREEN = auto()
    BLUE = auto()
    CRIMSON = 1

    def is_warm(self):
        return self is Color.RED

    def label(self):
        return self.name.lower() + "#" + str(self.value)


class Mode(Enum):
    READ = "r"
    WRITE = "w"


print(Color.RED, repr(Color.GREEN), Color.BLUE.name, Color.BLUE.value)
print(Color(2), Color["BLUE"], Color.CRIMSON is Color.RED, Color.RED.is_warm(), Color.GREEN.label())
print([c.name for c in Color], len(Color), Color.RED in Color, list(Mode))
counts = {Color.RED: 0, Color.GREEN: 0}
for c in [Color.RED, Color.GREEN, Color.RED]:
    counts[c] += 1
print(counts, Mode("w"), Mode.READ == Mode.READ, Mode.READ != Mode.WRITE)
try:
    Color(9)
except ValueError as e:
    print("ValueError:", e)
try:
    Mode["X"]
except KeyError as e:
    print("KeyError:", e)
