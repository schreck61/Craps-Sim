#!/usr/bin/env python3
"""Regenerate STRATEGY_DSL.md §7 from the examples the test suite compiles.

§7 was hand-written once and had drifted out of agreement with the grammar
within a single milestone — describing triggers that do not exist and an
operator that was never built. The examples in `examples.rs` are parsed,
compiled and simulated by the tests, so they cannot drift; generating the
section from them means the document cannot either.

    python3 tools/regenerate_dsl_examples.py

Run it from the repository root after changing `examples.rs`.
"""

import re
src = open('crates/craps-engine/src/strategy/examples.rs').read()
doc = open('docs/STRATEGY_DSL.md').read()
lines = src.split('\n')

label = dict((v, k) for k, v in re.findall(r'\("([^"]+)", (\w+)\)', src))

out = []
for i, line in enumerate(lines):
    m = re.match(r'pub const (\w+): &str = r#"$', line)
    if not m:
        continue
    const = m.group(1)
    if const not in label:
        continue
    # Doc comment: the contiguous /// lines immediately above.
    j = i - 1
    doc_lines = []
    while j >= 0 and lines[j].startswith('///'):
        doc_lines.insert(0, lines[j][3:].strip())
        j -= 1
    # Body: up to the closing "#;
    k = i + 1
    body = []
    while lines[k] != '"#;':
        body.append(lines[k])
        k += 1
    # Keep the blank line the doc comment had: the blockquote and the
    # sentence under it are two paragraphs, not one.
    prose = '\n'.join(doc_lines).strip()
    out.append("**%s**\n\n%s\n\n```\n%s\n```" % (label[const], prose, '\n'.join(body).strip()))

assert len(out) == 4, "expected four examples, found %d" % len(out)

new_section = (
    "## 7. Worked Examples\n\n"
    "Each is a strategy that could not be expressed before the language\n"
    "existed. **This section is generated** from\n"
    "`crates/craps-engine/src/strategy/examples.rs`, where every one of them\n"
    "is parsed, compiled and simulated by the test suite. It was hand-written\n"
    "once and had drifted out of agreement with the grammar within a\n"
    "milestone — naming triggers that do not exist and an `in` operator that\n"
    "was never built — so it is no longer hand-written.\n\n"
    "They ship in the app under **Examples**, as demonstrations of the\n"
    "language and not as advice: most are bad bets and one is deliberately\n"
    "superstitious.\n\n"
    + "\n\n".join(out) + "\n\n"
)
start = doc.index("## 7. Worked Examples")
end = doc.index("## 8. The Bench")
open('docs/STRATEGY_DSL.md','w').write(doc[:start] + new_section + doc[end:])
print("regenerated", len(out), "examples")
