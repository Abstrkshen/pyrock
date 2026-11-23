# main.py: AI-agent server. HTTP in Rust (net.rs); agent logic in Python compiled natively.
#   pyrock run main.py -- --demo
#   pyrock run main.py -- --serve 8090 [--model qwen3.5:4b]
import net
import agents

DEFAULT_MODEL = "qwen3.5:4b"


def arg_value(argv, flag, fallback):
    for i in range(len(argv) - 1):
        if argv[i] == flag:
            return argv[i + 1]
    return fallback


def make_defaults(model):
    agents.create_agent("math-tutor", model,
        "You are a patient math tutor. For ANY arithmetic you must call the calculate tool, then explain the result in one short sentence.",
        ["calculate"])
    agents.create_agent("note-taker", model,
        "You keep notes for the user. Use remember to store facts and recall to look them up. Be brief.",
        ["remember", "recall"])
    agents.create_agent("text-analyst", model,
        "You analyse text. Use the word_stats tool on the text the user gives you and report the numbers.",
        ["word_stats"])


def demo(model):
    make_defaults(model)
    prompts = [
        ("math-tutor", "What is (12.5 + 3) * 4 - 2 ** 5 / 8 ?"),
        ("note-taker", "Remember that my locker code is 4821."),
        ("note-taker", "What is my locker code?"),
        ("text-analyst", "Analyse this text: the cat saw the dog and the dog saw the cat run"),
    ]
    for p in prompts:
        a = agents.get_agent(p[0])
        t0 = net.now_ms()
        print("=== " + a.name + " [" + a.model + "] ===")
        print("user> " + p[1])
        reply = a.chat(p[1])
        for s in a.steps:
            print("  tool " + s.tool + " " + s.args + " -> " + s.result)
        print("agent> " + reply)
        print("  (" + str(net.now_ms() - t0) + " ms)")


def err(msg):
    return '{"error":' + net.json_quote(msg) + "}"


def handle(conn, method, path, body, model):
    if method == "GET" and path == "/health":
        net.respond(conn, 200, '{"status":"ok","agents":' + str(len(agents.AGENTS)) + "}")
    elif method == "GET" and path == "/agents":
        net.respond(conn, 200, agents.list_json())
    elif method == "POST" and path == "/agents":
        name = net.json_str(body, "/name")
        if name == "":
            net.respond(conn, 400, err("missing name"))
            return
        system = net.json_str(body, "/system")
        if system == "":
            system = "You are a helpful assistant."
        m = net.json_str(body, "/model")
        if m == "":
            m = model
        names = []
        for i in range(net.json_len(body, "/tools")):
            names.append(net.json_str(body, "/tools/" + str(i)))
        problem = agents.create_agent(name, m, system, names)
        if problem != "":
            net.respond(conn, 409, err(problem))
        else:
            net.respond(conn, 201, agents.get_agent(name).describe())
    elif method == "POST" and path.startswith("/agents/") and path.endswith("/chat"):
        name = path[8:len(path) - 5]
        if not agents.has_agent(name):
            net.respond(conn, 404, err("no such agent: " + name))
            return
        message = net.json_str(body, "/message")
        if message == "":
            net.respond(conn, 400, err("missing message"))
            return
        a = agents.get_agent(name)
        reply = a.chat(message)
        net.respond(conn, 200, '{"reply":' + net.json_quote(reply) + ',"steps":' + a.steps_json() + "}")
    elif method == "DELETE" and path.startswith("/agents/"):
        name = path[8:]
        if agents.delete_agent(name):
            net.respond(conn, 200, '{"deleted":' + net.json_quote(name) + "}")
        else:
            net.respond(conn, 404, err("no such agent: " + name))
    else:
        net.respond(conn, 404, err("no route " + method + " " + path))


def serve(port, model):
    make_defaults(model)
    bound = net.listen(port)
    print("listening on http://127.0.0.1:" + str(bound) + " model=" + model)
    while True:
        conn, method, path, body = net.accept()
        try:
            handle(conn, method, path, body, model)
        except RuntimeError as e:
            net.respond(conn, 500, err(str(e)))


def main():
    argv = net.args()
    model = arg_value(argv, "--model", DEFAULT_MODEL)
    serve_port = arg_value(argv, "--serve", "")
    if serve_port != "":
        serve(int(serve_port), model)
    else:
        demo(model)


main()
