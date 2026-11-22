import asyncio


async def ticker(n, delay):
    for i in range(n):
        await asyncio.sleep(delay)
        yield i * 10


async def fetch_pages(names):
    try:
        for name in names:
            await asyncio.sleep(0)
            if name == "bad":
                raise ValueError("bad page")
            yield name.upper()
    finally:
        print("fetch cleanup")


async def square(x):
    await asyncio.sleep(0)
    return x * x


async def squares(n):
    for i in range(n):
        yield await square(i)


async def collect(agen):
    out = []
    async for v in agen:
        out.append(v)
    return out


async def main():
    total = 0
    async for t in ticker(4, 0.001):
        total += t
        print("tick", t)
    print("total", total)
    print(await collect(squares(5)))
    try:
        async for p in fetch_pages(["a", "b", "bad", "c"]):
            print("page", p)
    except ValueError as e:
        print("caught", e)
    pages = fetch_pages(["x", "y", "z"])
    async for p in pages:
        print("first", p)
        break
    await pages.aclose()
    print("closed")
    both = await asyncio.gather(collect(ticker(3, 0)), collect(squares(3)))
    print(both)
    # Driven by CPython's own anext().
    g = ticker(3, 0)
    print(await anext(g), await anext(g), await anext(g), await anext(g, "end"))


asyncio.run(main())
