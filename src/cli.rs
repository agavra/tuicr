//! CLI argument parsing, backed by `clap`.
//!
//! The struct [`Cli`] is the clap-derived parser; [`CliArgs`] is the simple
//! POJO the rest of the binary consumes. Conversion lives in `From<Cli>`.

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

use crate::theme::{AppearanceArg, ThemeArg};

/// CLI arguments consumed by the rest of the binary.
#[derive(Debug, Clone, Default)]
pub struct CliArgs {
    pub theme: Option<String>,
    pub appearance: Option<AppearanceArg>,
    /// Output to stdout instead of clipboard when exporting.
    pub output_to_stdout: bool,
    /// Skip checking for updates on startup.
    pub no_update_check: bool,
    /// Commit/revision range to review.
    pub revisions: Option<String>,
    /// Skip commit selector and review uncommitted changes directly.
    pub working_tree: bool,
    /// Filter diff to a specific file or directory path.
    pub path_filter: Option<String>,
    /// Open a single file or directory for annotation (no VCS required).
    pub file_path: Option<String>,
    /// Whole-repo annotation mode.
    pub all_files: bool,
    /// Direct PR target from `tuicr pr <target>`.
    pub pr_target: Option<String>,
    /// Override the GitHub repo used for PR operations.
    pub repo_url: Option<String>,
    /// Non-interactive review session operation.
    pub review_command: Option<ReviewCommand>,
}

#[derive(Parser, Debug)]
#[command(
    name = "tuicr",
    version,
    about = "A code review TUI with vim keybindings. Export to GitHub or clipboard.",
    after_help = "Press ? in the application for keybinding help.",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(flatten)]
    tui_options: TuiOptions,

    #[command(subcommand)]
    command: Option<Subcmd>,
}

/// Options that launch or configure the interactive TUI.
#[derive(Args, Debug, Clone, Default)]
struct TuiOptions {
    /// Commit range / revset to review (syntax depends on VCS backend).
    #[arg(
        short = 'r',
        long = "revisions",
        value_name = "REVSET",
        allow_hyphen_values = true
    )]
    revisions: Option<String>,

    /// Color theme to use. Bundled themes resolve first; local themes are
    /// loaded from the config `themes/` directory.
    #[arg(long, value_name = "THEME", value_parser = non_empty_theme_name)]
    theme: Option<String>,

    /// Appearance mode (light/dark/system); used when no explicit theme is set.
    #[arg(long, value_name = "MODE", value_parser = parse_appearance_arg)]
    appearance: Option<AppearanceArg>,

    /// Filter diff to a specific file or directory.
    #[arg(
        short = 'p',
        long = "path",
        value_name = "PATH",
        value_parser = non_empty_path,
        conflicts_with_all = ["file_path", "all_files"],
    )]
    path_filter: Option<String>,

    /// Include uncommitted changes (skip commit selector when used alone;
    /// combine with commits when used with -r).
    #[arg(
        short = 'w',
        long = "working-tree",
        action = ArgAction::SetTrue,
        conflicts_with_all = ["file_path", "all_files"],
    )]
    working_tree: bool,

    /// Open a file or directory for annotation (no VCS required).
    #[arg(
        long = "file",
        value_name = "PATH",
        value_parser = non_empty_path,
        conflicts_with_all = ["path_filter", "revisions", "working_tree", "all_files"],
    )]
    file_path: Option<String>,

    /// Review every tracked file in the cwd's git repo.
    #[arg(
        short = 'A',
        long = "all-files",
        action = ArgAction::SetTrue,
        conflicts_with_all = ["path_filter", "revisions", "working_tree", "file_path"],
    )]
    all_files: bool,

    /// Output to stdout instead of clipboard when exporting.
    #[arg(long = "stdout", action = ArgAction::SetTrue)]
    stdout: bool,

    /// Skip checking for updates on startup.
    #[arg(long = "no-update-check", action = ArgAction::SetTrue)]
    no_update_check: bool,

    /// Override the GitHub repo for PR operations (HTTPS, SCP-style SSH,
    /// or ssh:// URLs accepted).
    #[arg(
        long = "repo-url",
        value_name = "URL",
        value_parser = parse_repo_url
    )]
    repo_url: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Subcmd {
    /// Open the interactive TUI.
    Tui(TuiCommand),
    /// Review a GitHub pull request or GitLab merge request.
    #[command(visible_alias = "mr")]
    Pr(PrCommand),
    /// Inspect or update persisted review sessions.
    Review {
        #[command(subcommand)]
        command: ReviewCommand,
    },
}

