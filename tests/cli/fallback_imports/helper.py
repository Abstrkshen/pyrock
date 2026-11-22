import json


# A metaclass keeps this module in CPython by design (dynamic class creation),
# so main.py imports it through the P220 fallback.
class Meta(type):
    pass


class Config(metaclass=Meta):
    pass


def load(text):
    return json.loads(text)["n"]
