"""Reads a project: `pyproject.toml`'s `[project]` (PEP 621) and `[tool.pycc]`
tables, its core metadata, and the modules a wheel ships.

Nothing here imports or executes project code: metadata must be static (a
non-empty `dynamic` is refused), and modules are found by walking the source
tree without following symbolic links.
"""

from __future__ import annotations

import dataclasses
import os
import pathlib
import re
import tomllib


class ProjectError(Exception):
    """A project the backend cannot build, with a message saying why."""


# PEP 440's canonical version pattern (its appendix B), case-insensitive.
_VERSION = re.compile(
    r"""^\s*v?(?:(?:[0-9]+!)?[0-9]+(?:\.[0-9]+)*
    (?:[-_.]?(?:a|b|c|rc|alpha|beta|pre|preview)[-_.]?[0-9]*)?
    (?:-[0-9]+|[-_.]?(?:post|rev|r)[-_.]?[0-9]*)?
    (?:[-_.]?dev[-_.]?[0-9]*)?
    (?:\+[a-z0-9]+(?:[-_.][a-z0-9]+)*)?)\s*$""",
    re.IGNORECASE | re.VERBOSE,
)
# PEP 508 project names.
_NAME = re.compile(r"^([A-Z0-9]|[A-Z0-9][A-Z0-9._-]*[A-Z0-9])$", re.IGNORECASE)
_IDENTIFIER = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")

# Top-level directories and modules a flat layout never ships unless
# `[tool.pycc] packages`/`py-modules` names them (setuptools' list, roughly).
_FLAT_EXCLUDE = {
    "bench", "benches", "benchmark", "benchmarks", "bin", "build", "conftest", "dist",
    "doc", "docs", "example", "examples", "noxfile", "scripts", "setup", "site", "test",
    "tests", "tools", "venv",
}

_TOOL_KEYS = {
    "strict", "release", "packages", "py-modules", "source-dir", "pure-python",
    "fallback-to-python", "extra-args", "sdist-include", "overrides",
}
_OVERRIDE_KEYS = {"strict", "pure-python", "fallback-to-python", "extra-args"}


def normalize_name(name: str) -> str:
    """PEP 503's normalized name (`My.Project_x` -> `my-project-x`)."""
    return re.sub(r"[-_.]+", "-", name).lower()


def wheel_name(name: str) -> str:
    """The distribution name as wheel and sdist file names spell it."""
    return normalize_name(name).replace("-", "_")


@dataclasses.dataclass(frozen=True)
class ModuleOptions:
    """How one module is built."""

    strict: bool
    pure: bool
    fallback: bool
    extra_args: tuple[str, ...]


@dataclasses.dataclass(frozen=True)
class SourceFile:
    """A file the wheel ships: `src` on disk, `rel` its path in the wheel, and
    `module` its dotted name when it is a module pycc may compile."""

    src: pathlib.Path
    rel: str
    module: str | None


@dataclasses.dataclass
class ToolConfig:
    strict: bool = True
    release: bool = True
    packages: list[str] | None = None
    py_modules: list[str] | None = None
    source_dir: str | None = None
    pure_python: list[str] = dataclasses.field(default_factory=list)
    fallback: bool | list[str] = False
    extra_args: list[str] = dataclasses.field(default_factory=list)
    sdist_include: list[str] = dataclasses.field(default_factory=list)
    overrides: dict[str, dict] = dataclasses.field(default_factory=dict)

    def options(self, module: str) -> ModuleOptions:
        """The options `module` builds with: the project-wide settings, then the
        most specific `[tool.pycc.overrides]` entry naming it or a package above it."""
        fallback = self.fallback if isinstance(self.fallback, bool) \
            else any(_covers(p, module) for p in self.fallback)
        opts = {
            "strict": self.strict,
            "pure-python": any(_covers(p, module) for p in self.pure_python),
            "fallback-to-python": fallback,
            "extra-args": list(self.extra_args),
        }
        for prefix in sorted((p for p in self.overrides if _covers(p, module)), key=len):
            o = self.overrides[prefix]
            for key in ("strict", "pure-python", "fallback-to-python"):
                if key in o:
                    opts[key] = o[key]
            opts["extra-args"] += o.get("extra-args", [])
        return ModuleOptions(opts["strict"], opts["pure-python"], opts["fallback-to-python"],
                             tuple(opts["extra-args"]))


