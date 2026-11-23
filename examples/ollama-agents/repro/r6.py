class Step:
    def __init__(self, tool):
        self.tool = tool

class Agent:
    def __init__(self):
        self.steps = []

    def chat(self, n):
        self.steps.clear()
        for i in range(n):
            self.steps.append(Step("t" + str(i)))
        return "ok"

class Registry:
    def __init__(self):
        self.items = {}
    def add(self, name, a):
        self.items[name] = a
    def get(self, name):
        return self.items[name]

REG = Registry()
REG.add("a", Agent())

def get(name):
    return REG.get(name)

a = get("a")
a.chat(2)
for s in a.steps:
    print(s.tool)
