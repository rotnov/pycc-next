"""pycc-next's Python-side tooling: `pycc.build`, a PEP 517 build backend.

A project selects it in its `pyproject.toml`:

    [build-system]
    requires = ["pycc-build"]
    build-backend = "pycc.build"

and `pip wheel .` / `pip install .` then compile its modules to abi3 CPython
extension modules with the `pycc` compiler (`pycc build --ext`). The README's
"Use pycc in your project" section is the user documentation.
"""

__version__ = "0.8.0"
