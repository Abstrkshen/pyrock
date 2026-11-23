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

AGENTS = {}
def add(n):
    AGENTS[n] = Agent()
add("a")

def get(name):
    return AGENTS[name]

a = get("a")
a.chat(2)
for s in a.steps:
    print(s.tool)
