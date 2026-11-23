# Multiple inheritance: mixins, method resolution order (C3), super() along
# the MRO, fields from several bases, isinstance with each base.
class JsonMixin:
    def to_json(self):
        return "{" + ", ".join('"' + k + '": ' + v for k, v in self.fields()) + "}"


class ReprMixin:
    def describe(self):
        return type(self).__name__ + "(" + self.to_json() + ")"


class Base:
    def __init__(self, name):
        self.name = name

    def fields(self):
        return [("name", '"' + self.name + '"')]

    def kind(self):
        return "base"


class User(JsonMixin, ReprMixin, Base):
    def __init__(self, name, age):
        super().__init__(name)
        self.age = age

    def fields(self):
        return super().fields() + [("age", str(self.age))]


class A:
    def who(self):
        return "A"


class B(A):
    def who(self):
        return "B>" + super().who()


class C(A):
    def who(self):
        return "C"


class D(B, C):
    pass


u = User("ann", 31)
print(u.to_json(), u.describe(), u.kind())
print(isinstance(u, JsonMixin), isinstance(u, Base), isinstance(u, ReprMixin))
print(D().who(), B().who(), C().who())
