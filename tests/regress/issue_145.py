def side_effect() -> int:
    print(2)
    return 3


print(1, side_effect())
