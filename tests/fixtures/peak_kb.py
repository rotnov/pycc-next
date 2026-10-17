# The running process's own peak resident set, in kilobytes, for memory
# probes that measure from inside Python (`tests/memory_support/mod.rs`
# prepends this to them). On Linux it is `VmHWM`: `ru_maxrss` also counts
# the resident set of the test harness that spawned the process, which
# `exec` folds into it (see that module's documentation). On Windows it is
# the process's peak working set (`GetProcessMemoryInfo`).
import sys

if sys.platform == "win32":
    import ctypes
    from ctypes import wintypes

    class _Counters(ctypes.Structure):
        _fields_ = [
            ("cb", wintypes.DWORD),
            ("PageFaultCount", wintypes.DWORD),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
        ]

    def peak_kb() -> int:
        counters = _Counters()
        counters.cb = ctypes.sizeof(counters)
        process = ctypes.windll.kernel32.GetCurrentProcess()
        if not ctypes.windll.psapi.GetProcessMemoryInfo(
            ctypes.c_void_p(process), ctypes.byref(counters), counters.cb
        ):
            raise OSError("GetProcessMemoryInfo failed")
        return counters.PeakWorkingSetSize // 1024

else:
    import resource

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
