# str builtins and methods lowered to runtime calls (pycc_hir::builtins).
s = "  a bb  ccc "
xs = s.split()
print(xs, len(xs), len(s), len(""), len("héllo"))
print(" ".join(xs), "-".join("abc"), s.strip() + "|", s.lstrip() + "|", s.rstrip() + "|")
print("a,b,,c".split(","), "a,b,,c".split(",", 1), "x".strip("x"), "xxaxx".lstrip("x"), "xxaxx".rstrip("x"))
print("\t a \n".split(), "".split(), " ".split(" "), "a b".rsplit())
print(s[1], s[-1], s[2:5], s[::-1], "hello"[1:3], "hello"[::2], "hello"[10:], "hello"[-2:], "héllo"[1])
print("abcabc".count("b"), "abcabc".count(""), "abcabc".find("c"), "abcabc".rfind("c"), "abc".find("z"))
print("abcabc".index("c"), "abcabc".rindex("a"), "héllo".find("l"))
print("hello".replace("l", "L"), "ab".replace("", "-"), "Hi".upper(), "Hi".lower(), "Hi".swapcase())
print("abc".startswith("ab"), "abc".endswith("bc"), "abc".startswith("c"))
print("123".isdigit(), "12a".isdigit(), "".isdigit(), "ab".isalpha(), "a1".isalpha(), "a1".isalnum())
print("AB".isupper(), "Ab".isupper(), "ab".islower(), " \t".isspace(), "".isspace())
print("5".zfill(3), "-5".zfill(4), "+7".zfill(3), "abc".zfill(2))
print("ab".ljust(4) + "|", "ab".rjust(4), "ab".center(5) + "|", "ab".center(6) + "|", "abc".center(6) + "|")
print(list("abc"), sorted("cba"), list(reversed("abc")), "a\nb\r\nc\n".splitlines())
print(ord("A"), ord("é"), chr(66), chr(233), str(12) + str(1.5) + str(True) + str("s") + str())
print(min("b", "a"), max("b", "a", "c"), "b" < "a")
words = ["b", "a"]
words.extend(["c"])
print(words, sorted(words), min(words), max(words), words.count("a"), words.index("c"))
t = "x"
for i in range(3):
    t = t + str(i)
print(t, t[1:], t[:-1])
print("ab".index("z"))
