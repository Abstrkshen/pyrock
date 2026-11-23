# Control flow: for/range, if/elif/else, modulo, string building.
def fizzbuzz(n):
    if n % 15 == 0:
        return "FizzBuzz"
    elif n % 3 == 0:
        return "Fizz"
    elif n % 5 == 0:
        return "Buzz"
    return str(n)


line = ""
for i in range(1, 31):
    line = line + fizzbuzz(i) + " "
print(line)
