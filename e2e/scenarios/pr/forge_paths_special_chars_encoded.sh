#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# paths with # ? % and a Unix backslash must reach the forge intact. Before, the REST
# contents URL carried them raw (`#`/`?` cut the URL, `\` was rewritten to `/`), so the file could not be
# fetched and the end-of-file/expand rows were wrong.
n=$(e2e_name); sc_pr_open $n special_char_paths
e2e_wait_for $n 'a\?b%\.cs|C#' 10
e2e_wait_forge_log 'contents/src/a%5Cb\.rs' 10; e2e_wait_forge_log 'contents/src/C%23/a%3Fb%25\.cs' 10
log=$(e2e_forge_log)
echo "$log" | grep -F 'contents/' | cut -c1-220
# '#', '?', '%' are percent-encoded in the REST contents path
e2e_forge_expect 'contents/src/C%23/a%3Fb%25\.cs\?ref=' || { echo "special-char path not encoded in contents request"; exit 1; }
# a Unix backslash is a filename byte: encoded as %5C, never turned into '/'
e2e_forge_expect 'contents/src/a%5Cb\.rs\?ref=' || { echo "backslash path mangled in contents request"; exit 1; }
sc_quit $n