def _covers(prefix: str, module: str) -> bool:
    """Whether the dotted name `prefix` is `module` or a package above it (`pkg`
    covers `pkg.__init__` and `pkg.sub`; `pkg.__init__` only itself)."""
    return module == prefix or module.startswith(prefix + ".")


def _expect(value, kind, what: str):
    ok = isinstance(value, kind) and not (kind is int and isinstance(value, bool))
    if not ok:
        raise ProjectError(f"pyproject.toml: {what} must be a {_kind_name(kind)}")
    return value


def _kind_name(kind) -> str:
    return {bool: "boolean", str: "string", list: "array", dict: "table"}.get(kind, str(kind))


def _str_list(value, what: str) -> list[str]:
    _expect(value, list, what)
    for item in value:
        _expect(item, str, f"every item of {what}")
    return list(value)


def _dotted(value: str, what: str) -> str:
    if not all(_IDENTIFIER.match(part) for part in value.split(".")):
        raise ProjectError(f"pyproject.toml: {what}: {value!r} is not a dotted module name")
    return value


def parse_tool(table: dict) -> ToolConfig:
    """Validates `[tool.pycc]`; an unknown key is an error, so a typo cannot
    silently change what is built."""
    _expect(table, dict, "[tool.pycc]")
    unknown = set(table) - _TOOL_KEYS
    if unknown:
        raise ProjectError("pyproject.toml: unknown [tool.pycc] key(s): "
                           f"{', '.join(sorted(unknown))}"
                           f" (known: {', '.join(sorted(_TOOL_KEYS))})")
    cfg = ToolConfig()
    if "strict" in table:
        cfg.strict = _expect(table["strict"], bool, "[tool.pycc] strict")
    if "release" in table:
        cfg.release = _expect(table["release"], bool, "[tool.pycc] release")
    if "packages" in table:
        cfg.packages = [_dotted(p, "[tool.pycc] packages")
                        for p in _str_list(table["packages"], "[tool.pycc] packages")]
        for p in cfg.packages:
            if "." in p:
                raise ProjectError(f"pyproject.toml: [tool.pycc] packages: {p!r} is not a "
                                   "top-level package (its subpackages are included with it)")
    if "py-modules" in table:
        cfg.py_modules = [_dotted(m, "[tool.pycc] py-modules")
                          for m in _str_list(table["py-modules"], "[tool.pycc] py-modules")]
        for m in cfg.py_modules:
            if "." in m:
                raise ProjectError(f"pyproject.toml: [tool.pycc] py-modules: {m!r} is not a "
                                   "top-level module")
    if "source-dir" in table:
        cfg.source_dir = _expect(table["source-dir"], str, "[tool.pycc] source-dir")
    if "pure-python" in table:
        cfg.pure_python = [_dotted(m, "[tool.pycc] pure-python")
                           for m in _str_list(table["pure-python"], "[tool.pycc] pure-python")]
    if "fallback-to-python" in table:
        value = table["fallback-to-python"]
        if isinstance(value, bool):
            cfg.fallback = value
        else:
            cfg.fallback = [_dotted(m, "[tool.pycc] fallback-to-python")
                            for m in _str_list(value, "[tool.pycc] fallback-to-python "
                                               "(a boolean or an array of module names)")]
    if "extra-args" in table:
        cfg.extra_args = _str_list(table["extra-args"], "[tool.pycc] extra-args")
    if "sdist-include" in table:
        cfg.sdist_include = _str_list(table["sdist-include"], "[tool.pycc] sdist-include")
    if "overrides" in table:
        overrides = _expect(table["overrides"], dict, "[tool.pycc.overrides]")
        for module, o in overrides.items():
            what = f'[tool.pycc.overrides."{module}"]'
            _dotted(module, what)
            _expect(o, dict, what)
            unknown = set(o) - _OVERRIDE_KEYS
            if unknown:
                raise ProjectError(f"pyproject.toml: unknown {what} key(s): "
                                   f"{', '.join(sorted(unknown))} (known: "
                                   f"{', '.join(sorted(_OVERRIDE_KEYS))})")
            for key in ("strict", "pure-python", "fallback-to-python"):
                if key in o:
                    _expect(o[key], bool, f"{what} {key}")
            if "extra-args" in o:
                _str_list(o["extra-args"], f"{what} extra-args")
        cfg.overrides = dict(overrides)
    override_args = [a for o in cfg.overrides.values() for a in o.get("extra-args", [])]
    for arg in cfg.extra_args + override_args:
        if arg in ("-o", "--ext", "--target") or arg.startswith(("-o", "--target=")):
            raise ProjectError(f"pyproject.toml: extra-args may not set {arg!r}: the backend "
                               "chooses the output, the target and --ext itself")
    return cfg


