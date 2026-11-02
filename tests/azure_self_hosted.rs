use std::process::Command;
use tuicr::forge::traits::ForgeRepository;

#[test]
fn self_hosted_azure_routes_through_the_shared_forge_parser() {
    const CHILD: &str = "TUICR_TEST_AZURE_ROUTING_CHILD";
    if let Some(configured) = std::env::var_os(CHILD) {
        use tuicr::forge::parse_any_remote_url;
        let url = "https://tfs.example.test/tfs/Collection/Project/_git/repo";
        let expected = ForgeRepository::azure("tfs.example.test", "tfs/Collection/Project", "repo");
        let parsed = parse_any_remote_url(url).unwrap();
        if configured == "yes" {
            assert_eq!(parsed, expected);
        } else {
            assert_ne!(
                parsed.kind,
                ForgeRepository::azure("dev.azure.com", "o/p", "r").kind
            );
        }
        assert_eq!(
            parse_any_remote_url("https://dev.azure.com/org/project/_git/repo"),
            Some(ForgeRepository::azure(
                "dev.azure.com",
                "org/project",
                "repo"
            ))
        );
        assert_eq!(
            parse_any_remote_url("https://github.com/owner/repo"),
            Some(ForgeRepository::github("github.com", "owner", "repo"))
        );
        return;
    }
    for configured in ["yes", "no", "other"] {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "self_hosted_azure_routes_through_the_shared_forge_parser",
                "--nocapture",
            ])
            .env(CHILD, configured)
            .env_remove("AZURE_DEVOPS_SERVER_URL");
        if configured == "yes" {
            command.env(
                "AZURE_DEVOPS_SERVER_URL",
                "https://tfs.example.test/tfs/Collection",
            );
        }
        if configured == "other" {
            command.env("AZURE_DEVOPS_SERVER_URL", "https://different.example.test");
        }
        let out = command.output().unwrap();
        assert!(
            out.status.success(),
            "{configured}: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
