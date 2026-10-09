''' E. Staircases
https://codeforces.com/contest/1598/problem/E
'''

import sys
input = sys.stdin.readline
output = sys.stdout.write

def solve(R, C, Q, queries):
	blocked = [[0]*C for _ in range(R)]

	# total staircases if no cell blocked
	res = R*C  # 1-block
	for c in range(C):  # right-down from row 0
		for ln in range(1, R+C+1):
			nr = ln // 2
			nc = c + (ln+1) // 2
			if nr >= R or nc >= C: break
			res += ln
	for r in range(R):  # down-right from col 0
		for ln in range(1, R+C+1):
			nr = r + (ln+1) // 2
			nc = ln // 2
			if nr >= R or nc >= C: break
			res += ln

	# each query (qr, qc) changes total by num staircases passing through it
	for qr, qc in queries:
		# num staircases passing through (qr, qc)
		change = 1

		# type 1
		r, c = qr, qc
		up = down = 0
		for ln in range(1, R+C+1):  # go farthest down-right from (qr, qc)
			nr = r + ln//2
			nc = c + (ln+1)//2
			if nr >= R or nc >= C or blocked[nr][nc]: break
			down += 1
		for ln in range(1, R+C+1):  # left-up
			nr = r - (ln+1)//2
			nc = c - ln//2
			if nr < 0 or nc < 0 or blocked[nr][nc]: break
			up += 1
		change += up + down + up * down

		# type 2
		r, c = qr, qc
		up = down = 0
		for ln in range(1, R+C+1):  # right-down
			nr = r + (ln+1)//2
			nc = c + ln//2
			if nr >= R or nc >= C or blocked[nr][nc]: break
			down += 1
		for ln in range(1, R+C+1):  # up-left
			nr = r - ln//2
			nc = c - (ln+1)//2
			if nr < 0 or nc < 0 or blocked[nr][nc]: break
			up += 1
		change += up + down + up * down

		# add or subtract to total
		res += change if blocked[qr][qc] else -change
		blocked[qr][qc] ^= 1
		output(str(res) + '\n')



def main():
	N, M, Q = list(map(int, input().split()))
	queries = []
	for _ in range(Q):
		x, y = list(map(int, input().split()))
		queries.append((x-1, y-1))
	solve(N, M, Q, queries)


if __name__ == '__main__':
	main()