@dataclasses.dataclass
class Project:
    root: pathlib.Path
    name: str
    version: str
    project: dict
    tool: ToolConfig

    @property
    def wheel_name(self) -> str:
        return wheel_name(self.name)

    @property
    def dist_info(self) -> str:
        return f"{self.wheel_name}-{self.version}.dist-info"


def load(root: pathlib.Path) -> Project:
    """Reads `root/pyproject.toml`."""
    path = root / "pyproject.toml"
    try:
        with open(path, "rb") as f:
            data = tomllib.load(f)
    except FileNotFoundError:
        raise ProjectError(f"{path} does not exist") from None
    except tomllib.TOMLDecodeError as e:
        raise ProjectError(f"{path}: {e}") from None
    project = data.get("project")
    if not isinstance(project, dict):
        raise ProjectError("pyproject.toml has no [project] table (PEP 621)")
    dynamic = project.get("dynamic", [])
    if dynamic:
        raise ProjectError("pyproject.toml: pycc.build reads only static metadata; set "
                           f"[project] {', '.join(map(str, dynamic))} directly and drop "
                           "`dynamic` (computing it would run project code)")
    name = _expect(project.get("name"), str, "[project] name")
    if not _NAME.match(name):
        raise ProjectError(f"pyproject.toml: [project] name {name!r} is not a valid project name")
    version = _expect(project.get("version"), str, "[project] version")
    if not _VERSION.match(version):
        raise ProjectError(f"pyproject.toml: [project] version {version!r} is not a "
                           "PEP 440 version")
    version = version.strip()
    tool = data.get("tool", {}).get("pycc", {})
    return Project(root, name, version, project, parse_tool(tool))


# ---------------------------------------------------------------------------
# Core metadata (version 2.1)


def _header(key: str, value: str) -> str:
    if "\n" in value or "\r" in value:
        raise ProjectError(f"pyproject.toml: the value of {key} must be a single line")
    return f"{key}: {value}\n"


def _contacts(people, what: str) -> tuple[str | None, str | None]:
    """PEP 621's authors/maintainers as the `X` and `X-email` fields."""
    _expect(people, list, f"[project] {what}")
    names, emails = [], []
    for person in people:
        _expect(person, dict, f"every item of [project] {what}")
        name, email = person.get("name"), person.get("email")
        if email is not None:
            emails.append(f'"{name}" <{email}>' if name else str(email))
        elif name is not None:
            names.append(str(name))
    return (", ".join(names) or None, ", ".join(emails) or None)


def readme(project: Project) -> tuple[str, str] | None:
    """The long description and its content type, from `[project] readme`."""
    value = project.project.get("readme")
    if value is None:
        return None
    types = {".md": "text/markdown", ".rst": "text/x-rst", ".txt": "text/plain"}
    if isinstance(value, str):
        path = safe_path(project.root, value, "[project] readme")
        ctype = types.get(path.suffix.lower())
        if ctype is None:
            raise ProjectError(f"pyproject.toml: cannot tell the content type of readme {value!r}; "
                               "use a table with content-type")
        return path.read_text(encoding="utf-8"), ctype
    _expect(value, dict, "[project] readme")
    ctype = _expect(value.get("content-type"), str, "[project] readme content-type")
    if "file" in value and "text" in value:
        raise ProjectError("pyproject.toml: [project] readme has both file and text")
    if "file" in value:
        path = safe_path(project.root, _expect(value["file"], str, "[project] readme file"),
                         "[project] readme")
        return path.read_text(encoding="utf-8"), ctype
    return _expect(value.get("text"), str, "[project] readme text"), ctype


