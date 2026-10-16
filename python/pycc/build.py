"""`pycc.build`: a PEP 517 (and PEP 660) build backend that compiles a project's
modules to abi3 CPython extension modules with pycc.

    [build-system]
    requires = ["pycc-build"]
    build-backend = "pycc.build"

    [tool.pycc]
    strict = true                      # false: every module builds --non-strict
    pure-python = ["mypkg.cli"]        # modules (or whole packages) kept as .py
    fallback-to-python = false         # or a list: ship these as .py if pycc refuses them
    extra-args = []                    # more `pycc build` flags for every module
    # packages = ["mypkg"], py-modules = ["mod"], source-dir = "src": what ships,
    # when the default discovery (src/ or flat layout) does not find it.
    # [tool.pycc.overrides."mypkg.slow"] strict = false: per module or package.

The build runs no project code: metadata is static PEP 621 metadata, and the
sources are only read and handed to pycc. Every file it writes lands under the
directory pip passes it (a `.pycc-build-*` staging directory there is removed
before the hook returns), and pycc runs with `TMPDIR` inside that staging
directory. The README's "Use pycc in your project" section documents the rest.
"""

from __future__ import annotations

import os
import pathlib
import shutil
import sys
import tempfile

from . import __version__
from ._compile import CompileError, compile_modules, find_pycc
from ._project import (Project, ProjectError, entry_points, license_files, load, metadata,
                       sdist_files, source_dir, sources)
from ._wheel import ABI3_PYTHON, TagError, WheelWriter, platform_tag, write_sdist

__all__ = [
    "build_editable", "build_sdist", "build_wheel", "get_requires_for_build_editable",
    "get_requires_for_build_sdist", "get_requires_for_build_wheel",
    "prepare_metadata_for_build_editable", "prepare_metadata_for_build_wheel",
]


class BuildError(Exception):
    """A build that failed; the message says why (pycc's diagnostics included)."""


def get_requires_for_build_wheel(config_settings=None) -> list[str]:
    return []


def get_requires_for_build_sdist(config_settings=None) -> list[str]:
    return []


def get_requires_for_build_editable(config_settings=None) -> list[str]:
    return []


def _project() -> Project:
    try:
        return load(pathlib.Path.cwd())
    except ProjectError as e:
        raise BuildError(str(e)) from None


def _dist_info_files(project: Project) -> list[tuple[str, bytes]]:
    """METADATA, entry_points.txt and the license files (WHEEL and RECORD aside)."""
    files = [("METADATA", metadata(project).encode())]
    eps = entry_points(project)
    if eps is not None:
        files.append(("entry_points.txt", eps.encode()))
    for f in license_files(project):
        files.append((f"licenses/{f.relative_to(project.root).as_posix()}", f.read_bytes()))
    return files


def _wheel_file(tag: str, pure: bool) -> bytes:
    return (f"Wheel-Version: 1.0\nGenerator: pycc.build {__version__}\n"
            f"Root-Is-Purelib: {'true' if pure else 'false'}\nTag: {tag}\n").encode()


def _write_metadata_dir(metadata_directory: str) -> str:
    project = _project()
    try:
        files = _dist_info_files(project)
    except ProjectError as e:
        raise BuildError(str(e)) from None
    target = pathlib.Path(metadata_directory) / project.dist_info
    for rel, data in files:
        path = target / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    return project.dist_info


def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None) -> str:
    return _write_metadata_dir(metadata_directory)


def prepare_metadata_for_build_editable(metadata_directory, config_settings=None) -> str:
    return _write_metadata_dir(metadata_directory)


def _finish(project: Project, writer: WheelWriter, wheel_directory, tag: str, pure: bool) -> str:
    for rel, data in _dist_info_files(project):
        writer.add(f"{project.dist_info}/{rel}", data)
    writer.add(f"{project.dist_info}/WHEEL", _wheel_file(tag, pure))
    name = f"{project.wheel_name}-{project.version.replace('-', '_')}-{tag}.whl"
    writer.write(pathlib.Path(wheel_directory) / name)
    return name


