# backend: a JSON API on asyncio

A todo API (`GET/POST /todos`, `GET/PATCH /todos/<id>`, `/stats`, `/health`) served
with `asyncio.start_server`. The routing, validation and the store (`store.py`, plain
classes and a dataclass) are compiled natively; sockets and `json` are the stdlib
through CPython.

Running it starts the server on a free port, sends it scripted requests (some
concurrently with `asyncio.gather`), prints the responses and exits.

```sh
pyrock check main.py
pyrock run main.py
```

To keep it serving: `pyrock run main.py -- --serve 8080`, then
`curl -X POST localhost:8080/todos -d '{"title": "x", "tags": ["a"]}'`.

What to look at in `pyrock explain main.py`: `route`, `parse_query` and the `Store`
methods are native; the handlers that touch sockets are bridged.
