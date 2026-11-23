"""A small JSON API on asyncio streams.

`pyrock run main.py` starts the server on a free port, sends it a scripted
set of requests (some concurrently), prints the responses and exits.
`pyrock run main.py -- --serve 8080` keeps serving on port 8080.
"""
import asyncio
import json
import sys

from store import NotFound, Store

REASONS = {200: "OK", 201: "Created", 400: "Bad Request", 404: "Not Found", 405: "Method Not Allowed"}


def parse_query(qs: str) -> dict:
    out = {}
    for part in qs.split("&"):
        if "=" in part:
            k, v = part.split("=", 1)
            out[k] = v
    return out


def route(store, method: str, path: str, body: str):
    """(status, payload) for one request. Everything here is native code."""
    target, _, qs = path.partition("?")
    query = parse_query(qs)
    parts = [p for p in target.split("/") if p]
    try:
        if parts == ["health"]:
            return 200, {"status": "ok"}
        if parts == ["todos"] and method == "GET":
            done = None
            if "done" in query:
                done = query["done"] == "true"
            found = store.search(query.get("tag"), done)
            return 200, {"todos": [t.to_dict() for t in found]}
        if parts == ["todos"] and method == "POST":
            data = json.loads(body)
            todo = store.add(str(data.get("title", "")), [str(t) for t in data.get("tags", [])])
            return 201, todo.to_dict()
        if len(parts) == 2 and parts[0] == "todos" and parts[1].isdigit():
            if method == "GET":
                return 200, store.get(int(parts[1])).to_dict()
            if method == "PATCH":
                return 200, store.toggle(int(parts[1])).to_dict()
            return 405, {"error": f"{method} not allowed"}
        if parts == ["stats"]:
            return 200, store.stats()
        return 404, {"error": f"no route for {target}"}
    except NotFound as e:
        return 404, {"error": f"todo {e} not found"}
    except ValueError as e:
        return 400, {"error": str(e)}


async def handle(reader, writer, store):
    request_line = (await reader.readline()).decode()
    method, path, _ = request_line.split(" ", 2)
    length = 0
    while True:
        line = (await reader.readline()).decode().strip()
        if not line:
            break
        name, _, value = line.partition(":")
        if name.lower() == "content-length":
            length = int(value.strip())
    body = (await reader.readexactly(length)).decode() if length else ""
    status, payload = route(store, method, path, body)
    data = json.dumps(payload).encode()
    head = f"HTTP/1.1 {status} {REASONS[status]}\r\nContent-Type: application/json\r\nContent-Length: {len(data)}\r\nConnection: close\r\n\r\n"
    writer.write(head.encode() + data)
    await writer.drain()
    writer.close()
    await writer.wait_closed()


async def request(port, method, path, payload=None):
    reader, writer = await asyncio.open_connection("127.0.0.1", port)
    body = json.dumps(payload) if payload is not None else ""
    writer.write(f"{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {len(body.encode())}\r\n\r\n{body}".encode())
    await writer.drain()
    raw = (await reader.read()).decode()
    writer.close()
    await writer.wait_closed()
    head, _, text = raw.partition("\r\n\r\n")
    status = int(head.split(" ")[1])
    return status, json.loads(text)


async def selftest():
    store = Store()
    server = await asyncio.start_server(lambda r, w: handle(r, w, store), "127.0.0.1", 0)
    port = server.sockets[0].getsockname()[1]
    print(await request(port, "GET", "/health"))
    created = await asyncio.gather(
        request(port, "POST", "/todos", {"title": "write docs", "tags": ["Docs", "work"]}),
        request(port, "POST", "/todos", {"title": "fix bug", "tags": ["work"]}),
        request(port, "POST", "/todos", {"title": "buy milk", "tags": ["home"]}),
    )
    print(sorted(c[1]["title"] for c in created))
    print(await request(port, "POST", "/todos", {"title": "   "}))
    first = min(c[1]["id"] for c in created)
    print(await request(port, "PATCH", f"/todos/{first}"))
    print(await request(port, "GET", "/todos?tag=work&done=false"))
    print(await request(port, "GET", "/todos/99"))
    print(await request(port, "DELETE", f"/todos/{first}"))
    print(await request(port, "GET", "/stats"))
    print(await request(port, "GET", "/nope"))
    server.close()
    await server.wait_closed()


async def serve(port):
    store = Store()
    server = await asyncio.start_server(lambda r, w: handle(r, w, store), "127.0.0.1", port)
    print(f"listening on http://127.0.0.1:{port}")
    async with server:
        await server.serve_forever()


if "--serve" in sys.argv:
    asyncio.run(serve(int(sys.argv[-1])))
else:
    asyncio.run(selftest())
