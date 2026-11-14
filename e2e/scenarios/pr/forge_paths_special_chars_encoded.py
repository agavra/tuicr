#!/usr/bin/env python3
# paths with # ? % and a Unix backslash must reach the forge intact. Before, the REST
# contents URL carried them raw (`#`/`?` cut the URL, `\` was rewritten to `/`), so the file could not be
# fetched and the end-of-file/expand rows were wrong.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open("special_char_paths")
    tui.wait_for(r"a\?b%\.cs|C#", 10)
    tui.wait_forge_log(r"contents/src/a%5Cb\.rs", 10).wait_forge_log(r"contents/src/C%23/a%3Fb%25\.cs", 10)
    for line in tui.forge_log_text().splitlines():
        if "contents/" in line:
            print(line[:220])
    # '#', '?', '%' are percent-encoded in the REST contents path
    tui.forge_expect(r"contents/src/C%23/a%3Fb%25\.cs\?ref=")  # else: special-char path not encoded in contents request
    # a Unix backslash is a filename byte: encoded as %5C, never turned into '/'
    tui.forge_expect(r"contents/src/a%5Cb\.rs\?ref=")  # else: backslash path mangled in contents request
    tui.quit()
