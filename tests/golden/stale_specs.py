# A dict that gains a key in each of 20 functions: every round of inference
# widens its type and leaves the old specializations of `show` behind. Those
# stale ones must not count towards the limit of 16, or `show` would fall
# back to the shared object-typed specialization (x: object, bridged).
def show(st, x):
    return x * 0.5 + st["base"]

def step0(st):
    st["k0"] = 0
    return show(st, st["base"] + 0)

def step1(st):
    st["k1"] = 1
    return show(st, st["base"] + 1)

def step2(st):
    st["k2"] = 2
    return show(st, st["base"] + 2)

def step3(st):
    st["k3"] = 3
    return show(st, st["base"] + 3)

def step4(st):
    st["k4"] = 4
    return show(st, st["base"] + 4)

def step5(st):
    st["k5"] = 5
    return show(st, st["base"] + 5)

def step6(st):
    st["k6"] = 6
    return show(st, st["base"] + 6)

def step7(st):
    st["k7"] = 7
    return show(st, st["base"] + 7)

def step8(st):
    st["k8"] = 8
    return show(st, st["base"] + 8)

def step9(st):
    st["k9"] = 9
    return show(st, st["base"] + 9)

def step10(st):
    st["k10"] = 10
    return show(st, st["base"] + 10)

def step11(st):
    st["k11"] = 11
    return show(st, st["base"] + 11)

def step12(st):
    st["k12"] = 12
    return show(st, st["base"] + 12)

def step13(st):
    st["k13"] = 13
    return show(st, st["base"] + 13)

def step14(st):
    st["k14"] = 14
    return show(st, st["base"] + 14)

def step15(st):
    st["k15"] = 15
    return show(st, st["base"] + 15)

def step16(st):
    st["k16"] = 16
    return show(st, st["base"] + 16)

def step17(st):
    st["k17"] = 17
    return show(st, st["base"] + 17)

def step18(st):
    st["k18"] = 18
    return show(st, st["base"] + 18)

def step19(st):
    st["k19"] = 19
    return show(st, st["base"] + 19)

def run():
    st = {"base": 1}
    print(step0(st))
    print(step1(st))
    print(step2(st))
    print(step3(st))
    print(step4(st))
    print(step5(st))
    print(step6(st))
    print(step7(st))
    print(step8(st))
    print(step9(st))
    print(step10(st))
    print(step11(st))
    print(step12(st))
    print(step13(st))
    print(step14(st))
    print(step15(st))
    print(step16(st))
    print(step17(st))
    print(step18(st))
    print(step19(st))
    print(len(st))

run()
