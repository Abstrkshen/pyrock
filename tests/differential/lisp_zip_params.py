# mode: native-only
# Was tests/known_bugs/zip_collapses_inference.py: `zip(f.params, args)` in a function
# first analyzed before any call reached it (arguments of type Never) was bridged, so
# `args` escaped to CPython and inference gave up on the whole program.
# A small Lisp interpreter: tokenizer, parser, environments, lambda with
# closures, define, set!, if, and recursion. Lisp values are Python classes,
# which pyrock compiles to one Rust enum.


class Val:
    def show(self):
        return "?"


class Num(Val):
    def __init__(self, n):
        self.n = n

    def show(self):
        return str(self.n)


class Sym(Val):
    def __init__(self, name):
        self.name = name

    def show(self):
        return self.name


class Lst(Val):
    def __init__(self, items):
        self.items = items

    def show(self):
        return "(" + " ".join(v.show() for v in self.items) + ")"


class Builtin(Val):
    def __init__(self, name):
        self.name = name

    def show(self):
        return "<builtin " + self.name + ">"


class Fn(Val):
    def __init__(self, params, body, env):
        self.params = params
        self.body = body
        self.env = env

    def show(self):
        return "<lambda (" + " ".join(self.params) + ")>"


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
        raise NameError("unbound symbol: " + name)


class LispError(Exception):
    pass


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


def num(v):
    if isinstance(v, Num):
        return v.n
    raise LispError("not a number: " + v.show())


def truthy(v):
    if isinstance(v, Num):
        return v.n != 0
    if isinstance(v, Lst):
        return len(v.items) > 0
    return True


def call_builtin(name, args):
    if name == "+":
        total = 0
        for a in args:
            total += num(a)
        return Num(total)
    if name == "*":
        prod = 1
        for a in args:
            prod *= num(a)
        return Num(prod)
    if name == "-":
        if len(args) == 1:
            return Num(-num(args[0]))
        return Num(num(args[0]) - num(args[1]))
    if name == "<":
        return Num(1 if num(args[0]) < num(args[1]) else 0)
    if name == "=":
        return Num(1 if num(args[0]) == num(args[1]) else 0)
    if name == "mod":
        return Num(num(args[0]) % num(args[1]))
    if name == "list":
        return Lst(args)
    if name == "cons":
        rest = args[1]
        if isinstance(rest, Lst):
            return Lst([args[0]] + rest.items)
        raise LispError("cons onto a non-list")
    if name == "car" or name == "cdr":
        xs = args[0]
        if isinstance(xs, Lst) and len(xs.items) > 0:
            return xs.items[0] if name == "car" else Lst(xs.items[1:])
        raise LispError(name + " of an empty list")
    if name == "null?":
        xs = args[0]
        return Num(1 if isinstance(xs, Lst) and len(xs.items) == 0 else 0)
    raise LispError("unknown builtin " + name)


def apply(f, args):
    if isinstance(f, Builtin):
        return call_builtin(f.name, args)
    if isinstance(f, Fn):
        env = Env(f.env)
        for name, value in zip(f.params, args):
            env.vars[name] = value
        return evaluate(f.body, env)
    raise LispError("not a function: " + f.show())


def evaluate(x, env):
    if isinstance(x, Num):
        return x
    if isinstance(x, Sym):
        return env.find(x.name).vars[x.name]
    if isinstance(x, Lst):
        if len(x.items) == 0:
            return x
        head = x.items[0]
        if isinstance(head, Sym):
            op = head.name
            if op == "quote":
                return x.items[1]
            if op == "if":
                branch = x.items[2] if truthy(evaluate(x.items[1], env)) else x.items[3]
                return evaluate(branch, env)
            if op == "define":
                target = x.items[1]
                if isinstance(target, Sym):
                    env.vars[target.name] = evaluate(x.items[2], env)
                    return target
            if op == "set!":
                target = x.items[1]
                if isinstance(target, Sym):
                    env.find(target.name).vars[target.name] = evaluate(x.items[2], env)
                    return target
            if op == "lambda":
                params = []
                ps = x.items[1]
                if isinstance(ps, Lst):
                    for p in ps.items:
                        params.append(p.show())
                return Fn(params, x.items[2], env)
            if op == "begin":
                result = x
                for e in x.items[1:]:
                    result = evaluate(e, env)
                return result
        f = evaluate(head, env)
        return apply(f, [evaluate(a, env) for a in x.items[1:]])
    return x


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
"""

g = Env(None)
for name in ["+", "-", "*", "<", "=", "mod", "list", "cons", "car", "cdr", "null?"]:
    g.vars[name] = Builtin(name)

tokens = tokenize(PROGRAM)
pos = 0
while pos < len(tokens):
    start = pos
    expr, pos = parse(tokens, pos)
    try:
        result = evaluate(expr, g)
        print(f"> {expr.show()}")
        print(f"  {result.show()}")
    except LispError as e:
        print(f"> {expr.show()}")
        print(f"  error: {e}")
