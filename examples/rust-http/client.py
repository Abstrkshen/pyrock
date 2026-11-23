import time
import urllib.request


def get(port: int, path: str) -> str:
    url = "http://127.0.0.1:" + str(port) + path
    last = "no response"
    i = 0
    while i < 40:
        try:
            with urllib.request.urlopen(url, timeout=1) as resp:
                return resp.read().decode()
        except Exception as exc:
            last = str(exc)
            time.sleep(0.05)
            i = i + 1
    return last
