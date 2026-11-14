#!/usr/bin/env python3
"""Entry point of the bin/{gh,glab,tea,bkt} wrappers: main.py <tool> <args...>."""
import importlib
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from fakeforge.core import Fake  # noqa: E402

tool = sys.argv[1]
importlib.import_module("fakeforge." + tool).handle(Fake(tool, sys.argv[2:]))
