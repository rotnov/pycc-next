"""`return` out of `for x in <object>:` loops (#42).

CPython pops a `for` loop's iterator when a `return` unwinds the loop, before
any `finally` the return then passes through. The functions here return from
inside such loops, through `finally` clauses that sit inside and outside them.
`tests/return_releases_loop_iterator.rs` imports this module compiled with
`--ext` and as source and counts the `list_iterator` objects each function
leaves behind.
"""

from typing import Any, Dict, List


def early(tokens: List[Dict[str, Any]]) -> bool:
    for token in tokens:
        if token["type"] == "text":
            return True
    return False


def early_void(items: List[Any], log: List[Any]) -> None:
    for item in items:
        log.append("body")
        return
    log.append("after")


def nested(outer: List[Any], inner: List[Any], log: List[Any]) -> int:
    for a in outer:
        for b in inner:
            log.append("body")
            return len(log)
    return -1


def try_in_loop(items: List[Any], log: List[Any]) -> int:
    for item in items:
        try:
            return len(log)
        finally:
            log.append("finally")
    return -1


def loop_in_try(items: List[Any], log: List[Any]) -> int:
    try:
        for item in items:
            return len(log)
    finally:
        log.append("finally")
    return -1


def between(outer: List[Any], inner: List[Any], log: List[Any]) -> int:
    for a in outer:
        try:
            for b in inner:
                try:
                    return len(log)
                finally:
                    log.append("inner finally")
        finally:
            log.append("outer finally")
    return -1


def overridden(items: List[Any], log: List[Any]) -> int:
    for item in items:
        try:
            return len(log)
        finally:
            log.append("finally")
            for again in items:
                return len(log) + 10
    return -1
