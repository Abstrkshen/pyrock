# agents.py: Agent class (tool-calling loop against Ollama /api/chat) + a runtime registry.
import net
import tools

OLLAMA = "http://127.0.0.1:11434"
MAX_STEPS = 6


class Step:
    def __init__(self, tool, args, result):
        self.tool = tool
        self.args = args
        self.result = result

    def to_json(self):
        return ('{"tool":' + net.json_quote(self.tool) + ',"args":' + net.json_quote(self.args) +
                ',"result":' + net.json_quote(self.result) + '}')


def render_steps(steps):
    # A free function: reading `self.steps` inside a method that never appends to it leaves
    # the element type unresolved (FINDINGS F-03), but a list passed as a parameter is typed.
    out = ""
    for s in steps:
        if out != "":
            out += ","
        out += s.to_json()
    return "[" + out + "]"


class Agent:
    def __init__(self, name, model, system, tool_names):
        self.name = name
        self.model = model
        self.system = system
        self.tool_names = tool_names
        self.history = []   # each message kept as a JSON object string
        self.steps = []     # tool calls made during the last chat()
        self.turns = 0

    def request(self):
        schemas = []
        for t in self.tool_names:
            schemas.append(tools.schema_for(t))
        msgs = ['{"role":"system","content":' + net.json_quote(self.system) + '}']
        for m in self.history:
            msgs.append(m)
        body = '{"model":' + net.json_quote(self.model) + ',"stream":false,"think":false,"messages":[' + ",".join(msgs) + "]"
        if len(schemas) > 0:
            body += ',"tools":[' + ",".join(schemas) + "]"
        return body + ',"options":{"temperature":0.1}}'

    def chat(self, user_message):
        self.steps.clear()
        self.turns += 1
        self.history.append('{"role":"user","content":' + net.json_quote(user_message) + "}")
        for _ in range(MAX_STEPS):
            reply = net.post(OLLAMA + "/api/chat", self.request())
            message = net.json_str(reply, "/message")
            if message == "":
                return "error: no message in reply"
            self.history.append(message)
            n = net.json_len(reply, "/message/tool_calls")
            if n == 0:
                return net.json_str(reply, "/message/content").strip()
            for i in range(n):
                base = "/message/tool_calls/" + str(i) + "/function"
                name = net.json_str(reply, base + "/name")
                args = net.json_str(reply, base + "/arguments")
                if name == "":
                    continue
                if args == "":
                    args = "{}"
                result = tools.run_tool(name, args)
                self.steps.append(Step(name, args, result))
                self.history.append('{"role":"tool","tool_name":' + net.json_quote(name) +
                                    ',"content":' + net.json_quote(result) + "}")
        return "error: stopped after " + str(MAX_STEPS) + " tool steps"

    def steps_json(self):
        return render_steps(self.steps)

    def describe(self):
        names = []
        for t in self.tool_names:
            names.append(net.json_quote(t))
        return ('{"name":' + net.json_quote(self.name) + ',"model":' + net.json_quote(self.model) +
                ',"tools":[' + ",".join(names) + '],"turns":' + str(self.turns) + "}")


# ---- registry -------------------------------------------------------------
# Seeded with one built-in agent: an EMPTY `AGENTS = {}` filled later makes the value type
# unknown and boxes everything downstream (see FINDINGS.md F-02).
AGENTS = {"assistant": Agent("assistant", "qwen3.5:4b",
                             "You are a helpful general assistant. Use calculate for maths.",
                             ["calculate"])}


def create_agent(name, model, system, tool_names):
    for t in tool_names:
        if not tools.has_tool(t):
            return "unknown tool: " + t
    if name in AGENTS:
        return "agent exists: " + name
    AGENTS[name] = Agent(name, model, system, tool_names)
    return ""


def get_agent(name):
    return AGENTS[name]


def has_agent(name):
    return name in AGENTS


def delete_agent(name):
    if name in AGENTS:
        del AGENTS[name]
        return True
    return False


def list_json():
    parts = []
    for name in AGENTS:
        parts.append(AGENTS[name].describe())
    return "[" + ",".join(parts) + "]"
