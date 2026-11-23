# `**mapping` forwarded into a native function's own **kwargs.
def handle(event, *args, **kwargs):
    keys = sorted(kwargs)
    return event + str(len(args)) + ":" + ",".join(k + "=" + str(kwargs[k]) for k in keys)


def on_start(*args, **kwargs):
    return handle("start", *args, **kwargs)


def tagged(name, **kwargs):
    return handle(name, tag="t", **kwargs)


print(on_start(1, 2, run_id=7, parent=0), on_start(), tagged("x", level=3), tagged("y"))
