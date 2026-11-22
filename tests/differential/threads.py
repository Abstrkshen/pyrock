import queue
import threading
import time
from concurrent.futures import ThreadPoolExecutor


def work(n):
    s = 0
    for i in range(n):
        s += i % 7
    return s


results = queue.Queue()


def worker(k):
    time.sleep(0.01)
    results.put(k * k)


ts = [threading.Thread(target=worker, args=(k,)) for k in range(4)]
for t in ts:
    t.start()
print(work(1000000))
for t in ts:
    t.join()
print(sorted(results.get() for _ in range(4)))
with ThreadPoolExecutor(max_workers=3) as ex:
    print(list(ex.map(work, [10, 100, 1000])))

total = 0
lock = threading.Lock()


def add(k):
    global total
    for _ in range(1000):
        with lock:
            total += k


pool = [threading.Thread(target=add, args=(k,)) for k in range(1, 6)]
for t in pool:
    t.start()
for t in pool:
    t.join()
print("total", total)


def shout(name):
    print("hello from", name)


for n in ["a", "b"]:
    t = threading.Thread(target=shout, args=(n,))
    t.start()
    t.join()
