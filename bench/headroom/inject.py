"""Swap pydantic's three hot functions for the hand-written C in hrfast, everywhere they are referenced."""
import sys, gc
import hrfast

def install(verbose=False):
    import pydantic.json_schema as js
    import pydantic._internal._config as cfg
    hrfast.setup(cfg, cfg.ConfigWrapper)
    orig = {
        js._make_json_hashable: hrfast._make_json_hashable,
        js._deduplicate_schemas: hrfast._deduplicate_schemas,
    }
    old_getattr = cfg.ConfigWrapper.__dict__["__getattr__"]
    new_getattr = hrfast.instancemethod(hrfast.config_getattr)
    replaced = []
    # every module / class namespace holding a reference
    for modname, mod in list(sys.modules.items()):
        d = getattr(mod, "__dict__", None)
        if not isinstance(d, dict):
            continue
        for k, v in list(d.items()):
            try:
                if v in orig:   # hashable functions only
                    d[k] = orig[v]; replaced.append(f"{modname}.{k}")
            except TypeError:
                pass
    cfg.ConfigWrapper.__getattr__ = new_getattr
    replaced.append("ConfigWrapper.__getattr__")
    # any other holders (closures, default args, containers)?
    leftovers = [r for f in list(orig) + [old_getattr] for r in gc.get_referrers(f)
                 if not (isinstance(r, dict) and r is orig) and type(r).__name__ not in ("frame", "function", "cell")]
    if verbose:
        print("replaced:", replaced, file=sys.stderr)
        print("other referrers:", [type(r).__name__ for r in leftovers], file=sys.stderr)
    return replaced