def license_files(project: Project) -> list[pathlib.Path]:
    """Files under `[project] license = {file = ...}` and the `license-files` globs."""
    files: list[pathlib.Path] = []
    lic = project.project.get("license")
    if isinstance(lic, dict) and "file" in lic:
        files.append(safe_path(project.root, _expect(lic["file"], str, "[project] license file"),
                               "[project] license"))
    for pattern in _str_list(project.project.get("license-files", []), "[project] license-files"):
        if pattern.startswith("/") or ".." in pathlib.PurePosixPath(pattern).parts:
            raise ProjectError(f"pyproject.toml: license-files pattern {pattern!r} leaves "
                               "the project")
        matched = sorted(p for p in project.root.glob(pattern) if p.is_file())
        if not matched:
            raise ProjectError(f"pyproject.toml: license-files pattern {pattern!r} matches no file")
        files.extend(matched)
    unique = []
    for f in files:
        _no_links(project.root, f)
        if f not in unique:
            unique.append(f)
    return unique


def metadata(project: Project) -> str:
    """The `METADATA` (and sdist `PKG-INFO`) text: core metadata 2.1."""
    p = project.project
    out = ["Metadata-Version: 2.1\n", _header("Name", project.name),
           _header("Version", project.version)]
    if "description" in p:
        out.append(_header("Summary", _expect(p["description"], str, "[project] description")))
    for key, field in (("authors", "Author"), ("maintainers", "Maintainer")):
        if key in p:
            names, emails = _contacts(p[key], key)
            if names:
                out.append(_header(field, names))
            if emails:
                out.append(_header(f"{field}-email", emails))
    lic = p.get("license")
    if isinstance(lic, str):
        out.append(_header("License", lic))
    elif isinstance(lic, dict) and "text" in lic:
        text = _expect(lic["text"], str, "[project] license text")
        out.append("License: " + text.replace("\n", "\n        ") + "\n")
    elif lic is not None and not (isinstance(lic, dict) and "file" in lic):
        raise ProjectError("pyproject.toml: [project] license must be a string or a table "
                           "with file or text")
    if "keywords" in p:
        out.append(_header("Keywords", ",".join(_str_list(p["keywords"], "[project] keywords"))))
    for c in _str_list(p.get("classifiers", []), "[project] classifiers"):
        out.append(_header("Classifier", c))
    for label, url in _expect(p.get("urls", {}), dict, "[project] urls").items():
        out.append(_header("Project-URL", f"{label}, {_expect(url, str, '[project] urls')}"))
    if "requires-python" in p:
        out.append(_header("Requires-Python",
                           _expect(p["requires-python"], str, "[project] requires-python")))
    for dep in _str_list(p.get("dependencies", []), "[project] dependencies"):
        out.append(_header("Requires-Dist", dep))
    extras = _expect(p.get("optional-dependencies", {}), dict, "[project] optional-dependencies")
    for extra, deps in extras.items():
        out.append(_header("Provides-Extra", extra))
        for dep in _str_list(deps, f"[project] optional-dependencies {extra}"):
            req, _, marker = dep.partition(";")
            marker = marker.strip()
            cond = f'({marker}) and extra == "{extra}"' if marker else f'extra == "{extra}"'
            out.append(_header("Requires-Dist", f"{req.strip()}; {cond}"))
    long = readme(project)
    if long is not None:
        text, ctype = long
        out.append(_header("Description-Content-Type", ctype))
        out.append("\n" + text)
    return "".join(out)


