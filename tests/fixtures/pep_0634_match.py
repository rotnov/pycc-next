def classify(n: int) -> str:
    match n:
        case 0:
            return "zero"
        case 1:
            return "one"
        case _:
            return "many"

print(classify(0))
print(classify(1))
print(classify(42))

x = True
match x:
    case True:
        print("yes")
    case False:
        print("no")

y = 5
match y:
    case z if z > 3:
        print(z)
    case _:
        print("small")

w = 2
match w:
    case 1 | 2 | 3:
        print("low")
    case _:
        print("high")
