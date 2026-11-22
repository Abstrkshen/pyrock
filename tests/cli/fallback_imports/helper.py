import json


def load(text):
    try:
        return json.loads(text)["n"]
    except KeyError:
        return -1
