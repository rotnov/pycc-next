import sys
input = sys.stdin.readline
p = 998244353
class matrix_data:
    def __init__(self, n, m):
        self.n = n
        self.m = m
        self.filled = {}
        self.alternating = [0, 0]
        self.rows = [[0]*(n + 1), [0]*(n + 1)]
        self.cols = [[0]*(m + 1), [0]*(m + 1)]
        self.frozen_rows = 0
        self.frozen_cols = 0
        self.bad_rows = 0
        self.bad_cols = 0

    def clear(self, x, y):
        if (x, y) not in self.filled:
            return
        t = self.filled.pop((x, y))
        self.alternating[(x + y + t) % 2] -= 1
        self.rows[(y + t) % 2][x] -= 1
        if self.rows[(y + t) % 2][x] == 0:
            if self.rows[(y + t + 1) % 2][x] == 0:
                self.frozen_rows -= 1
            else:
                self.bad_rows -= 1
        self.cols[(x + t) % 2][y] -= 1
        if self.cols[(x + t) % 2][y] == 0:
            if self.cols[(x + t + 1) % 2][y] == 0:
                self.frozen_cols -= 1
            else:
                self.bad_cols -= 1

    def write(self, x, y, t):
        self.filled[(x, y)] = t;self.alternating[(x + y + t) % 2] += 1;self.rows[(y + t) % 2][x] += 1
        if self.rows[(y + t) % 2][x] == 1:
            if self.rows[(y + t + 1) % 2][x] == 0:self.frozen_rows += 1
            else:self.bad_rows += 1
        self.cols[(x + t) % 2][y] += 1
        if self.cols[(x + t) % 2][y] == 1:
            if self.cols[(x + t + 1) % 2][y] == 0:self.frozen_cols += 1
            else:self.bad_cols += 1
    def count(self):
        total = 0
        if self.bad_rows == 0:total += pow(2, (n - self.frozen_rows), p)
        if self.bad_cols == 0:total += pow(2, (m - self.frozen_cols), p)
        if self.alternating[0] == 0:total -= 1
        if self.alternating[1] == 0:total -= 1
        print((total % p))

n, m, k = map(int, input().split());data = matrix_data(n, m)
for _ in range(k):
    x, y, t = map(int, input().split())
    if t == -1:data.clear(x, y)
    else:data.clear(x, y);data.write(x, y, t)
    data.count()
