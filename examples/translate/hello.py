def add(a, b):
    return a + b

def main():
    xs = [1, 2, 3]
    total = 0
    for x in xs:
        total = total + x
    print(add(2, 3))
    print(total)
    if total > 5:
        print("ok")
