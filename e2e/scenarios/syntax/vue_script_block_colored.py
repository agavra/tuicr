#!/usr/bin/env python3
# Covers #907: `--file App.vue` colors the <script> block (container grammar, whole-file highlight).
# Baseline's FileBackend skipped enhance_with_full_file_highlight, so the Vue file stayed plain (a single foreground colour).
import re
import time

from e2e import Failure, Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    r = repos.empty_repo(s.dir / "repo")
    repos.write(
        r / "App.vue",
        '<template>\n  <div class="a">{{ msg }}</div>\n</template>\n<script>\nconst answer = 42;\n'
        'export default { name: "x" }\n</script>\n',
    )
    tui = s.start("--file", "App.vue", cwd=r)
    tui.wait_for("const answer")
    # colouring is applied at render time: poll until the script line carries token colours (or give up)
    colors = 0
    for _ in range(100):
        scr = tui.screen(colors=True)
        line = next((ln for ln in scr.split("\n") if "const" in ln), None)
        if line is None:
            raise Failure("no line with 'const' on the screen", scr)
        colors = len(set(re.findall(r"38;2;[0-9;]*", line)))
        if colors >= 7:
            break
        time.sleep(0.1)
    print(f"distinct fg colours on the script line: {colors}")
    # gutter (3 colours) + plain text colour = 4 on an uncoloured line; coloured tokens add several more
    assert colors >= 7, "Vue <script> line is not syntax coloured under --file"
    tui.quit()
