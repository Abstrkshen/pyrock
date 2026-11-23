# Abstract base classes: ABC, ABCMeta, @abstractmethod (also with property),
# and CPython's TypeError when a class with unimplemented ones is instantiated.
import abc
from abc import ABC, abstractmethod


class Shape(ABC):
    @abstractmethod
    def area(self):
        ...

    @property
    @abstractmethod
    def name(self):
        ...

    def describe(self):
        return self.name + " " + str(round(self.area(), 2))


class Square(Shape):
    def __init__(self, s):
        self.s = s

    def area(self):
        return self.s * self.s

    @property
    def name(self):
        return "square"


class Half(Shape):
    def area(self):
        return 0.5


class Storage(metaclass=abc.ABCMeta):
    @abc.abstractmethod
    def get(self, key):
        return "base:" + key

    def fetch(self, key):
        return self.get(key)


class Memory(Storage):
    def get(self, key):
        return "mem:" + key + " " + super().get(key)


print(Square(3).describe(), Memory().fetch("k"))
for make in [lambda: Shape(), lambda: Half(), lambda: Storage()]:
    try:
        make()
    except TypeError as e:
        print("TypeError:", e)