def entry_points(project: Project) -> str | None:
    """`entry_points.txt` from `[project.scripts]`, `gui-scripts` and `entry-points`."""
    p = project.project
    groups: dict[str, dict] = {}
    for key, group in (("scripts", "console_scripts"), ("gui-scripts", "gui_scripts")):
        if key in p:
            groups[group] = _expect(p[key], dict, f"[project.{key}]")
    other = _expect(p.get("entry-points", {}), dict, "[project.entry-points]")
    for group, entries in other.items():
        if group in ("console_scripts", "gui_scripts"):
            raise ProjectError(f"pyproject.toml: use [project.scripts]/[project.gui-scripts], not "
                               f"[project.entry-points.{group}]")
        groups[group] = _expect(entries, dict, f"[project.entry-points.{group}]")
    if not groups:
        return None
    out = []
    for group, entries in groups.items():
        out.append(f"[{group}]\n")
        for name, target in entries.items():
            out.append(f"{name} = {_expect(target, str, f'entry point {name}')}\n")
        out.append("\n")
    return "".join(out)


# ---------------------------------------------------------------------------
# Sources


def safe_path(root: pathlib.Path, rel: str, what: str) -> pathlib.Path:
    """`root/rel`, refused when it is absolute, leaves `root`, is a symbolic link or
    is not a regular file or directory."""
    pure = pathlib.PurePosixPath(rel)
    if pure.is_absolute() or ".." in pure.parts or "\\" in rel:
        raise ProjectError(f"pyproject.toml: {what} {rel!r} must be a relative path inside "
                           "the project")
    path = root.joinpath(*pure.parts) if pure.parts else root
    probe = root
    for part in pure.parts:
        probe = probe / part
        if probe.is_symlink():
            raise ProjectError(f"{what} {rel!r}: {probe} is a symbolic link")
    if not path.exists():
        raise ProjectError(f"{what} {rel!r} does not exist")
    if not (path.is_file() or path.is_dir()):
        raise ProjectError(f"{what} {rel!r} is not a regular file or directory")
    return path


def _skipped(name: str) -> bool:
    return name == "__pycache__" or name.endswith((".pyc", ".pyo"))


def _walk(top: pathlib.Path) -> list[pathlib.Path]:
    """Every file under `top`, sorted, without following links; a link or a special
    file is an error rather than something silently shipped or skipped."""
    files = []
    for dirpath, dirs, names in os.walk(top, followlinks=False):
        dirs[:] = sorted(d for d in dirs if not _skipped(d))
        for name in sorted(dirs + names):
            q = pathlib.Path(dirpath) / name
            if q.is_symlink():
                raise ProjectError(f"{q} is a symbolic link; pycc.build does not ship links")
            if name in names and not _skipped(name):
                if not q.is_file():
                    raise ProjectError(f"{q} is not a regular file")
                files.append(q)
    return sorted(files)


def source_dir(project: Project) -> pathlib.Path:
    """`[tool.pycc] source-dir`, else `src/` when it holds a package or module, else the
    project root (flat layout)."""
    if project.tool.source_dir is not None:
        path = safe_path(project.root, project.tool.source_dir, "[tool.pycc] source-dir")
        if not path.is_dir():
            raise ProjectError(f"[tool.pycc] source-dir {project.tool.source_dir!r} is not a "
                               "directory")
        return path
    src = project.root / "src"
    if src.is_dir() and not src.is_symlink() and (
            any(src.glob("*/__init__.py")) or any(src.glob("*.py"))):
        return src
    return project.root


def _discover(project: Project, base: pathlib.Path) -> tuple[list[str], list[str]]:
    """The top-level packages and modules to ship when `[tool.pycc]` names none: the
    package or module named after the project when there is one, else everything in a
    `src/` layout, else everything a flat layout does not exclude."""
    own = wheel_name(project.name)
    if (base / own / "__init__.py").is_file():
        return [own], []
    if (base / f"{own}.py").is_file():
        return [], [own]
    flat = base == project.root
    packages, modules = [], []
    for entry in sorted(base.iterdir()):
        name = entry.name
        stem = entry.stem if entry.is_file() else name
        if entry.is_symlink() or name.startswith((".", "_")) or (flat and stem in _FLAT_EXCLUDE):
            continue
        if entry.is_dir() and (entry / "__init__.py").is_file() and _IDENTIFIER.match(name):
            if not (flat and name.startswith("test")):
                packages.append(name)
        elif entry.is_file() and entry.suffix == ".py" and _IDENTIFIER.match(entry.stem):
            if not (flat and entry.stem.startswith("test")):
                modules.append(entry.stem)
    if not packages and not modules:
        raise ProjectError(f"found no package or module to ship in {base}; name them with "
                           "[tool.pycc] packages / py-modules (and source-dir)")
    return packages, modules


