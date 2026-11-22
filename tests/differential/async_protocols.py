import asyncio


class Account:
    def __init__(self, name):
        self.name = name
        self.balance = 0

    async def deposit(self, amount):
        await asyncio.sleep(0)
        self.balance += amount
        return self.balance


class Ticker:
    def __init__(self, n):
        self.n = n
        self.i = 0

    def __aiter__(self):
        return self

    async def __anext__(self):
        if self.i >= self.n:
            raise StopAsyncIteration
        self.i += 1
        await asyncio.sleep(0)
        return self.i


class session:
    def __init__(self, name):
        self.name = name

    async def __aenter__(self):
        await asyncio.sleep(0)
        print("open", self.name)
        return self.name.upper()

    async def __aexit__(self, kind, value, tb):
        print("close", self.name)
        return False


async def producer(q, n):
    for i in range(n):
        await q.put(i)
    await q.put(None)


async def consumer(q):
    total = 0
    while True:
        item = await q.get()
        if item is None:
            break
        total += item
    return total


def make_adder(k):
    async def add(x):
        await asyncio.sleep(0)
        return x + k
    return add


async def busy(n):
    s = 0
    for i in range(n):
        s += i * i % 7
    await asyncio.sleep(0)
    return s


async def main():
    acct = Account("alice")
    await acct.deposit(5)
    print(acct.name, await acct.deposit(7))
    async for t in Ticker(3):
        print("tick", t)
    async with session("db") as s:
        print("using", s)
    try:
        async with session("bad"):
            raise KeyError("k")
    except KeyError as e:
        print("keyerror", e)
    q = asyncio.Queue()
    res = await asyncio.gather(producer(q, 100), consumer(q))
    print(res[1])
    add5 = make_adder(5)
    print(await add5(10))
    many = await asyncio.gather(*[busy(1000 + i) for i in range(2000)])
    print(len(many), sum(many))
    r = await asyncio.gather(asyncio.sleep(0, "x"), busy(10), return_exceptions=True)
    print(r)
    c = busy(3)
    print(await c)
    try:
        await c
    except RuntimeError as e:
        print("reuse:", e)
    await asyncio.sleep(0)
    raise ValueError("fatal")


asyncio.run(main())
