def identity[T](x: T) -> T:
    return x


nums = [1, 2, 3, 4, 5]
counts = {"a": 1, "b": 2, "c": 3}
seen = {10, 20, 30}
point = (7, 8)

doubled = [n * 2 for n in nums]
squared_counts = {k: counts[k] * counts[k] for k in counts}

middle = nums[1:3]

last = nums.pop()
value = counts.get("b", 0)
missing = counts.get("z", 0)
seen.add(40)

for n in nums:
    print(n)
for n in doubled:
    print(n)
for n in middle:
    print(n)
print(len(seen))
print(squared_counts["b"])
print(point[0])
print(point[1])
print(last)
print(value)
print(missing)
print(identity(1))
print(identity("hi"))
