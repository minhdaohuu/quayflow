"""Generates assets/screenshots/hero-switch.svg: a looping terminal scene of a suggested switch.

Sample data; the wording follows the Mod's own messages and status line.
"""
from xml.sax.saxutils import escape

W, H = 820, 310
CYCLE = 14  # seconds
BG, BAR, FG, DIM, WARN, ACC, OK = '#0F1720', '#1E2A36', '#F8FAF9', '#8A99A8', '#FBBF24', '#A7F3D0', '#34D399'
X, Y0, LH = 24, 70, 24

# (text, colour, line index or 'foot', start %, end %)
items = [
    ('> Refactor the parser to stream tokens', ACC, 0, 2, 96),
    ('● Done: the parser streams tokens; 4 files changed, tests pass.', FG, 1, 8, 96),
    ('● quayflow: Quayflow suggests "personal": work at 91% 5h &gt;= 85%; personal at 12% 5h.', DIM, 2, 20, 96),
    ('> /quay accept', ACC, 3, 32, 96),
    ('  ⎿ quayflow: Switch to "personal" for environment default: requested.', DIM, 4, 38, 96),
    ('● quayflow: Quayflow: Switched to personal (suggestion accepted).', OK, 5, 45, 96),
    ('> Now add a benchmark for the streaming path', ACC, 6, 56, 96),
    ('● Added bench/stream.rs and a CI job that runs it.', FG, 7, 68, 96),
    ('Quayflow · work · Suggest · 5h 91% · 7d 40%', DIM, 'foot', 0, 18),
    ('⚠ quayflow: Quayflow · work · Suggest · 5h 91% · 7d 40% | personal suggested', WARN, 'foot', 18, 50),
    ('Quayflow · personal · Suggest · 5h 12% · 7d 8%', DIM, 'foot', 50, 96),
]
FINAL = {0, 1, 2, 3, 4, 5, 6, 7, 10}  # what a still frame shows (reduced motion)

css, body = [], []
for i, (text, colour, line, s, e) in enumerate(items):
    y = H - 22 if line == 'foot' else Y0 + line * LH
    css.append(
        f'.e{i}{{animation:k{i} {CYCLE}s linear infinite}}'
        f'@keyframes k{i}{{0%,{s}%{{opacity:0}}{s + 1}%,{e}%{{opacity:1}}{min(e + 1, 100)}%,100%{{opacity:0}}}}'
    )
    style = '' if i in FINAL else ' data-still="hide"'
    body.append(f'<text class="e{i}" x="{X}" y="{y}" fill="{colour}"{style}>{text}</text>')

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-label="Claude Code with Quayflow: the status line suggests the account personal near the limit, /quay accept switches, and the next request runs on personal in the same session">
<style>
text{{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,'Liberation Mono',monospace;font-size:14px;white-space:pre}}
{''.join(css)}
@media (prefers-reduced-motion:reduce){{text{{animation:none!important;opacity:1}}text[data-still="hide"]{{opacity:0}}}}
</style>
<rect width="{W}" height="{H}" rx="12" fill="{BG}"/>
<rect width="{W}" height="36" rx="12" fill="{BAR}"/><rect y="24" width="{W}" height="12" fill="{BAR}"/>
<circle cx="22" cy="18" r="6" fill="#f38ba8"/><circle cx="42" cy="18" r="6" fill="#f9e2af"/><circle cx="62" cy="18" r="6" fill="#a6e3a1"/>
<text x="{W // 2}" y="23" fill="{DIM}" text-anchor="middle" style="font-size:13px">claude: ~/work/parser</text>
<line x1="{X}" y1="{H - 44}" x2="{W - X}" y2="{H - 44}" stroke="{BAR}"/>
{chr(10).join(body)}
</svg>
'''
open('assets/screenshots/hero-switch.svg', 'w', encoding='utf-8', newline='\n').write(svg)
print('ok', len(svg))
