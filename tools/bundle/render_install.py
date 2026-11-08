#!/usr/bin/env python3
"""Fill docs/colleague-install.md for one bundle (tools/colleague-bundle.sh).

    NOTETAKER=0|1 VERSION=... [other {{NAME}} values] render_install.py TEMPLATE > INSTALL.md

{{NAME}} placeholders take the environment variable NAME; a missing one is
an error. Blocks between `<!-- notetaker -->` and `<!-- /notetaker -->`
lines are kept (markers dropped) when NOTETAKER=1, removed otherwise.
"""
import os
import re
import sys

s = open(sys.argv[1], encoding="utf-8").read()
keep = r"\1" if os.environ.get("NOTETAKER") == "1" else ""
s = re.sub(r"<!-- notetaker -->\n(.*?)<!-- /notetaker -->\n", keep, s, flags=re.S)


def value(m):
    v = os.environ.get(m.group(1))
    if v is None:
        sys.exit(f"render_install.py: no value for {{{{{m.group(1)}}}}}")
    return v


sys.stdout.write(re.sub(r"\{\{(\w+)\}\}", value, s))
