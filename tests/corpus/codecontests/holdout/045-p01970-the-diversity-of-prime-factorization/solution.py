from collections import defaultdict
MAX = 1000000
ROOT = 1000
MOD = 1000000007
is_prime = [True] * (MAX + 1)
is_prime[0] = is_prime[1] = False
for i in range(2, ROOT + 1):
  if is_prime[i]:
    for j in range(i * i, MAX + 1, i):
      is_prime[j] = False

n = int(input())
qlst = list(map(int, input().split()))
total1 = 0#next is kisuu or sisuu
total2 = 1#next is kisuu only(pre is index)
last_prime = 0
dic = {}
dic[(last_prime, 0)] = total1
dic[(last_prime, 1)] = total2
for q in qlst:
  new_dic = defaultdict(int)
  for k, v in dic.items():
    last_prime, t = k
    if is_prime[q]:
      if t == 0:
        if last_prime < q:
          new_dic[(q, 0)] = (new_dic[(q, 0)] + v) % MOD
          new_dic[(last_prime, 1)] = (new_dic[(last_prime, 1)] + v) % MOD
        else:
          new_dic[(last_prime, 1)] = (new_dic[(last_prime, 1)] + v) % MOD
      else:
        if last_prime < q:
          new_dic[(q, 0)] = (new_dic[(q, 0)] + v) % MOD
    
    if not is_prime[q]:
      if t == 0:
        new_dic[(last_prime, 1)] = (new_dic[(last_prime, 1)] + v) % MOD
  dic = new_dic
print(sum(dic.values()) % MOD)
