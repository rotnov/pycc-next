# Issue #20: the entry module of a program importing a project module
# (`issue_20_project_helper.py`, copied as `helper.py`). Its fallback code
# reaches globals by a string; the project import stays compiled.
from helper import answer

total = answer() + 1
print(answer())
print(eval("1"), eval("total"))
