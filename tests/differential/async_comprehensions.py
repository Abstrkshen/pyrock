import asyncio


async def numbers(n):
    for i in range(n):
        await asyncio.sleep(0)
        yield i


async def double(x):
    await asyncio.sleep(0)
    return x * 2


async def is_even(x):
    await asyncio.sleep(0)
    return x % 2 == 0


async def main():
    print([x async for x in numbers(5)])
    print({x % 3 async for x in numbers(7)})
    print({x: x * x async for x in numbers(4) if x})
    print([await double(x) for x in range(4)])
    print([x for x in range(8) if await is_even(x)])
    print([(x, y) async for x in numbers(3) for y in range(x)])
    print([await double(x) async for x in numbers(3) if x != 1])
    print(sum([await double(x) for x in [1, 2, 3]]))
    agen = (x * 10 async for x in numbers(4))
    print([v async for v in agen])
    doubled = (await double(x) for x in range(3))
    async for v in doubled:
        print("got", v)
    log = []

    async def noisy(x):
        log.append(x)
        await asyncio.sleep(0)
        return x

    lazy = (await noisy(x) for x in range(3))
    print(log)
    print(await lazy.__anext__(), log)
    total = 0
    async for v in lazy:
        total += v
    print(total, log)


asyncio.run(main())
