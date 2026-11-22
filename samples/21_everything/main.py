# Python, Rust, C and CPython packages in one program.
#   simulate.py -> compiled natively      kernels.rs -> Rust      noise.c -> C
#   json, statistics -> CPython at runtime
import json
import statistics

import simulate
import kernels
import noise


def run(steps: int, seed: int) -> float:
    x = 0.0
    state = seed
    for _ in range(steps):
        state = noise.lcg(state)
        x = simulate.step(x, kernels.smooth(noise.to_unit(state)))
    return x


results = []
for seed in range(1, 6):
    results.append(run(10000, seed))

summary = {
    "runs": len(results),
    "mean": round(statistics.mean(results), 6),
    "stdev": round(statistics.stdev(results), 6),
    "max": round(max(results), 6),
}
print(json.dumps(summary, sort_keys=True))
