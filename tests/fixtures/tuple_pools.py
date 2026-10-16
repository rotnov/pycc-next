# Tuples and strings recycled through the runtime's free lists (crates/pycc_rt/src/pool.rs):
# tuples of every pooled length and beyond, nested tuples freed by their parents, many
# objects alive at once (more than a pool holds) and then dropped, and values that must
# not be disturbed when a freed object's memory is reused.


def keys(n: int) -> list[tuple[str, ...]]:
    out: list[tuple[str, ...]] = []
    for i in range(n):
        key: tuple[str, ...] = ()
        for j in range(i % 12):
            key = key + (str(i * 31 + j),)
        out.append(key)
    return out


def describe(key: tuple[str, ...]) -> str:
    if not key:
        return "()"
    return key[0] + ".." + key[-1] + "/" + str(len(key))


def shuffle(xs: list[tuple[str, ...]]) -> list[tuple[str, ...]]:
    # Free every other tuple, then build new ones of other lengths in their place.
    kept: list[tuple[str, ...]] = []
    for i in range(len(xs)):
        if i % 2 == 0:
            kept.append(xs[i])
    for i in range(len(kept)):
        kept[i] = kept[i][1:] + ("tail" + str(i),)
    return kept


def nested(depth: int) -> tuple[str, ...]:
    t: tuple[str, ...] = ("root",)
    for d in range(depth):
        t = t[-3:] + (str(d) * (d % 30),)
    return t


def long_strings(n: int) -> list[str]:
    out: list[str] = []
    for i in range(n):
        # Inline (<= 22 bytes) and heap payloads alternate.
        out.append("x" * (i % 40) + str(i))
    return out


def main() -> None:
    many = keys(600)
    print(len(many), describe(many[0]), describe(many[11]), describe(many[599]))
    total = 0
    for k in many:
        total += len(k)
    print("elements", total)
    survivors = shuffle(many)
    many = []
    print(len(survivors), describe(survivors[0]), describe(survivors[5]), describe(survivors[299]))
    seen: dict[tuple[str, ...], int] = {}
    for i in range(len(survivors)):
        seen[survivors[i][:2]] = i
    print("distinct prefixes", len(seen))
    survivors = []
    print(describe(nested(500)), describe(nested(7)))
    strs = long_strings(700)
    print(len(strs), strs[0], strs[39], strs[699][-5:])
    joined = ",".join(strs[:5])
    strs = []
    again = long_strings(3)
    print(joined, again)
    abc: tuple[str, ...] = ("a", "b", "c")
    stem = abc * 3
    print(stem[2:7], stem[::4], stem[-1:-4:-1], stem[5:2])
    try:
        print(stem[::0])
    except ValueError as e:
        print("ValueError:", e)
    print(describe(stem + stem[:1]))
    try:
        print(keys(13)[11][20])
    except IndexError as e:
        print("IndexError:", e, describe(keys(13)[11]))


main()
