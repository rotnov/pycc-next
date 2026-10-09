class PluginMissing(ImportError):
    pass


def load_plain() -> None:
    raise ImportError("plain import failure")


def load_missing() -> None:
    raise ModuleNotFoundError("No module named 'zz'")


def load_plugin() -> None:
    raise PluginMissing("plugin gone")


def main() -> None:
    try:
        load_plain()
    except ImportError as e:
        print("ImportError:", e)
        print("again:", e)
    try:
        load_missing()
    except ModuleNotFoundError as e:
        print("ModuleNotFoundError:", e)
    try:
        load_missing()
    except ImportError as e:
        print("caught as ImportError:", e)
    try:
        load_missing()
    except Exception as e:
        print("caught as Exception:", e)
    try:
        load_plain()
    except ModuleNotFoundError as e:
        print("wrong handler:", e)
    except ImportError as e:
        print("second handler:", e)
    try:
        load_plugin()
    except ImportError as e:
        print("user subclass:", e)
    print(issubclass(ModuleNotFoundError, ImportError), issubclass(ImportError, ModuleNotFoundError))
    print(issubclass(PluginMissing, ImportError), issubclass(ImportError, Exception))


main()
