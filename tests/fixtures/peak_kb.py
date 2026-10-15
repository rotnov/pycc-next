# The running process's own peak resident set, in kilobytes, for memory
# probes that measure from inside Python (`tests/memory_support/mod.rs`
# prepends this to them). On Linux it is `VmHWM`: `ru_maxrss` also counts
# the resident set of the test harness that spawned the process, which
# `exec` folds into it (see that module's documentation).
import resource
import sys


def peak_kb() -> int:
    try:
        with open("/proc/self/status") as status:
            for line in status:
                if line.startswith("VmHWM:"):
                    return int(line.split()[1])
    except OSError:
        pass
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    # `ru_maxrss` is in bytes on macOS and in kilobytes elsewhere.
    return peak // 1024 if sys.platform == "darwin" else peak
