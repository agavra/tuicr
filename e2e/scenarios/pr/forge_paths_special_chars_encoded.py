#!/usr/bin/env python3
# paths with # ? % and a Unix backslash must reach the forge intact. Before, the REST
# contents URL carried them raw (`#`/`?` cut the URL, `\` was rewritten to `/`), so the file could not be
# fetched (and the end-of-file/expand rows built from it were wrong). This checks the contents requests, the cause;
# the rows are a downstream effect of the fetch and are not asserted here.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open("special_char_paths")
    tui.wait_for(r"a\?b%\.cs", 10).wait_for(r"a\\b\.rs", 10)
    tui.wait_forge_log(r"contents/src/a%5Cb\.rs", 10).wait_forge_log(r"contents/src/C%23/a%3Fb%25\.cs", 10)
    # '#', '?', '%' are percent-encoded in the REST contents path
    tui.forge_expect(r"contents/src/C%23/a%3Fb%25\.cs\?ref=")  # else: special-char path not encoded in contents request
    # a Unix backslash is a filename byte: encoded as %5C, never turned into '/'
    tui.forge_expect(r"contents/src/a%5Cb\.rs\?ref=")  # else: backslash path mangled in contents request
    # and no request carries one of them raw
    tui.forge_reject(r"contents/src/(C#|a/b\.rs|a\\\\b\.rs|C%23/a\?b)")
    tui.quit()
