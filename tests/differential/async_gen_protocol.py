import asyncio


async def accumulator():
    total = 0
    try:
        while True:
            x = yield total
            await asyncio.sleep(0)
            if x is not None:
                total += x
    except KeyError as e:
        print("caught", e)
        yield -1
    finally:
        print("acc cleanup")


async def main():
    acc = accumulator()
    print(await acc.__anext__())
    print(await acc.asend(5), await acc.asend(7))
    print(await acc.athrow(KeyError("boom")))
    await acc.aclose()
    fresh = accumulator()
    try:
        await fresh.asend(1)
    except TypeError as e:
        print("TypeError", e)
    done = accumulator()
    await done.__anext__()
    await done.aclose()
    try:
        await done.asend(3)
    except StopAsyncIteration:
        print("stopped")


asyncio.run(main())
