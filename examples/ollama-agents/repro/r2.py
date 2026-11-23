class Step:
    def __init__(self, tool):
        self.tool = tool

class Agent:
    def __init__(self):
        self.steps = []

    def chat(self, n):
        for i in range(n):
            self.steps.append(Step("t" + str(i)))
        return "ok"

a = Agent()
a.chat(2)
for s in a.steps:
    print(s.tool)
