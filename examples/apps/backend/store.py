"""In-memory todo store: plain Python classes, compiled natively."""
from dataclasses import dataclass, field


class NotFound(KeyError):
    pass


@dataclass
class Todo:
    id: int
    title: str
    done: bool = False
    tags: list = field(default_factory=list)

    def to_dict(self):
        return {"id": self.id, "title": self.title, "done": self.done, "tags": list(self.tags)}


class Store:
    def __init__(self):
        self.items = {}
        self.next_id = 1

    def add(self, title, tags):
        if not title.strip():
            raise ValueError("title must not be empty")
        todo = Todo(self.next_id, title.strip(), False, [t.lower() for t in tags])
        self.items[todo.id] = todo
        self.next_id += 1
        return todo

    def get(self, todo_id):
        if todo_id not in self.items:
            raise NotFound(todo_id)
        return self.items[todo_id]

    def toggle(self, todo_id):
        todo = self.get(todo_id)
        todo.done = not todo.done
        return todo

    def search(self, tag=None, done=None):
        out = []
        for todo in self.items.values():
            if tag is not None and tag not in todo.tags:
                continue
            if done is not None and todo.done != done:
                continue
            out.append(todo)
        return out

    def stats(self):
        tags = {}
        done = 0
        for todo in self.items.values():
            if todo.done:
                done += 1
            for t in todo.tags:
                tags[t] = tags.get(t, 0) + 1
        top = sorted(tags.items(), key=lambda kv: (-kv[1], kv[0]))[:3]
        return {"total": len(self.items), "done": done, "open": len(self.items) - done, "top_tags": [t for t, _ in top]}
