# Non-strict mode: storing a value of another type into an item of a
# nested list (`rows[i][j] = int(rows[i][j])`) makes the outer list
# CPython's own, so the store is seen through every alias of the row.
inputdata = []
for i in range(2):
    inputdata.append("1 2 3".split(" "))
    for j in range(len(inputdata[i])):
        inputdata[i][j] = int(inputdata[i][j])
print(inputdata, sum(inputdata[0]))

h, w = 2, 3
s = [["."] * w for i in range(h)]
row = s[1]
for i in range(h):
    t = "abc"
    for j in range(w):
        s[i][j] = t[j] if j else 5
print(s, row, row is s[1])
