lst = [set(map(int, input().split())) for _ in range(6)]
rec = []
while lst:
  x = lst[0]
  count = lst.count(x)
  if count % 2 == 1:
    print("no")
    break
  rec.append((count, x))
  for _ in range(count):
    lst.pop(lst.index(x))
else:
  if len(rec) == 1:
    if len(rec[0][1]) == 1:
      print("yes")
    else:
      print("no")
  elif len(rec) == 2:
    rec.sort()
    if rec[0][1] & rec[1][1] == rec[0][1]:
      print("yes")
    else:
      print("no")
  elif len(rec) == 3:
    if len(rec[0][1]) == len(rec[1][1]) == len(rec[2][1]) == 2 and (rec[2][1] & (rec[0][1] | rec[1][1]) == rec[2][1]):
      print("yes")
    else:
      print("no")
