import math
from sys import stdin, stdout

fin = stdin
fout = stdout

n = int(fin.readline().strip())
s = fin.readline().strip()
ans = []

gl = frozenset({'a', 'e', 'i', 'y', 'o', 'u'})

met = False
cdel = False
for i in range(n):

    if i > 0:
        if s[i] != s[i - 1]:
            met = False
            cdel = False
            ans.append(s[i])
        else:
            if s[i] in gl:
                if s[i] == 'e' or s[i] == 'o':
                    if not met:
                        ans.append(s[i])
                    elif not cdel:
                        ans.pop()
                        cdel = True
                met = True
            else:
                ans.append(s[i])
    else:
        ans.append(s[i])

fout.write(''.join(ans))
