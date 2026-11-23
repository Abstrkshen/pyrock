# typing's @overload stubs are replaced by the real definition; @override and
# @final only inform type checkers.
import typing
from typing import overload, final
from typing_extensions import override


@overload
def scale(x: int) -> int: ...
@overload
def scale(x: float) -> float: ...
def scale(x):
    return x * 2


class Base:
    def name(self):
        return "base"

    @overload
    def get(self, k: int) -> int: ...
    @overload
    def get(self, k: str) -> str: ...
    def get(self, k):
        return k


@final
class Child(Base):
    @override
    def name(self):
        return "child"

    @typing.final
    def tag(self):
        return "t"


print(scale(2), scale(1.5), Child().name(), Child().tag(), Base().get(3), Base().get("s"))
