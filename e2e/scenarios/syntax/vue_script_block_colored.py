#!/usr/bin/env python3
# Covers #907: `--file App.vue` colors the <script> block (container grammar, whole-file highlight).
# Baseline's FileBackend skipped enhance_with_full_file_highlight, so the Vue file stayed plain (a single foreground
# colour).
import re

from e2e import Failure, Scenario, because, repos, scenario, wait_cmd


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
    # colouring is applied at render time: poll until the script line carries token colours
    # gutter (3 colours) + plain text colour = 4 on an uncoloured line; coloured tokens add several more

    def script_colours() -> int:
        scr = tui.screen(colors=True)
        line = next((ln for ln in scr.split("\n") if "const" in ln), None)
        if line is None:
            raise Failure("no line with 'const' on the screen", scr)
        return len(set(re.findall(r"38;2;[0-9;]*", line)))

    with because("Vue <script> line is not syntax coloured under --file"):
        wait_cmd(10, lambda: script_colours() >= 7)
    tui.quit()
