# Issue #20: imports `api` (`issue_20_api.py`, as source or built `--ext`)
# and checks what its fallback code reads and binds by a string.
import api

api.reads()
print(api.writes())
print(api.count, api.label, api.p.x, api.read_count(), api.read_point(), api.read_hidden())
api.count = 11
print(api.read_count())
print(eval("api.p.norm1()"))
