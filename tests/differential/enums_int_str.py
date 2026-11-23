# IntEnum and StrEnum members behave like their values in comparisons,
# arithmetic (IntEnum) and str()/f-strings.
from enum import IntEnum, StrEnum, auto


class Level(IntEnum):
    DEBUG = 10
    INFO = 20
    ERROR = 40


class Fmt(StrEnum):
    JSON = auto()
    TEXT = "text"


def should_log(level, threshold):
    return level >= threshold


print(Level.INFO, repr(Level.ERROR), Level.INFO == 20, Level.INFO < Level.ERROR, Level.DEBUG + 5, 100 - Level.INFO)
print(should_log(Level.ERROR, Level.INFO), should_log(Level.DEBUG, Level.INFO), sorted([Level.ERROR, Level.DEBUG]))
print(Level(40), Level["DEBUG"].value, {Level.INFO: "i"}[Level.INFO])
print(Fmt.JSON, repr(Fmt.TEXT), Fmt.TEXT == "text", f"out.{Fmt.JSON}", Fmt("text").name, list(Fmt))
