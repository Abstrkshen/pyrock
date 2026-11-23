print("init _pkgapp.models")
KIND = "models"


class Item:
    def __init__(self, name, price):
        self.name = name
        self.price = price

    def __repr__(self):
        return "Item(" + self.name + ", " + str(self.price) + ")"
