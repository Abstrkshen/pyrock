# mode: native-only
import json

a = [1, 2]
b = a
b.append(3)
print(json.dumps(a))
