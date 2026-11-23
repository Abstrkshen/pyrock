# tools.py: the functions a model may call. Every tool is (args_json: str) -> str.
import net

NOTES = {}


# ---- calculate: recursive-descent arithmetic ------------------------------
class Parser:
    def __init__(self, text):
        self.text = text
        self.pos = 0

    def skip(self):
        while self.pos < len(self.text) and self.text[self.pos] == " ":
            self.pos += 1

    def peek(self):
        self.skip()
        if self.pos < len(self.text):
            return self.text[self.pos]
        return ""

    def expr(self):
        value = self.term()
        while True:
            c = self.peek()
            if c == "+":
                self.pos += 1
                value = value + self.term()
            elif c == "-":
                self.pos += 1
                value = value - self.term()
            else:
                return value

    def term(self):
        value = self.power()
        while True:
            c = self.peek()
            if c == "*" and self.text[self.pos:self.pos + 2] != "**":
                self.pos += 1
                value = value * self.power()
            elif c == "/":
                self.pos += 1
                divisor = self.power()
                if divisor == 0.0:
                    raise ValueError("division by zero")
                value = value / divisor
            else:
                return value

    def power(self):
        base = self.unary()
        if self.peek() == "*" and self.text[self.pos:self.pos + 2] == "**":
            self.pos += 2
            exponent = self.power()  # right associative
            return pow_float(base, exponent)
        return base

    def unary(self):
        c = self.peek()
        if c == "-":
            self.pos += 1
            return 0.0 - self.unary()
        if c == "+":
            self.pos += 1
            return self.unary()
        return self.atom()

    def atom(self):
        c = self.peek()
        if c == "(":
            self.pos += 1
            value = self.expr()
            if self.peek() != ")":
                raise ValueError("missing )")
            self.pos += 1
            return value
        start = self.pos
        while self.pos < len(self.text) and (self.text[self.pos].isdigit() or self.text[self.pos] == "."):
            self.pos += 1
        if start == self.pos:
            raise ValueError("unexpected '" + c + "' at position " + str(self.pos))
        return float(self.text[start:self.pos])


def pow_float(base, exponent):
    if exponent == float(int(exponent)) and exponent >= 0.0 and exponent < 1000.0:
        result = 1.0
        n = int(exponent)
        for _ in range(n):
            result = result * base
        return result
    return base ** exponent


def format_number(x):
    if x == float(int(x)) and x < 1e15 and x > -1e15:
        return str(int(x))
    return str(x)


def calculate(args_json: str) -> str:
    expression = net.json_str(args_json, "/expression")
    if expression == "":
        return "error: missing 'expression'"
    try:
        p = Parser(expression)
        value = p.expr()
        if p.peek() != "":
            return "error: unexpected '" + p.peek() + "' at position " + str(p.pos)
        return format_number(value)
    except ValueError as e:
        return "error: " + str(e)


# ---- word_stats -----------------------------------------------------------
def word_stats(args_json: str) -> str:
    text = net.json_str(args_json, "/text")
    if text == "":
        return "error: missing 'text'"
    counts = {}
    total = 0
    for raw in text.lower().split():
        word = raw.strip(".,;:!?\"'()")
        if word == "":
            continue
        total += 1
        counts[word] = counts.get(word, 0) + 1
    best = ""
    best_n = 0
    for word, n in counts.items():
        if n > best_n:
            best = word
            best_n = n
    return ("words=" + str(total) + " characters=" + str(len(text)) +
            " unique=" + str(len(counts)) + " most_common=" + best + " (" + str(best_n) + "x)")


# ---- remember / recall ----------------------------------------------------
def remember(args_json: str) -> str:
    key = net.json_str(args_json, "/key")
    value = net.json_str(args_json, "/value")
    if key == "" or value == "":
        return "error: need 'key' and 'value'"
    NOTES[key.lower()] = value
    return "stored " + key + " (" + str(len(NOTES)) + " notes)"


def recall(args_json: str) -> str:
    key = net.json_str(args_json, "/key")
    if key == "":
        return "error: missing 'key'"
    if key.lower() in NOTES:
        return NOTES[key.lower()]
    names = ", ".join(list(NOTES.keys()))
    return "no note named " + key + ". known: " + names


# ---- registry + JSON-schema descriptions ----------------------------------
TOOLS = {
    "calculate": calculate,
    "word_stats": word_stats,
    "remember": remember,
    "recall": recall,
}

SCHEMAS = {
    "calculate": '{"type":"function","function":{"name":"calculate","description":"Evaluate an arithmetic expression with + - * / ** and parentheses. Always use this for maths.","parameters":{"type":"object","properties":{"expression":{"type":"string","description":"e.g. (12.5+3)*4"}},"required":["expression"]}}}',
    "word_stats": '{"type":"function","function":{"name":"word_stats","description":"Count words and characters of a text and find its most common word.","parameters":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}}',
    "remember": '{"type":"function","function":{"name":"remember","description":"Store a note under a key.","parameters":{"type":"object","properties":{"key":{"type":"string"},"value":{"type":"string"}},"required":["key","value"]}}}',
    "recall": '{"type":"function","function":{"name":"recall","description":"Look up a stored note by key.","parameters":{"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}}}',
}


def has_tool(name: str) -> bool:
    return name in TOOLS


def run_tool(name: str, args_json: str) -> str:
    if name not in TOOLS:
        return "error: unknown tool " + name
    fn = TOOLS[name]
    return fn(args_json)


def schema_for(name: str) -> str:
    return SCHEMAS[name]
