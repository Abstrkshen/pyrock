# Float numerics: Newton's method, trapezoid integration, Leibniz series.
def newton_sqrt(x, iterations):
    guess = x / 2.0
    for _ in range(iterations):
        guess = (guess + x / guess) / 2.0
    return guess


def f(x):
    return x * x * x - 2.0 * x + 1.0


def integrate(a, b, steps):
    h = (b - a) / steps
    total = (f(a) + f(b)) / 2.0
    for i in range(1, steps):
        total += f(a + i * h)
    return total * h


def leibniz_pi(terms):
    acc = 0.0
    sign = 1.0
    for k in range(terms):
        acc += sign / (2 * k + 1)
        sign = -sign
    return 4.0 * acc


print("sqrt(2) ~", newton_sqrt(2.0, 6))
print("integral of x^3-2x+1 on [0,2] ~", integrate(0.0, 2.0, 100000))
print("pi ~", leibniz_pi(1000000))
print("float edge cases:", 1e16, 1.5e-5, -0.0, 0.1 + 0.2, 7.5 // 2, -7.5 % 2)
