"""stdin HTML -> plain text from the first START to (excluding) the line starting END.

    curl … | python3 -I tools/notice/html_text.py START END > out.txt
"""
import html
import re
import sys

start, end = sys.argv[1], sys.argv[2]
s = sys.stdin.read()
s = re.sub(r"<(script|style)\b.*?</\1>", "", s, flags=re.S)
s = re.sub(r"<br\s*/?>|</p>|</li>|</h\d>|</div>|</tr>", "\n", s)
s = html.unescape(re.sub(r"<[^>]+>", "", s))
s = re.sub(r"[ \t ]+", " ", s)
s = "\n".join(line.strip() for line in s.splitlines())
s = re.sub(r"\n{3,}", "\n\n", s)
a = s.find(start)
if a < 0:
    sys.exit(f"html_text.py: {start!r} not found")
b = s.find("\n" + end, a)
text = s[a:] if b < 0 else s[a:b + 1 + len(s[b + 1:].split("\n", 1)[0])]
sys.stdout.write(text.strip() + "\n")
