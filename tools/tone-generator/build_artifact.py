"""Builds the tone generator as one HTML page, for publishing as a Claude artifact.

An artifact runs inside claude.ai, so a viewer uses Claude on their own account.
It takes one file, so this inlines the same sources the docs page loads.

Usage: python tools/tone-generator/build_artifact.py <output.html>
"""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCES = ROOT / "docs/assets/generator"
STYLES = ROOT / "docs/stylesheets/generator.css"

PAGE = """<title>Amp GPUI Tone Generator</title>
<meta name="claude:viewport-fit-sides" content="cover">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Instrument+Sans:wght@400;600&family=Fraunces:opsz,wght@9..144,500&display=swap">
<style>
/* A forest floor at night, like the app: one green light and nothing else. */
:root {
  --bg: #0a120e;
  --fg: #dfe9e3;
  --muted: #9fb3a8;
  --line: #2b3d33;
  --accent: #6fce9c;
  --on-accent: #0a120e;
  --error: #f0907f;
  --display: "Fraunces", Georgia, serif;
  --body: "Instrument Sans", system-ui, sans-serif;
  color-scheme: dark;
}
@media (prefers-color-scheme: light) {
  :root:not([data-theme="dark"]) {
    --bg: #f2f7f4;
    --fg: #13241b;
    --muted: #4c6357;
    --line: #c3d3ca;
    --accent: #1f5c3d;
    --on-accent: #f2f7f4;
    --error: #a8352a;
    color-scheme: light;
  }
}
:root[data-theme="light"] {
  --bg: #f2f7f4;
  --fg: #13241b;
  --muted: #4c6357;
  --line: #c3d3ca;
  --accent: #1f5c3d;
  --on-accent: #f2f7f4;
  --error: #a8352a;
  color-scheme: light;
}
body {
  background: var(--bg);
  color: var(--fg);
  font-family: var(--body);
  font-size: 16px;
  line-height: 1.55;
}
main {
  margin-inline: auto;
  max-width: 42rem;
  padding-block: 2.5rem;
  padding-left: max(16px, env(safe-area-inset-left, 0px));
  padding-right: max(16px, env(safe-area-inset-right, 0px));
}
h1, h3 {
  font-family: var(--display);
  font-weight: 500;
  text-wrap: balance;
}
h1 {
  font-size: 2.2rem;
  line-height: 1.15;
  margin: 0 0 0.4em;
}
main > p {
  color: var(--muted);
  margin: 0;
  max-width: 34rem;
}
@@STYLES@@
#tone-generator {
  --gen-text: var(--fg);
  --gen-muted: var(--muted);
  --gen-line: var(--line);
  --gen-accent: var(--accent);
  --gen-accent-text: var(--on-accent);
  --gen-error: var(--error);
}
@media (prefers-reduced-motion: no-preference) {
  .gen-button { transition: border-color 120ms; }
}
</style>
<main>
  <h1>Describe a tone, get a full rig</h1>
  <p>Claude builds a complete signal chain for Amp GPUI from your description. Download the tone pack, then import it into the app. This uses your own Claude account.</p>
  <div id="tone-generator"><p>Loading...</p></div>
</main>
<script>
@@SCRIPTS@@
</script>
"""


def main():
    scripts = "\n".join(
        (SOURCES / name).read_text(encoding="utf-8")
        for name in ("catalog.js", "core.js", "ui.js")
    )
    if "</script" in scripts:
        sys.exit("A script contains a closing tag, which would end the page's script early.")
    page = PAGE.replace("@@STYLES@@", STYLES.read_text(encoding="utf-8"))
    page = page.replace("@@SCRIPTS@@", scripts)
    Path(sys.argv[1]).write_text(page, encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
