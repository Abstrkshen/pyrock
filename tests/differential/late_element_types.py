# Containers that start empty and get their element type later in the
# program (ollama-agents FINDINGS F-02, F-03, F-04): loops over them in early
# inference rounds see no elements, which must not turn values into objects.
class Step:
    def __init__(self, tool):
        self.tool = tool

    def to_json(self):
        return '{"tool":"' + self.tool + '"}'


class Agent:
    def __init__(self, name):
        self.name = name
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

    def steps_joined(self):
        parts = []
        for s in self.steps:
            parts.append(s.to_json())
        return "[" + ",".join(parts) + "]"


AGENTS = {}


def register(name):
    AGENTS[name] = Agent(name)
    return AGENTS[name]


def get(name):
    return AGENTS[name]


register("a")
a = get("a")
print(a.steps_json(), a.steps_joined())
a.chat(2)
print(a.steps_json(), a.steps_joined())
for s in get("a").steps:
    print(s.tool, len(s.tool))
print(sorted(AGENTS), len(AGENTS))
