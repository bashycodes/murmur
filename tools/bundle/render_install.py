#!/usr/bin/env python3
"""Fill docs/colleague-install.md for one bundle (tools/colleague-bundle.sh).

    NOTETAKER=0|1 IMAGE=0|1 NOIMAGE=0|1 VERSION=... [other {{NAME}} values] \
        render_install.py TEMPLATE > INSTALL.md

{{NAME}} placeholders take the environment variable NAME; a missing one is
an error. A block between `<!-- name -->` and `<!-- /name -->` lines is
kept (markers dropped) when the variable NAME (upper case) is 1, removed
otherwise: `notetaker` (the speaker models), `image` (the bundle has the
Docker image), `noimage` (it has not).
"""
import os
import re
import sys

s = open(sys.argv[1], encoding="utf-8").read()


def block(m):
    return m.group(2) if os.environ.get(m.group(1).upper()) == "1" else ""


s = re.sub(r"<!-- (\w+) -->\n(.*?)<!-- /\1 -->\n", block, s, flags=re.S)


def value(m):
    v = os.environ.get(m.group(1))
    if v is None:
        sys.exit(f"render_install.py: no value for {{{{{m.group(1)}}}}}")
    return v


sys.stdout.write(re.sub(r"\{\{(\w+)\}\}", value, s))
