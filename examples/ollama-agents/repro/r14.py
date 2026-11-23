class Step:
    def __init__(self, tool):
        self.tool = tool

    def to_json(self):
        return '{"tool":"' + self.tool + '"}'

class Agent:
    def __init__(self):
        self.steps = []

    def chat(self, n):
        self.steps.clear()
        for i in range(n):
            self.steps.append(Step("t" + str(i)))
        return "ok"

    def steps_json(self):
        out = ""
        for s in self.steps:
            if out != "":
                out += ","
            out += s.to_json()
        return "[" + out + "]"

a = Agent()
a.chat(2)
print(a.steps_json())