/// Explicit `tuicr tui` entrypoint. With no nested command, opens the local
/// target selector / local diff TUI. `tuicr tui pr <target>` opens PR mode.
#[derive(Args, Debug, Clone, Default)]
struct TuiCommand {
    #[command(flatten)]
    options: TuiOptions,

    #[command(subcommand)]
    command: Option<TuiSubcmd>,
}

#[derive(Subcommand, Debug, Clone)]
enum TuiSubcmd {
    /// Review a GitHub pull request or GitLab merge request in the TUI.
    #[command(visible_alias = "mr")]
    Pr(PrCommand),
}

#[derive(Args, Debug, Clone, Default)]
struct PrCommand {
    /// PR target: <number>, <owner/repo#N>, or a PR URL.
    target: String,

    #[command(flatten)]
    options: TuiOptions,
}

/// Non-interactive review session commands.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ReviewCommand {
    /// List persisted review sessions for a checkout or forge repo.
    List {
        /// Repo selector: a checkout path, or a forge coordinate like
        /// `owner/repo`, `host/owner/repo`, or a repo/PR URL. A path also
        /// surfaces PR sessions for that checkout's origin repo.
        #[arg(long, value_name = "PATH|OWNER/REPO", default_value = ".")]
        repo: PathBuf,

        /// List every persisted session (local and PR), ignoring --repo.
        #[arg(long)]
        all: bool,
    },

    /// Add a local draft comment to a persisted session.
    Add {
        /// Session slug from `tuicr review list` (local or PR), or path to a
        /// session JSON file.
        #[arg(long, value_name = "SESSION")]
        session: String,

        /// JSON payload. Use literal JSON, @path/to/file.json, or - for stdin.
        #[arg(long, value_name = "JSON|@FILE|-")]
        input: Option<String>,

        /// Repo selector used to resolve a local session slug (path or
        /// `owner/repo`). PR slugs and JSON paths resolve without it.
        #[arg(long, value_name = "PATH|OWNER/REPO", default_value = ".")]
        repo: PathBuf,

        /// Comment classification.
        #[arg(long = "type", value_name = "TYPE", default_value = "note", value_parser = non_empty_comment_type)]
        comment_type: String,

        /// File path for a file, line, or range comment. Omit for a review comment.
        #[arg(long = "target-file", value_name = "PATH")]
        file: Option<PathBuf>,

        /// Line number for a line or range comment. Requires --target-file.
        #[arg(long, value_name = "LINE", requires = "file")]
        line: Option<u32>,

        /// End line for a range comment. Requires --line.
        #[arg(long = "end-line", value_name = "LINE", requires = "line")]
        end_line: Option<u32>,

        /// Diff side for line and range comments.
        #[arg(long, value_enum, default_value_t = LineSideArg::New)]
        side: LineSideArg,

        /// Author stamped on the new comment. Pass an explicit value when
        /// invoking from an agent (e.g. `--username "Claude Opus 4.7"`) so
        /// human and agent comments are visually distinguished in the TUI.
        /// Falls back to the config `username` setting, then to `"user"`.
        #[arg(long, value_name = "NAME")]
        username: Option<String>,

        /// Comment text.
        #[arg(
            value_name = "COMMENT",
            required_unless_present = "input",
            value_parser = non_empty_comment_text,
            allow_hyphen_values = true
        )]
        content: Option<String>,
    },

    /// Print comments stored in a persisted session.
    #[command(alias = "get")]
    Comments {
        /// Session slug from `tuicr review list` (local or PR), or path to a
        /// session JSON file.
        #[arg(long, value_name = "SESSION")]
        session: String,

        /// Repo selector used to resolve a local session slug (path or
        /// `owner/repo`). PR slugs and JSON paths resolve without it.
        #[arg(long, value_name = "PATH|OWNER/REPO", default_value = ".")]
        repo: PathBuf,
    },
}

/// Diff side accepted by `tuicr review add --side`.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineSideArg {
    Old,
    #[default]
    New,
}