def sources(project: Project) -> list[SourceFile]:
    """Every file the wheel ships, sorted by its path in the wheel."""
    base = source_dir(project)
    packages, modules = project.tool.packages, project.tool.py_modules
    if packages is None and modules is None:
        packages, modules = _discover(project, base)
    files: list[SourceFile] = []
    for mod in modules or []:
        path = safe_path(base, f"{mod}.py", "[tool.pycc] py-modules")
        files.append(SourceFile(path, f"{mod}.py", mod))
    for pkg in packages or []:
        top = safe_path(base, pkg, "[tool.pycc] packages")
        if not (top / "__init__.py").is_file():
            raise ProjectError(f"package {pkg!r} ({top}) has no __init__.py")
        for f in _walk(top):
            rel = f.relative_to(base).as_posix()
            files.append(SourceFile(f, rel, _module_name(base, f)))
    seen = set()
    for f in files:
        if f.rel in seen:
            raise ProjectError(f"{f.rel} is listed twice")
        seen.add(f.rel)
    return sorted(files, key=lambda f: f.rel)


def _module_name(base: pathlib.Path, path: pathlib.Path) -> str | None:
    """The dotted name of the module at `path` (`pkg.__init__` for a package's own
    module, so that configuration can name it apart from the package's other
    modules), or None for a file that is not one (data, or a `.py` in a directory
    without `__init__.py`, which ships as is)."""
    if path.suffix != ".py":
        return None
    parts = path.relative_to(base).with_suffix("").parts
    if not all(_IDENTIFIER.match(p) for p in parts):
        return None
    d = path.parent
    while d != base:
        if not (d / "__init__.py").is_file():
            return None
        d = d.parent
    return ".".join(parts)


def sdist_files(project: Project) -> list[pathlib.Path]:
    """Every file an sdist holds: pyproject.toml, readme, license files, the sources
    and `[tool.pycc] sdist-include` globs."""
    files = {project.root / "pyproject.toml"}
    value = project.project.get("readme")
    if isinstance(value, str):
        files.add(safe_path(project.root, value, "[project] readme"))
    elif isinstance(value, dict) and isinstance(value.get("file"), str):
        files.add(safe_path(project.root, value["file"], "[project] readme"))
    files.update(license_files(project))
    files.update(f.src for f in sources(project))
    for pattern in project.tool.sdist_include:
        if pattern.startswith("/") or ".." in pathlib.PurePosixPath(pattern).parts:
            raise ProjectError(f"[tool.pycc] sdist-include pattern {pattern!r} leaves the project")
        matched = False
        for p in sorted(project.root.glob(pattern)):
            if p.is_symlink():
                raise ProjectError(f"{p} is a symbolic link; pycc.build does not ship links")
            if p.is_dir():
                files.update(_walk(p))
                matched = True
            elif p.is_file():
                if not any(_skipped(part) for part in p.relative_to(project.root).parts):
                    files.add(p)
                matched = True
        if not matched:
            raise ProjectError(f"[tool.pycc] sdist-include pattern {pattern!r} matches nothing")
    for f in files:
        _no_links(project.root, f)
    return sorted(files, key=lambda f: f.relative_to(project.root).as_posix())


def _no_links(root: pathlib.Path, path: pathlib.Path) -> None:
    """Refuses `path` when it, or a directory between `root` and it, is a link."""
    probe = root
    for part in path.relative_to(root).parts:
        probe = probe / part
        if probe.is_symlink():
            raise ProjectError(f"{probe} is a symbolic link; pycc.build does not ship links")


__all__ = [
    "ModuleOptions", "Project", "ProjectError", "SourceFile", "ToolConfig", "entry_points",
    "license_files", "load", "metadata", "normalize_name", "parse_tool", "readme",
    "sdist_files", "source_dir", "sources", "wheel_name",
]
