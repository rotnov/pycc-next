/*
 * pycc embedded-executable launcher (Part 1 of #1028).
 *
 * The `main` of an embedded executable: it starts the CPython interpreter
 * bundled in the sidecar directory beside the executable, then runs the
 * pycc-compiled program as that interpreter's `__main__` module. It holds
 * interpreter *lifecycle* code only. Every PyObject* that crosses between
 * pycc code and CPython still goes through `src/ext/pycc_ext_module.c`,
 * compiled unchanged into the same link under the module name `__main__`
 * (the #1045 bridge split, recorded in the embedded-executable decision
 * entry under `docs/decisions/`).
 *
 * Unlike the shim, this file uses the non-limited API: `PyConfig_*` is not
 * part of the stable ABI. The two are separate translation units, so the
 * shim's own `Py_LIMITED_API` definition does not reach this one.
 *
 * On Windows (D-253) this file is linked into the program DLL
 * `OUT.pycc\pycc_program.dll` instead of the executable: it exports
 * `pycc_embed_main`, which the stub `OUT` (`pycc_embed_stub_windows.c`)
 * calls with the sidecar's path. The POSIX `main` below is not compiled
 * there. Both entry points share `pycc_embed_run`.
 */
#include <Python.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifndef _WIN32
#include <libgen.h>
#include <limits.h>
#if defined(__APPLE__)
#include <mach-o/dyld.h>
#else
#include <unistd.h>
#endif
#else
#include <windows.h>
#endif

/* Defines PYCC_EMBED_SIDECAR: the sidecar directory's file name, fixed at
 * build time from `-o`'s file name (never derived from the running
 * executable's name, so renaming the executable keeps it working). Also
 * defines PYCC_EMBED_CLOSURE when the sidecar holds a locked dependency
 * closure in `closure/` (#1242), and, on Windows, PYCC_EMBED_NATIVES when
 * it holds the closure's natives in `natives\` (#1306). */
#include "pycc_embed_config.inc"

extern PyObject *PyInit___main__(void);

/* Runs the compiled program as `__main__` in the initialized interpreter,
 * then finalizes it. Returns the process exit status: 0, 1 after an
 * uncaught exception, or 120 when finalization fails. */
static int pycc_embed_run(void) {
    int rc = 1;
    PyObject *def = PyInit___main__();
    PyObject *machinery = def ? PyImport_ImportModule("importlib.machinery") : NULL;
    PyObject *spec =
        machinery ? PyObject_CallMethod(machinery, "ModuleSpec", "sO", "__main__", Py_None) : NULL;
    PyObject *module = spec ? PyModule_FromDefAndSpec((PyModuleDef *)def, spec) : NULL;
    if (module != NULL &&
        PyDict_SetItemString(PyImport_GetModuleDict(), "__main__", module) == 0 &&
        PyModule_ExecDef(module, (PyModuleDef *)def) == 0) {
        rc = 0;
    }
    if (rc != 0) {
        /* A `SystemExit` exits the process here with its own code. */
        PyErr_Print();
    }
    Py_XDECREF(module);
    Py_XDECREF(spec);
    Py_XDECREF(machinery);
    if (Py_FinalizeEx() < 0) {
        rc = 120;
    }
    return rc;
}

#ifdef _WIN32
/* The program DLL's entry point, called by the stub `OUT` with its own
 * argv and the sidecar directory's full path (D-253). The search path is
 * explicit -- `<sidecar>\Lib`, then `<sidecar>\DLLs`, then the locked
 * closure `<sidecar>\closure` when the sidecar holds one (#1296) -- so
 * nothing is inherited from `getpath`, the registry or the environment. */