impl From<Cli> for CliArgs {
    fn from(cli: Cli) -> Self {
        let (options, pr_target, review_command) = match cli.command {
            Some(Subcmd::Tui(command)) => match command.command {
                Some(TuiSubcmd::Pr(pr)) => (
                    cli.tui_options.merge(command.options).merge(pr.options),
                    Some(pr.target),
                    None,
                ),
                None => (cli.tui_options.merge(command.options), None, None),
            },
            Some(Subcmd::Pr(pr)) => (cli.tui_options.merge(pr.options), Some(pr.target), None),
            Some(Subcmd::Review { command }) => (TuiOptions::default(), None, Some(command)),
            None => (cli.tui_options, None, None),
        };
        Self {
            theme: options.theme,
            appearance: options.appearance,
            output_to_stdout: options.stdout,
            no_update_check: options.no_update_check,
            revisions: options.revisions,
            working_tree: options.working_tree,
            path_filter: options.path_filter,
            file_path: options.file_path,
            all_files: options.all_files,
            pr_target,
            repo_url: options.repo_url,
            review_command,
        }
    }
}

impl TuiOptions {
    fn has_any_explicit_value(&self) -> bool {
        self.theme.is_some()
            || self.appearance.is_some()
            || self.stdout
            || self.no_update_check
            || self.revisions.is_some()
            || self.working_tree
            || self.path_filter.is_some()
            || self.file_path.is_some()
            || self.all_files
            || self.repo_url.is_some()
    }

    fn merge(self, later: TuiOptions) -> Self {
        Self {
            theme: later.theme.or(self.theme),
            appearance: later.appearance.or(self.appearance),
            stdout: self.stdout || later.stdout,
            no_update_check: self.no_update_check || later.no_update_check,
            revisions: later.revisions.or(self.revisions),
            working_tree: self.working_tree || later.working_tree,
            path_filter: later.path_filter.or(self.path_filter),
            file_path: later.file_path.or(self.file_path),
            all_files: self.all_files || later.all_files,
            repo_url: later.repo_url.or(self.repo_url),
        }
    }
}

impl Cli {
    fn try_into_args(self) -> std::result::Result<CliArgs, clap::Error> {
        if matches!(self.command, Some(Subcmd::Review { .. }))
            && self.tui_options.has_any_explicit_value()
        {
            return Err(clap::Error::raw(
                clap::error::ErrorKind::ArgumentConflict,
                "TUI options cannot be used with `tuicr review`; run `tuicr review <command> --help` for review CLI options",
            ));
        }
        Ok(self.into())
    }
}

fn parse_appearance_arg(s: &str) -> Result<AppearanceArg, String> {
    AppearanceArg::parse_name(s).ok_or_else(|| {
        let valid = AppearanceArg::valid_values_display();
        format!("Unknown appearance '{s}'. Valid options: {valid}")
    })
}

fn non_empty_theme_name(s: &str) -> Result<String, String> {
    if s.is_empty() {
        let valid = ThemeArg::valid_values_display();
        Err(format!("--theme requires a value ({valid})"))
    } else {
        Ok(s.to_string())
    }
}

/// Reject `--repo-url` values that don't parse as a GitHub remote URL so the
/// failure is surfaced at startup rather than when the PR tab is opened.
fn parse_repo_url(s: &str) -> Result<String, String> {
    if crate::forge::github::gh::parse_github_remote_url(s).is_some() {
        Ok(s.to_string())
    } else {
        Err(format!(
            "--repo-url value '{s}' is not a recognized GitHub URL. \
             Expected forms: https://github.com/owner/repo, git@github.com:owner/repo, \
             or ssh://git@github.com/owner/repo"
        ))
    }
}

fn non_empty_path(s: &str) -> Result<String, String> {
    if s.is_empty() {
        Err("a file or directory path is required".to_string())
    } else {
        Ok(s.to_string())
    }
}

fn non_empty_comment_type(s: &str) -> Result<String, String> {
    if s.is_empty() {
        Err("a comment type is required".to_string())
    } else {
        Ok(s.to_string())
    }
}

fn non_empty_comment_text(s: &str) -> Result<String, String> {
    if s.trim().is_empty() {
        Err("comment text cannot be empty".to_string())
    } else {
        Ok(s.to_string())
    }
}

