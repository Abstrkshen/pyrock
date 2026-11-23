# mode: native-only
# Was tests/known_bugs/method_overrides_collapse.py: calls on a receiver of type Never
# (before any call reached the function) were bridged, so their arguments escaped to
# CPython and inference collapsed to object.
# A small Lisp interpreter: tokenizer, parser, environments, lambda with
# closures, define, set!, if and recursion. Each kind of Lisp value is a Python
# class with its own methods; pyrock compiles them to one Rust enum.


class LispError(Exception):
    pass


class Val:
    def show(self):
        return "?"

    def as_num(self):
        raise LispError("not a number: " + self.show())

    def as_list(self):
        raise LispError("not a list: " + self.show())

    def sym_name(self):
        return ""

    def truthy(self):
        return True

    def call(self, args):
        raise LispError("not a function: " + self.show())


class Num(Val):
    def __init__(self, n):
        self.n = n

    def show(self):
        return str(self.n)

    def as_num(self):
        return self.n

    def truthy(self):
        return self.n != 0


class Sym(Val):
    def __init__(self, name):
        self.name = name

    def show(self):
        return self.name

    def sym_name(self):
        return self.name


class Lst(Val):
    def __init__(self, items):
        self.items = items

    def show(self):
        return "(" + " ".join(v.show() for v in self.items) + ")"

    def as_list(self):
        return self.items

    def truthy(self):
        return len(self.items) > 0


class Builtin(Val):
    def __init__(self, name):
        self.name = name

    def show(self):
        return "<builtin " + self.name + ">"

    def call(self, args):
        return call_builtin(self.name, args)


class Fn(Val):
    def __init__(self, params, body, env):
        self.params = params
        self.body = body
        self.env = env

    def show(self):
        return "<lambda (" + " ".join(self.params) + ")>"

    def call(self, args):
        if len(args) != len(self.params):
            raise LispError(f"expected {len(self.params)} arguments, got {len(args)}")
        env = Env(self.env)
        for name, value in zip(self.params, args):
            env.vars[name] = value
        return evaluate(self.body, env)


class Env:
    def __init__(self, parent):
        self.vars = {}
        self.parent = parent

    def find(self, name):
        e = self
        while e is not None:
            if name in e.vars:
                return e
            e = e.parent
        raise LispError("unbound symbol: " + name)


def tokenize(src):
    return src.replace("(", " ( ").replace(")", " ) ").split()


def parse(tokens, pos):
    """Parse one expression starting at tokens[pos]; returns it and the next position."""
    tok = tokens[pos]
    if tok == "(":
        items = []
        pos += 1
        while tokens[pos] != ")":
            item, pos = parse(tokens, pos)
            items.append(item)
        return Lst(items), pos + 1
    if tok.lstrip("-").isdigit():
        return Num(int(tok)), pos + 1
    return Sym(tok), pos + 1


def call_builtin(name, args):
    if name == "+":
        return Num(sum(a.as_num() for a in args))
    if name == "*":
        prod = 1
        for a in args:
            prod *= a.as_num()
        return Num(prod)
    if name == "-":
        if len(args) == 1:
            return Num(-args[0].as_num())
        return Num(args[0].as_num() - args[1].as_num())
    if name == "<":
        return Num(1 if args[0].as_num() < args[1].as_num() else 0)
    if name == "=":
        return Num(1 if args[0].as_num() == args[1].as_num() else 0)
    if name == "mod":
        return Num(args[0].as_num() % args[1].as_num())
    if name == "list":
        return Lst(args)
    if name == "cons":
        return Lst([args[0]] + args[1].as_list())
    if name == "car":
        xs = args[0].as_list()
        if not xs:
            raise LispError("car of an empty list")
        return xs[0]
    if name == "cdr":
        xs = args[0].as_list()
        if not xs:
            raise LispError("cdr of an empty list")
        return Lst(xs[1:])
    if name == "null?":
        return Num(1 if len(args[0].as_list()) == 0 else 0)
    raise LispError("unknown builtin " + name)


def evaluate(x, env):
    name = x.sym_name()
    if name:
        return env.find(name).vars[name]
    if not isinstance(x, Lst) or len(x.items) == 0:
        return x
    items = x.items
    op = items[0].sym_name()
    if op == "quote":
        return items[1]
    if op == "if":
        branch = items[2] if evaluate(items[1], env).truthy() else items[3]
        return evaluate(branch, env)
    if op == "define":
        env.vars[items[1].sym_name()] = evaluate(items[2], env)
        return items[1]
    if op == "set!":
        target = items[1].sym_name()
        env.find(target).vars[target] = evaluate(items[2], env)
        return items[1]
    if op == "lambda":
        return Fn([p.sym_name() for p in items[1].as_list()], items[2], env)
    if op == "begin":
        result = x
        for e in items[1:]:
            result = evaluate(e, env)
        return result
    f = evaluate(items[0], env)
    return f.call([evaluate(a, env) for a in items[1:]])


PROGRAM = """
(define fact (lambda (n) (if (< n 2) 1 (* n (fact (- n 1))))))
(fact 20)
(define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2))))))
(fib 18)
(define map (lambda (f xs) (if (null? xs) (quote ()) (cons (f (car xs)) (map f (cdr xs))))))
(map (lambda (x) (* x x)) (list 1 2 3 4 5 6))
(define filter (lambda (p xs) (if (null? xs) (quote ())
  (if (p (car xs)) (cons (car xs) (filter p (cdr xs))) (filter p (cdr xs))))))
(define range (lambda (a b) (if (< a b) (cons a (range (+ a 1) b)) (quote ()))))
(filter (lambda (n) (= (mod n 3) 0)) (range 1 30))
(define make-counter (lambda () (begin (define n 0) (lambda () (begin (set! n (+ n 1)) n)))))
(define tick (make-counter))
(tick)
(tick)
(tick)
(define compose (lambda (f g) (lambda (x) (f (g x)))))
((compose (lambda (x) (* x 10)) (lambda (x) (+ x 1))) 4)
(car (quote ()))
(undefined-thing 1 2)
"""

g = Env(None)
for name in ["+", "-", "*", "<", "=", "mod", "list", "cons", "car", "cdr", "null?"]:
    g.vars[name] = Builtin(name)

tokens = tokenize(PROGRAM)
pos = 0
while pos < len(tokens):
    expr, pos = parse(tokens, pos)
    print(f"> {expr.show()}")
    try:
        print(f"  {evaluate(expr, g).show()}")
    except LispError as e:
        print(f"  error: {e}")
