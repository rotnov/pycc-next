"""Unit tests of the harness's outcome parsing: `python3 fuzz/test_harness.py`."""

from __future__ import annotations

import unittest

import harness

WARNING = ("/w/p1.py:1: DeprecationWarning: Bitwise inversion '~' on bool is deprecated\n"
           "  ~False\n")


class LastException(unittest.TestCase):
    def test_a_warning_before_a_system_exit_message_is_dropped(self) -> None:
        self.assertEqual(harness.last_exception(WARNING + "bye\n", "__main__"), "bye")

    def test_a_warning_before_a_bare_system_exit_is_dropped(self) -> None:
        self.assertEqual(harness.last_exception(WARNING + WARNING, "__main__"), "")

    def test_a_warning_before_a_traceback_is_dropped(self) -> None:
        stderr = WARNING + "Traceback (most recent call last):\n  File \"p1.py\", line 2\nValueError: x\n"
        self.assertEqual(harness.last_exception(stderr, "__main__"), "ValueError: x")

    def test_an_exception_message_shaped_like_a_warning_is_kept(self) -> None:
        for kind in ("ValueError", "RuntimeError"):
            line = f"{kind}: config.py:12: UserWarning: invalid value"
            stderr = ("Traceback (most recent call last):\n"
                      "  File \"p1.py\", line 1, in <module>\n" + line + "\n")
            self.assertEqual(harness.last_exception(stderr, "__main__"), line)

    def test_a_system_exit_message_shaped_like_a_warning_is_kept(self) -> None:
        message = "config.py:12: UserWarning: invalid value"
        self.assertEqual(harness.last_exception(message + "\n", "__main__"), message)
        self.assertEqual(harness.last_exception(WARNING + message + "\n", "__main__"), message)


if __name__ == "__main__":
    unittest.main()