/// Parse CLI arguments from `std::env::args`. On `--help`/`--version`/parse
/// errors, clap prints to stdout/stderr and exits the process.
pub fn parse_cli_args() -> CliArgs {
    match Cli::parse().try_into_args() {
        Ok(args) => args,
        Err(err) => err.exit(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn parse_for_test(args: &[&str]) -> Result<CliArgs, clap::Error> {
        Cli::try_parse_from(args).and_then(Cli::try_into_args)
    }

    #[test]
    fn should_parse_theme_when_provided() {
        let parsed = parse_for_test(&["tuicr", "--theme", "light"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some("light".to_string()));
    }

    #[test]
    fn should_parse_catppuccin_themes() {
        let parsed = parse_for_test(&["tuicr", "--theme", "catppuccin-mocha"])
            .expect("parse should succeed");
        assert_eq!(parsed.theme, Some("catppuccin-mocha".to_string()));

        let parsed =
            parse_for_test(&["tuicr", "--theme=catppuccin-latte"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some("catppuccin-latte".to_string()));
    }

    #[test]
    fn should_parse_ayu_light_theme() {
        let parsed =
            parse_for_test(&["tuicr", "--theme", "ayu-light"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some("ayu-light".to_string()));
    }

    #[test]
    fn should_parse_onedark_theme() {
        let parsed =
            parse_for_test(&["tuicr", "--theme", "onedark"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some("onedark".to_string()));
    }

    #[test]
    fn should_parse_gruvbox_themes() {
        let parsed =
            parse_for_test(&["tuicr", "--theme", "gruvbox-dark"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some("gruvbox-dark".to_string()));

        let parsed =
            parse_for_test(&["tuicr", "--theme=gruvbox-light"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some("gruvbox-light".to_string()));
    }

    #[test]
    fn should_parse_everforest_themes() {
        let parsed =
            parse_for_test(&["tuicr", "--theme", "everforest-dark"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some("everforest-dark".to_string()));

        let parsed =
            parse_for_test(&["tuicr", "--theme=everforest-light"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some("everforest-light".to_string()));
    }

    #[test]
    fn should_leave_theme_none_when_not_provided() {
        let parsed = parse_for_test(&["tuicr"]).expect("parse should succeed");
        assert_eq!(parsed.theme, None);
    }

    #[test]
    fn should_parse_working_tree_short_flag() {
        let parsed = parse_for_test(&["tuicr", "-w"]).expect("parse should succeed");
        assert!(parsed.working_tree);
    }

    #[test]
    fn should_parse_working_tree_long_flag() {
        let parsed = parse_for_test(&["tuicr", "--working-tree"]).expect("parse should succeed");
        assert!(parsed.working_tree);
    }

    #[test]
    fn should_default_working_tree_to_false() {
        let parsed = parse_for_test(&["tuicr"]).expect("parse should succeed");
        assert!(!parsed.working_tree);
    }

    #[test]
    fn should_parse_working_tree_with_revisions() {
        let parsed =
            parse_for_test(&["tuicr", "-w", "-r", "HEAD~3..HEAD"]).expect("parse should succeed");
        assert!(parsed.working_tree);
        assert_eq!(parsed.revisions, Some("HEAD~3..HEAD".to_string()));
    }

    #[test]
    fn should_allow_custom_theme_name_in_separate_arg() {
        let parsed = parse_for_test(&["tuicr", "--theme", "tuicr-teal"])
            .expect("custom theme parse should succeed");
        assert_eq!(parsed.theme, Some("tuicr-teal".to_string()));
    }

    #[test]
    fn should_allow_custom_theme_name_in_equals_arg() {
        let parsed = parse_for_test(&["tuicr", "--theme=tuicr-teal"])
            .expect("custom theme parse should succeed");
        assert_eq!(parsed.theme, Some("tuicr-teal".to_string()));
    }

    #[test]
    fn should_error_when_theme_value_missing() {
        let err = parse_for_test(&["tuicr", "--theme"]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn should_parse_appearance_when_provided() {
        let parsed =
            parse_for_test(&["tuicr", "--appearance", "system"]).expect("parse should succeed");
        assert_eq!(parsed.appearance, Some(AppearanceArg::System));
    }

    #[test]
    fn should_error_for_invalid_appearance() {
        let err =
            parse_for_test(&["tuicr", "--appearance", "nope"]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
        assert!(err.to_string().contains("Unknown appearance 'nope'"));
    }

    #[test]
    fn should_parse_path_short_flag() {
        let parsed = parse_for_test(&["tuicr", "-p", "src/main.rs"]).expect("parse should succeed");
        assert_eq!(parsed.path_filter, Some("src/main.rs".to_string()));
    }

    #[test]
    fn should_parse_path_long_flag() {
        let parsed = parse_for_test(&["tuicr", "--path", "src/"]).expect("parse should succeed");
        assert_eq!(parsed.path_filter, Some("src/".to_string()));
    }

    #[test]
    fn should_parse_path_equals_syntax() {
        let parsed = parse_for_test(&["tuicr", "--path=plans/current-plan.md"])
            .expect("parse should succeed");
        assert_eq!(
            parsed.path_filter,
            Some("plans/current-plan.md".to_string())
        );
    }

    #[test]
    fn should_error_when_path_value_missing() {
        let err = parse_for_test(&["tuicr", "--path"]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn should_error_when_path_equals_empty() {
        let err = parse_for_test(&["tuicr", "--path="]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn should_default_path_filter_to_none() {
        let parsed = parse_for_test(&["tuicr"]).expect("parse should succeed");
        assert_eq!(parsed.path_filter, None);
    }

    #[test]
    fn should_parse_path_with_working_tree() {
        let parsed =
            parse_for_test(&["tuicr", "-p", "file.md", "-w"]).expect("parse should succeed");
        assert_eq!(parsed.path_filter, Some("file.md".to_string()));
        assert!(parsed.working_tree);
    }

    #[test]
    fn should_parse_path_with_revisions() {
        let parsed = parse_for_test(&["tuicr", "--path", "src/", "-r", "HEAD~3.."])
            .expect("parse should succeed");
        assert_eq!(parsed.path_filter, Some("src/".to_string()));
        assert_eq!(parsed.revisions, Some("HEAD~3..".to_string()));
    }

    #[test]
    fn should_reject_file_combined_with_path() {
        let err = parse_for_test(&["tuicr", "--file", "f.md", "--path", "src/"])
            .expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn should_reject_file_combined_with_revisions() {
        let err = parse_for_test(&["tuicr", "--file", "f.md", "-r", "HEAD~1.."])
            .expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn should_reject_file_combined_with_working_tree() {
        let err =
            parse_for_test(&["tuicr", "--file", "f.md", "-w"]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn should_reject_all_files_combined_with_path() {
        let err =
            parse_for_test(&["tuicr", "-A", "--path", "src/"]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn should_reject_all_files_combined_with_file() {
        let err =
            parse_for_test(&["tuicr", "-A", "--file", "f.md"]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn should_parse_all_files_short_flag() {
        let parsed = parse_for_test(&["tuicr", "-A"]).expect("parse should succeed");
        assert!(parsed.all_files);
    }

    #[test]
    fn should_parse_all_files_long_flag() {
        let parsed = parse_for_test(&["tuicr", "--all-files"]).expect("parse should succeed");
        assert!(parsed.all_files);
    }

    #[test]
    fn should_parse_stdout_flag() {
        let parsed = parse_for_test(&["tuicr", "--stdout"]).expect("parse should succeed");
        assert!(parsed.output_to_stdout);
    }

    #[test]
    fn should_parse_no_update_check_flag() {
        let parsed = parse_for_test(&["tuicr", "--no-update-check"]).expect("parse should succeed");
        assert!(parsed.no_update_check);
    }

    #[test]
    fn should_parse_pr_target_as_bare_number() {
        let parsed = parse_for_test(&["tuicr", "pr", "125"]).expect("parse should succeed");
        assert_eq!(parsed.pr_target, Some("125".to_string()));
    }

    #[test]
    fn should_parse_mr_alias_like_pr() {
        let parsed = parse_for_test(&["tuicr", "mr", "125"]).expect("parse should succeed");
        assert_eq!(parsed.pr_target, Some("125".to_string()));
    }

    #[test]
    fn should_parse_tui_mr_alias_like_pr() {
        let parsed = parse_for_test(&["tuicr", "tui", "mr", "125"]).expect("parse should succeed");
        assert_eq!(parsed.pr_target, Some("125".to_string()));
    }

    #[test]
    fn should_parse_pr_target_as_owner_repo_hash() {
        let parsed =
            parse_for_test(&["tuicr", "pr", "agavra/tuicr#125"]).expect("parse should succeed");
        assert_eq!(parsed.pr_target, Some("agavra/tuicr#125".to_string()));
    }

    #[test]
    fn should_parse_pr_target_as_full_url() {
        let parsed = parse_for_test(&["tuicr", "pr", "https://github.com/agavra/tuicr/pull/125"])
            .expect("parse should succeed");
        assert_eq!(
            parsed.pr_target,
            Some("https://github.com/agavra/tuicr/pull/125".to_string()),
        );
    }

    #[test]
    fn should_error_when_pr_target_is_missing() {
        let err = parse_for_test(&["tuicr", "pr"]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn should_combine_pr_target_with_theme_flag() {
        // Legacy `tuicr pr` still accepts TUI flags on the subcommand.
        let parsed = parse_for_test(&["tuicr", "pr", "125", "--theme", "dark"])
            .expect("parse should succeed");
        assert_eq!(parsed.pr_target, Some("125".to_string()));
        assert_eq!(parsed.theme, Some("dark".to_string()));
    }

    #[test]
    fn should_allow_root_tui_options_before_legacy_pr_subcommand() {
        let parsed = parse_for_test(&["tuicr", "--theme", "dark", "pr", "125"])
            .expect("parse should succeed");
        assert_eq!(parsed.pr_target, Some("125".to_string()));
        assert_eq!(parsed.theme, Some("dark".to_string()));
    }

    #[test]
    fn should_parse_explicit_tui_command() {
        let parsed = parse_for_test(&["tuicr", "tui", "-w", "--theme", "dark"])
            .expect("parse should succeed");
        assert!(parsed.working_tree);
        assert_eq!(parsed.theme, Some("dark".to_string()));
        assert_eq!(parsed.pr_target, None);
        assert_eq!(parsed.review_command, None);
    }

    #[test]
    fn should_parse_explicit_tui_pr_command() {
        let parsed = parse_for_test(&["tuicr", "tui", "pr", "125", "--theme", "dark"])
            .expect("parse should succeed");
        assert_eq!(parsed.pr_target, Some("125".to_string()));
        assert_eq!(parsed.theme, Some("dark".to_string()));
    }

    #[test]
    fn should_reject_root_tui_options_before_subcommands() {
        let err = parse_for_test(&["tuicr", "--theme", "dark", "review", "list"])
            .expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn should_leave_pr_target_none_when_no_pr_subcommand() {
        let parsed = parse_for_test(&["tuicr"]).expect("parse should succeed");
        assert_eq!(parsed.pr_target, None);
    }

    #[test]
    fn should_parse_repo_url_https() {
        let parsed = parse_for_test(&["tuicr", "--repo-url", "https://github.com/slatedb/slatedb"])
            .expect("parse should succeed");
        assert_eq!(
            parsed.repo_url,
            Some("https://github.com/slatedb/slatedb".to_string())
        );
    }

    #[test]
    fn should_parse_repo_url_equals_form() {
        let parsed = parse_for_test(&["tuicr", "--repo-url=git@github.com:slatedb/slatedb.git"])
            .expect("parse should succeed");
        assert_eq!(
            parsed.repo_url,
            Some("git@github.com:slatedb/slatedb.git".to_string())
        );
    }

    #[test]
    fn should_parse_repo_url_ssh_scheme() {
        let parsed = parse_for_test(&[
            "tuicr",
            "--repo-url",
            "ssh://git@github.com/slatedb/slatedb.git",
        ])
        .expect("parse should succeed");
        assert_eq!(
            parsed.repo_url,
            Some("ssh://git@github.com/slatedb/slatedb.git".to_string())
        );
    }

    #[test]
    fn should_error_when_repo_url_value_missing() {
        let err = parse_for_test(&["tuicr", "--repo-url"]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn should_error_when_repo_url_unparseable() {
        let err =
            parse_for_test(&["tuicr", "--repo-url", "not-a-url"]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
        assert!(err.to_string().contains("not a recognized GitHub URL"));
    }

    #[test]
    fn should_error_when_repo_url_equals_empty() {
        let err = parse_for_test(&["tuicr", "--repo-url="]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn should_leave_repo_url_none_when_not_provided() {
        let parsed = parse_for_test(&["tuicr"]).expect("parse should succeed");
        assert_eq!(parsed.repo_url, None);
    }

    #[test]
    fn should_parse_review_list_command() {
        let parsed = parse_for_test(&["tuicr", "review", "list", "--repo", "/tmp/repo"])
            .expect("parse should succeed");
        assert_eq!(
            parsed.review_command,
            Some(ReviewCommand::List {
                repo: PathBuf::from("/tmp/repo"),
                all: false,
            })
        );
    }

    #[test]
    fn should_parse_review_list_all_flag() {
        let parsed =
            parse_for_test(&["tuicr", "review", "list", "--all"]).expect("parse should succeed");
        assert_eq!(
            parsed.review_command,
            Some(ReviewCommand::List {
                repo: PathBuf::from("."),
                all: true,
            })
        );
    }

    #[test]
    fn should_parse_review_list_by_coordinate() {
        let parsed = parse_for_test(&["tuicr", "review", "list", "--repo", "slatedb/slatedb"])
            .expect("parse should succeed");
        assert_eq!(
            parsed.review_command,
            Some(ReviewCommand::List {
                repo: PathBuf::from("slatedb/slatedb"),
                all: false,
            })
        );
    }

    #[test]
    fn should_reject_review_json_flag_because_output_is_always_json() {
        let err =
            parse_for_test(&["tuicr", "review", "list", "--json"]).expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn should_parse_review_add_line_comment() {
        let parsed = parse_for_test(&[
            "tuicr",
            "review",
            "add",
            "--session",
            "agavra/tuicr@main/worktree",
            "--target-file",
            "src/main.rs",
            "--line",
            "42",
            "--type",
            "issue",
            "--side",
            "old",
            "Handle the empty case",
        ])
        .expect("parse should succeed");

        assert_eq!(
            parsed.review_command,
            Some(ReviewCommand::Add {
                session: "agavra/tuicr@main/worktree".to_string(),
                input: None,
                repo: PathBuf::from("."),
                comment_type: "issue".to_string(),
                file: Some(PathBuf::from("src/main.rs")),
                line: Some(42),
                end_line: None,
                side: LineSideArg::Old,
                username: None,
                content: Some("Handle the empty case".to_string()),
            })
        );
    }

    #[test]
    fn should_parse_review_add_json_input() {
        let parsed = parse_for_test(&[
            "tuicr",
            "review",
            "add",
            "--session",
            "agavra/tuicr@main/worktree",
            "--input",
            r#"{"file":"src/main.rs","line":42,"side":"old","content":"note"}"#,
        ])
        .expect("parse should succeed");

        assert_eq!(
            parsed.review_command,
            Some(ReviewCommand::Add {
                session: "agavra/tuicr@main/worktree".to_string(),
                input: Some(
                    r#"{"file":"src/main.rs","line":42,"side":"old","content":"note"}"#.to_string()
                ),
                repo: PathBuf::from("."),
                comment_type: "note".to_string(),
                file: None,
                line: None,
                end_line: None,
                side: LineSideArg::New,
                username: None,
                content: None,
            })
        );
    }

    #[test]
    fn should_parse_review_comments_command() {
        let parsed = parse_for_test(&["tuicr", "review", "comments", "--session", "session.json"])
            .expect("parse should succeed");
        assert_eq!(
            parsed.review_command,
            Some(ReviewCommand::Comments {
                session: "session.json".to_string(),
                repo: PathBuf::from("."),
            })
        );
    }

    #[test]
    fn should_parse_review_comments_get_alias() {
        let parsed = parse_for_test(&["tuicr", "review", "get", "--session", "session.json"])
            .expect("parse should succeed");
        assert_eq!(
            parsed.review_command,
            Some(ReviewCommand::Comments {
                session: "session.json".to_string(),
                repo: PathBuf::from("."),
            })
        );
    }

    #[test]
    fn should_require_file_for_review_add_line() {
        let err = parse_for_test(&[
            "tuicr",
            "review",
            "add",
            "--session",
            "session",
            "--line",
            "42",
            "note",
        ])
        .expect_err("parse should fail");
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }
}
