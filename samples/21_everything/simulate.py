def step(x, push):
    # damped random walk
    return 0.99 * x + push
