# Imports `m` a second time after it has left `sys.modules`: a new
# namespace, the same compiled reader.
import sys

import m

del sys.modules["m"]
import m as again

print(again is m, again.RUN)