__declspec(dllexport) int pycc_embed_main(int argc, wchar_t **argv, const wchar_t *sidecar) {
    static wchar_t lib[32768];
    static wchar_t dlls[32768];
    const size_t size = sizeof lib / sizeof lib[0];
    if (_snwprintf(lib, size, L"%ls\\Lib", sidecar) < 0 ||
        _snwprintf(dlls, size, L"%ls\\DLLs", sidecar) < 0) {
        fwprintf(stderr, L"error: pycc could not form the search path under %ls\n", sidecar);
        return 1;
    }
    lib[size - 1] = L'\0';
    dlls[size - 1] = L'\0';
#ifdef PYCC_EMBED_CLOSURE
    static wchar_t closure[32768];
    if (_snwprintf(closure, size, L"%ls\\closure", sidecar) < 0) {
        fwprintf(stderr, L"error: pycc could not form the search path under %ls\n", sidecar);
        return 1;
    }
    closure[size - 1] = L'\0';
#endif
    /* The root DLLs resolve for every later extension load's static
     * imports: CPython loads extensions and `ctypes` libraries with
     * LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, which includes this directory
     * before System32 (#1305), as `os.add_dll_directory` would. */
    if (AddDllDirectory(sidecar) == NULL) {
        fwprintf(stderr, L"error: pycc could not add the DLL directory %ls (error %lu)\n",
                 sidecar, GetLastError());
        return 1;
    }
#ifdef PYCC_EMBED_NATIVES
    /* The closure's natives resolve for its extensions' static imports the
     * same way (#1306). */
    static wchar_t natives[32768];
    if (_snwprintf(natives, size, L"%ls\\natives", sidecar) < 0) {
        fwprintf(stderr, L"error: pycc could not form the search path under %ls\n", sidecar);
        return 1;
    }
    natives[size - 1] = L'\0';
    if (AddDllDirectory(natives) == NULL) {
        fwprintf(stderr, L"error: pycc could not add the DLL directory %ls (error %lu)\n",
                 natives, GetLastError());
        return 1;
    }
#endif
    PyConfig config;
    PyConfig_InitIsolatedConfig(&config);
    config.site_import = 0;
    config.write_bytecode = 0;
    /* Unbuffered for the reason the POSIX `main` gives. */
    config.buffered_stdio = 0;
    PyStatus status = PyConfig_SetString(&config, &config.home, sidecar);
    config.module_search_paths_set = 1;
    if (!PyStatus_Exception(status)) {
        status = PyWideStringList_Append(&config.module_search_paths, lib);
    }
    if (!PyStatus_Exception(status)) {
        status = PyWideStringList_Append(&config.module_search_paths, dlls);
    }
#ifdef PYCC_EMBED_CLOSURE
    if (!PyStatus_Exception(status)) {
        status = PyWideStringList_Append(&config.module_search_paths, closure);
    }
#endif
    if (!PyStatus_Exception(status)) {
        status = PyConfig_SetArgv(&config, argc, argv);
    }
    if (!PyStatus_Exception(status)) {
        status = Py_InitializeFromConfig(&config);
    }
    PyConfig_Clear(&config);
    if (PyStatus_Exception(status)) {
        Py_ExitStatusException(status);
    }
    return pycc_embed_run();
}
#else
/* Writes the directory holding the running executable, symlinks resolved,
 * into `dir`. Returns 0 on success. */
static int pycc_embed_exe_dir(char *dir, size_t size) {
    char exe[PATH_MAX];
    char real[PATH_MAX];
#if defined(__APPLE__)
    uint32_t n = sizeof exe;
    if (_NSGetExecutablePath(exe, &n) != 0) {
        return -1;
    }
#else
    ssize_t n = readlink("/proc/self/exe", exe, sizeof exe - 1);
    if (n < 0) {
        return -1;
    }
    exe[n] = '\0';
#endif
    if (realpath(exe, real) == NULL) {
        return -1;
    }
    if ((size_t)snprintf(dir, size, "%s", dirname(real)) >= size) {
        return -1;
    }
    return 0;
}

int main(int argc, char **argv) {
    char dir[PATH_MAX];
    char home[PATH_MAX];
    if (pycc_embed_exe_dir(dir, sizeof dir) != 0 ||
        (size_t)snprintf(home, sizeof home, "%s/%s", dir, PYCC_EMBED_SIDECAR) >= sizeof home) {
        fprintf(stderr, "error: pycc could not locate its own executable to find %s\n",
                PYCC_EMBED_SIDECAR);
        return 1;
    }
    PyConfig config;
    PyConfig_InitIsolatedConfig(&config);
    config.site_import = 0;
    config.write_bytecode = 0;
    /* Unbuffered Python-side stdio is what keeps a Python-side write (for
     * example `pprint.pprint`) and a pycc `print` in program order: pycc_rt
     * writes through Rust's line-buffered stdout, which flushes at every
     * newline, and every pycc `print` ends in one while `print(end=...)` is
     * still refused (C0001). The pycc change that admits `end=` must add an
     * explicit Rust-side stdout flush before each foreign call (risk R3 of
     * the #1028 plan). */
    config.buffered_stdio = 0;
    PyStatus status = PyConfig_SetBytesString(&config, &config.home, home);
    /* The sidecar always holds the standard library (with its lib-dynload)
     * under `lib/`, whatever the build host's `sys.platlibdir` was (a
     * distribution build may say `lib64`), so the search path must not
     * inherit the compiled-in value. */
    if (!PyStatus_Exception(status)) {
        status = PyConfig_SetBytesString(&config, &config.platlibdir, "lib");
    }
    if (!PyStatus_Exception(status)) {
        status = PyConfig_SetBytesArgv(&config, argc, argv);
    }
    if (!PyStatus_Exception(status)) {
        status = Py_InitializeFromConfig(&config);
    }
    PyConfig_Clear(&config);
    if (PyStatus_Exception(status)) {
        Py_ExitStatusException(status);
    }
#ifdef PYCC_EMBED_CLOSURE
    /* The locked closure is appended after the bundled standard library,
     * so a closure package never shadows a standard-library module. */
    {
        char closure[PATH_MAX];
        PyObject *path = PySys_GetObject("path"); /* borrowed */
        PyObject *entry = NULL;
        if ((size_t)snprintf(closure, sizeof closure, "%s/closure", home) < sizeof closure) {
            entry = PyUnicode_DecodeFSDefault(closure);
        } else {
            PyErr_SetString(PyExc_OSError, "the closure directory's path is too long");
        }
        if (entry == NULL || path == NULL || PyList_Append(path, entry) != 0) {
            if (!PyErr_Occurred()) {
                PyErr_SetString(PyExc_RuntimeError, "sys.path is missing");
            }
            PyErr_Print();
            Py_XDECREF(entry);
            Py_FinalizeEx();
            return 1;
        }
        Py_DECREF(entry);
    }
#endif
    return pycc_embed_run();
}
#endif /* _WIN32 */
