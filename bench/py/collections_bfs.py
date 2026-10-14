from collections import Counter, deque


def bfs_counts(n: int) -> int:
    # BFS over the implicit graph i -> (2i % n, (i + 1) % n), counting distances.
    dist = [-1] * n
    dist[0] = 0
    queue: deque[int] = deque([0])
    while queue:
        u = queue.popleft()
        a = (2 * u) % n
        if dist[a] < 0:
            dist[a] = dist[u] + 1
            queue.append(a)
        b = (u + 1) % n
        if dist[b] < 0:
            dist[b] = dist[u] + 1
            queue.append(b)
    counts: Counter[int] = Counter()
    for d in dist:
        counts[d] += 1
    return counts.most_common(1)[0][1] + len(counts)


total = 0
for _ in range(10):
    total += bfs_counts(300_000)
print(total)
