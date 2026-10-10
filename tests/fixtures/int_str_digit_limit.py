# int(str) refuses more than 4300 digits in a non-power-of-two base,
# exactly like CPython 3.13's default sys.get_int_max_str_digits().
ok = int("7" * 4300)
print(len(str(ok)), ok % 1000)
print(int("1_" * 2200 + "1") % 97)
print(int("f" * 4301, 16) % 1000003)
print(int("1" * 5000, 2) % 1000003)
for text in ["1" * 4301, "-" + "9" * 5000, " " + "3" * 4301 + "\n", "1" * 4301 + "x"]:
    try:
        int(text)
    except ValueError as e:
        print(e)
int("2" * 4301)
