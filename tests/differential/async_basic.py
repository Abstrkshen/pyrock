import asyncio


async def square(x):
    await asyncio.sleep(0.01)
    return x * x


async def worker(name, delay, log):
    for i in range(3):
        await asyncio.sleep(0)
        log.append(f"{name}{i}")
    return len(name)


async def fails():
    await asyncio.sleep(0)
    raise ValueError("boom")


async def main():
    print(await square(7))
    log = []
    a, b = await asyncio.gather(worker("a", 0.01, log), worker("bb", 0.015, log))
    print(a, b, log)
    total = 0
    for v in await asyncio.gather(*[square(i) for i in range(5)]):
        total += v
    print(total)
    try:
        await fails()
    except ValueError as e:
        print("caught", e)
    t = asyncio.create_task(square(3))
    print(await t)
    try:
        await asyncio.wait_for(asyncio.sleep(1), 0.01)
    except asyncio.TimeoutError:
        print("timeout")
    return "done"


print(asyncio.run(main()))