def build_wheel(wheel_directory, config_settings=None, metadata_directory=None) -> str:
    """Compiles every module not kept in Python and writes the wheel. A wheel with
    no compiled module is pure (`py3-none-any`); otherwise it is tagged
    `cp313-abi3-<platform>` (`_wheel.platform_tag`)."""
    project = _project()
    try:
        files = sources(project)
        compile_list = []
        for f in files:
            if f.module is None:
                continue
            options = project.tool.options(f.module)
            if not options.pure:
                compile_list.append((f.module, f.rel, options))
        writer = WheelWriter(project.dist_info)
        if not compile_list:
            for f in files:
                writer.add(f.rel, f.src.read_bytes())
            return _finish(project, writer, wheel_directory, "py3-none-any", True)
        if sys.implementation.name != "cpython" or sys.version_info < (3, 13):
            raise BuildError("pycc compiles abi3 extension modules for CPython 3.13 or newer; "
                             f"this build runs {sys.implementation.name} "
                             f"{sys.version.split()[0]}")
        pycc = find_pycc()
        staging = pathlib.Path(tempfile.mkdtemp(prefix=".pycc-build-", dir=wheel_directory))
        try:
            src = staging / "src"
            tmp = staging / "tmp"
            tmp.mkdir()
            for f in files:
                if f.rel.endswith(".py"):
                    (src / f.rel).parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(f.src, src / f.rel, follow_symlinks=False)
            env = dict(os.environ)
            env["TMPDIR"] = str(tmp)
            env.setdefault("PYCC_PYTHON", sys.executable)
            print(f"pycc.build {__version__}: compiling {len(compile_list)} module(s) of "
                  f"{project.name} {project.version} with {pycc}", file=sys.stderr)
            outcomes = {o.module: o for o in compile_modules(
                pycc, src, compile_list, project.tool.release, env, source_dir(project))}
            compiled = []
            for f in files:
                o = outcomes.get(f.module) if f.module is not None else None
                if o is not None and o.artifact is not None:
                    stem = f.rel[:-len(".py")]
                    suffix = o.artifact.name[len(pathlib.PurePosixPath(stem).name):]
                    writer.add(stem + suffix, o.artifact.read_bytes(), executable=True)
                    compiled.append(o.artifact)
                else:
                    writer.add(f.rel, f.src.read_bytes())
            if not compiled:
                return _finish(project, writer, wheel_directory, "py3-none-any", True)
            tag = f"{ABI3_PYTHON}-abi3-{platform_tag(compiled)}"
            return _finish(project, writer, wheel_directory, tag, False)
        finally:
            shutil.rmtree(staging, ignore_errors=True)
    except (ProjectError, CompileError, TagError) as e:
        raise BuildError(str(e)) from None


def build_editable(wheel_directory, config_settings=None, metadata_directory=None) -> str:
    """An editable install (PEP 660) runs the sources in place, uncompiled: the wheel
    holds a `.pth` file that puts the source directory on `sys.path`. Rebuild with
    `pip install .` to get the compiled modules."""
    project = _project()
    try:
        base = source_dir(project).resolve()
        writer = WheelWriter(project.dist_info)
        writer.add(f"_pycc_editable_{project.wheel_name}.pth", (str(base) + "\n").encode())
        return _finish(project, writer, wheel_directory, "py3-none-any", True)
    except ProjectError as e:
        raise BuildError(str(e)) from None


def build_sdist(sdist_directory, config_settings=None) -> str:
    """A `.tar.gz` of the sources, pyproject.toml, readme and license files (and the
    `[tool.pycc] sdist-include` globs), with PKG-INFO."""
    project = _project()
    try:
        files = [(f.relative_to(project.root).as_posix(), f.read_bytes())
                 for f in sdist_files(project)]
        files.append(("PKG-INFO", metadata(project).encode()))
        top = f"{project.wheel_name}-{project.version}"
        name = f"{top}.tar.gz"
        write_sdist(pathlib.Path(sdist_directory) / name, top, files)
        return name
    except ProjectError as e:
        raise BuildError(str(e)) from None
