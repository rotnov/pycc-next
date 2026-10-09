/*
 * pycc embedded-executable stub for a Windows host (D-253, Part 1 of #1226).
 *
 * Windows has no rpath: an executable's static imports resolve from its own
 * directory, System32 and PATH before any of its code runs, so the
 * executable `OUT` cannot import `python314.dll` from `OUT.pycc\`. This
 * stub is `OUT` instead. It imports only the system DLLs KERNEL32.dll and
 * ntdll.dll (it is linked with the static C runtime), finds `<its own directory>\<sidecar>\pycc_program.dll`,
 * loads it so that the DLL's own imports (`python314.dll`, `python3.dll`,
 * the VC runtime) resolve from the sidecar, and returns what the DLL's
 * `pycc_embed_main` returns as the process exit status.
 *
 * It holds no Python code and no boundary conversion: the launcher inside
 * the program DLL owns the interpreter lifecycle (D-248 rule 6, widened by
 * this one Python-free file). It does not resolve symlinks (D-253).
 *
 * On failure it prints `pycc: cannot load <path> (error N)` and exits 121.
 */
#include <stdio.h>
#include <string.h>
#include <wchar.h>
#include <windows.h>

/* Defines PYCC_EMBED_SIDECAR, the sidecar directory's UTF-8 file name. */
#include "pycc_embed_config.inc"

/* The program DLL's fixed name; `layout::PROGRAM_DLL_NAME` in pycc. */
#define PYCC_PROGRAM_DLL L"pycc_program.dll"

#define PYCC_STUB_LOAD_FAILURE 121

typedef int (*pycc_embed_main_fn)(int argc, wchar_t **argv, const wchar_t *sidecar);

static int pycc_stub_fail(const wchar_t *path, DWORD error) {
    fwprintf(stderr, L"pycc: cannot load %ls (error %lu)\n", path, (unsigned long)error);
    return PYCC_STUB_LOAD_FAILURE;
}

int wmain(int argc, wchar_t **argv) {
    static wchar_t sidecar[32768];
    static wchar_t program[32768];
    const DWORD capacity = (DWORD)(sizeof sidecar / sizeof sidecar[0]);
    /* The executable's own path, then its directory. */
    DWORD n = GetModuleFileNameW(NULL, sidecar, capacity);
    if (n == 0 || n >= capacity) {
        return pycc_stub_fail(L"<the executable's own path>", GetLastError());
    }
    wchar_t *slash = wcsrchr(sidecar, L'\\');
    if (slash == NULL) {
        return pycc_stub_fail(sidecar, ERROR_BAD_PATHNAME);
    }
    slash[1] = L'\0';
    size_t used = wcslen(sidecar);
    /* The sidecar name is UTF-8 in the generated header; widen it. */
    int widened = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, PYCC_EMBED_SIDECAR, -1,
                                      sidecar + used, (int)(capacity - used));
    if (widened == 0) {
        return pycc_stub_fail(sidecar, GetLastError());
    }
    /* `<sidecar>\pycc_program.dll`, built with no deprecated CRT call. */
    size_t sidecar_len = wcslen(sidecar);
    size_t name_len = wcslen(PYCC_PROGRAM_DLL);
    if (sidecar_len + 1 + name_len >= capacity) {
        return pycc_stub_fail(sidecar, ERROR_FILENAME_EXCED_RANGE);
    }
    wmemcpy(program, sidecar, sidecar_len);
    program[sidecar_len] = L'\\';
    wmemcpy(program + sidecar_len + 1, PYCC_PROGRAM_DLL, name_len + 1);
    /* No loader dialog: a missing DLL must fail the process, not block it. */
    SetErrorMode(SEM_FAILCRITICALERRORS);
    HMODULE module = LoadLibraryExW(
        program, NULL, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (module == NULL) {
        return pycc_stub_fail(program, GetLastError());
    }
    pycc_embed_main_fn entry = (pycc_embed_main_fn)(void (*)(void))GetProcAddress(
        module, "pycc_embed_main");
    if (entry == NULL) {
        return pycc_stub_fail(program, GetLastError());
    }
    return entry(argc, argv, sidecar);
}
