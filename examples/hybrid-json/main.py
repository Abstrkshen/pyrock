# pyrock run main.py
import json


def title(raw: str) -> str:
    return json.loads(raw)["title"]


def year(raw: str) -> int:
    return json.loads(raw)["year"]


raw = "{\"title\": \"pyrock\", \"year\": 2026}"
print(title(raw))
print(year(raw) + 1)
