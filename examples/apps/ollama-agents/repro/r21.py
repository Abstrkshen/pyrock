import net

def run(name: str, args: str) -> str:
    return name + args

def go(doc):
    for i in range(2):
        name = net.json_get(doc, "/n/" + str(i))
        args = net.json_get(doc, "/a")
        if name is None:
            continue
        if args is None:
            args = "{}"
        print(run(name, args))

hist = []
m = net.json_get("{}", "/message")
if m is None:
    print("none")
else:
    hist.append(m)
print(",".join(hist))
go('{"n":["x","y"],"a":"z"}')
