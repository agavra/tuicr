#!/usr/bin/env python3
"""Entry point of the bin/{gh,glab,tea,bkt} wrappers: main.py <tool> <args...>.

An adapter that raises (a missing fixture file, a bug) is a fake crash: it is logged as an UNEXPECTED call (exit 97),
so the scenario fails even when tuicr treats the failed call as best effort."""

import importlib
import os
import sys
import traceback

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from fakeforge.core import Fake  # noqa: E402

tool = sys.argv[1]
fake = Fake(tool, sys.argv[2:])
try:
    importlib.import_module("fakeforge." + tool).handle(fake)
except Exception:  # noqa: BLE001 - SystemExit (the fakes' normal exits) is not an Exception
    fake.unexpected("fake crashed: " + traceback.format_exc().strip().replace("\n", " | "))
