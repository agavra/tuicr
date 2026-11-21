#!/usr/bin/env python3
# Covers #941 (Gerrit "invalid peer certificate: UnknownIssuer"): a self-hosted Gerrit signed by a corporate CA
# in the OS trust store could not be read, because tuicr's HTTP agent only trusted the bundled Mozilla (webpki)
# roots. It now verifies through the platform verifier (rustls-platform-verifier, RootCerts::PlatformVerifier).
#
# Both halves serve the fake Gerrit over HTTPS with a throwaway CA:
#   * untrusted CA (every platform): tuicr refuses with UnknownIssuer, so trusting the OS never means verifying
#     nothing.
#   * CA trusted by the OS (Linux only): the change opens. Linux's platform verifier reads roots through
#     rustls-native-certs, which honors SSL_CERT_FILE, so the CA is trusted without touching the machine. macOS
#     verifies through Security.framework, which ignores SSL_CERT_FILE, and trusting a CA there means editing the
#     Keychain, so that half does not run on macOS (it is not a SKIP: CI treats skips as failures).
import platform

from e2e.forge import tls_files

from e2e import Failure, Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tls = tls_files(s.dir / "tls")
    repo, url = s.gerrit_serve("root", tls=tls)

    refused = s.cli_result("--no-update-check", "pr", "1", cwd=repo, env={"GERRIT_URL": url})
    if refused.rc == 0 or "UnknownIssuer" not in refused.out:
        raise Failure("an untrusted CA must be refused with UnknownIssuer", repr(refused))

    if platform.system() != "Linux":
        print(f"note: the OS-trusted-CA half runs on Linux only, not on {platform.system()}")
        return
    tui = s.start("pr", "1", cwd=repo, env={"GERRIT_URL": url, "SSL_CERT_FILE": str(tls.ca)})
    try:
        tui.wait_for("a.txt", 20)  # else: the OS-trusted CA was not used (webpki roots only)
    except Failure:
        log = s.dir / "gerrit" / "gerrit.log"
        print(log.read_text() if log.exists() else "")
        raise
    tui.settle().keys("}")
    tui.wait_for("line two")
    tui.quit()
