use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, bail};

use crate::model::LineSide;
use crate::review_api::{
    AddCommentRequest, CommentScope, FileDiffRequest, OpenReviewRequest, ReviewDiffSource,
    ReviewService, SessionIdRequest, SetReviewedRequest,
};

pub fn run_from_args(args: &[String]) -> anyhow::Result<()> {
    let command = args
        .get(2)
        .map(String::as_str)
        .ok_or_else(|| anyhow::anyhow!(help()))?;
    let cwd = std::env::current_dir()?;
    let service = ReviewService::new(cwd);

    match command {
        "open" => review_open(&service, &args[3..]),
        "diff" => review_diff(&service, &args[3..]),
        "comment" => review_comment(&service, &args[3..]),
        "file" => review_file(&service, &args[3..]),
        "export" => review_export(&service, &args[3..]),
        "clear" => review_clear(&service, &args[3..]),
        "-h" | "--help" | "help" => {
            println!("{}", help());
            Ok(())
        }
        unknown => bail!("Unknown review command '{unknown}'.\n\n{}", help()),
    }
}

fn review_open(service: &ReviewService, args: &[String]) -> anyhow::Result<()> {
    let flags = Flags::parse(args)?;
    let session = service.open_review(OpenReviewRequest {
        repo_path: flags.get("repo").map(PathBuf::from),
        diff_source: flags
            .get("diff-source")
            .map(parse_diff_source)
            .transpose()?,
        revisions: flags.get("revisions").map(str::to_string),
        include_working_tree: flags
            .get("include-working-tree")
            .map(parse_bool)
            .transpose()?,
        path: flags.get("path").map(str::to_string),
        file: flags.get("file").map(str::to_string),
    })?;

    if flags.has("json") {
        print_json(&session)?;
    } else {
        println!("{}", session.id);
    }
    Ok(())
}

fn review_diff(service: &ReviewService, args: &[String]) -> anyhow::Result<()> {
    let flags = Flags::parse(args)?;
    let payload = service.get_file_diff(FileDiffRequest {
        session_id: flags.required("session")?.to_string(),
        path: PathBuf::from(flags.required("path")?),
        max_lines: flags.get("max-lines").map(parse_usize).transpose()?,
    })?;

    if flags.has("json") {
        print_json(&payload)?;
    } else {
        println!("{}", payload.diff);
    }
    Ok(())
}

fn review_comment(service: &ReviewService, args: &[String]) -> anyhow::Result<()> {
    if args.first().map(String::as_str) != Some("add") {
        bail!(
            "Usage: tuicr review comment add --session <id> --body <text> [--path <path>] [--line <n>]"
        );
    }
    let flags = Flags::parse(&args[1..])?;
    let path = flags.get("path").map(PathBuf::from);
    let scope = if flags.has("scope") {
        parse_scope(flags.required("scope")?)?
    } else if flags.has("line") {
        CommentScope::Line
    } else if path.is_some() {
        CommentScope::File
    } else {
        CommentScope::Review
    };
    let session = service.add_comment(AddCommentRequest {
        session_id: flags.required("session")?.to_string(),
        scope,
        path,
        line: flags.get("line").map(parse_u32).transpose()?,
        end_line: flags.get("end-line").map(parse_u32).transpose()?,
        side: flags.get("side").map(parse_side).transpose()?,
        comment_type: flags.get("type").map(str::to_string),
        body: flags.required("body")?.to_string(),
    })?;
    print_json(&session)
}

fn review_file(service: &ReviewService, args: &[String]) -> anyhow::Result<()> {
    if args.first().map(String::as_str) != Some("reviewed") {
        bail!("Usage: tuicr review file reviewed --session <id> --path <path> --set <true|false>");
    }
    let flags = Flags::parse(&args[1..])?;
    let session = service.set_file_reviewed(SetReviewedRequest {
        session_id: flags.required("session")?.to_string(),
        path: PathBuf::from(flags.required("path")?),
        reviewed: parse_bool(flags.required("set")?)?,
    })?;
    print_json(&session)
}

fn review_export(service: &ReviewService, args: &[String]) -> anyhow::Result<()> {
    let flags = Flags::parse(args)?;
    let markdown = service.export_review(SessionIdRequest {
        session_id: flags.required("session")?.to_string(),
    })?;
    println!("{markdown}");
    Ok(())
}

fn review_clear(service: &ReviewService, args: &[String]) -> anyhow::Result<()> {
    let flags = Flags::parse(args)?;
    let session = service.clear_review(SessionIdRequest {
        session_id: flags.required("session")?.to_string(),
    })?;
    print_json(&session)
}

#[derive(Debug, Default)]
struct Flags {
    values: HashMap<String, String>,
    switches: Vec<String>,
}

impl Flags {
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut flags = Flags::default();
        let mut iter = args.iter().peekable();
        while let Some(arg) = iter.next() {
            let Some(raw_key) = arg.strip_prefix("--") else {
                bail!("Unexpected positional argument '{arg}'");
            };
            if let Some((key, value)) = raw_key.split_once('=') {
                if value.is_empty() {
                    bail!("--{key} requires a value");
                }
                flags.values.insert(key.to_string(), value.to_string());
                continue;
            }
            match raw_key {
                "json" => flags.switches.push(raw_key.to_string()),
                key => {
                    let value = iter
                        .next()
                        .with_context(|| format!("--{key} requires a value"))?;
                    if value.starts_with("--") {
                        bail!("--{key} requires a value");
                    }
                    flags.values.insert(key.to_string(), value.clone());
                }
            }
        }
        Ok(flags)
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    fn has(&self, key: &str) -> bool {
        self.values.contains_key(key) || self.switches.iter().any(|value| value == key)
    }

    fn required(&self, key: &str) -> anyhow::Result<&str> {
        self.get(key)
            .with_context(|| format!("--{key} is required"))
    }
}

fn parse_diff_source(value: &str) -> anyhow::Result<ReviewDiffSource> {
    match value {
        "working-tree" | "working_tree" => Ok(ReviewDiffSource::WorkingTree),
        "staged" => Ok(ReviewDiffSource::Staged),
        "unstaged" => Ok(ReviewDiffSource::Unstaged),
        _ => bail!("Unknown diff source '{value}'. Expected working-tree, staged, or unstaged"),
    }
}

fn parse_scope(value: &str) -> anyhow::Result<CommentScope> {
    match value {
        "review" => Ok(CommentScope::Review),
        "file" => Ok(CommentScope::File),
        "line" => Ok(CommentScope::Line),
        _ => bail!("Unknown comment scope '{value}'. Expected review, file, or line"),
    }
}

fn parse_side(value: &str) -> anyhow::Result<LineSide> {
    match value {
        "old" => Ok(LineSide::Old),
        "new" => Ok(LineSide::New),
        _ => bail!("Unknown line side '{value}'. Expected old or new"),
    }
}

fn parse_bool(value: &str) -> anyhow::Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => bail!("Expected true or false, got '{value}'"),
    }
}

fn parse_u32(value: &str) -> anyhow::Result<u32> {
    value
        .parse()
        .with_context(|| format!("Expected unsigned integer, got '{value}'"))
}

fn parse_usize(value: &str) -> anyhow::Result<usize> {
    value
        .parse()
        .with_context(|| format!("Expected unsigned integer, got '{value}'"))
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn help() -> &'static str {
    "Usage:
  tuicr review open --repo . --diff-source working-tree --json
  tuicr review diff --session <id> --path src/main.rs --max-lines 200
  tuicr review comment add --session <id> --path src/main.rs --line 42 --side new --type issue --body \"...\"
  tuicr review file reviewed --session <id> --path src/main.rs --set true
  tuicr review export --session <id>
  tuicr review clear --session <id>"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn flags_parse_switches_equals_and_separate_values() {
        let flags = Flags::parse(&args(&[
            "--repo",
            ".",
            "--diff-source=working-tree",
            "--json",
        ]))
        .expect("parse flags");

        assert_eq!(flags.get("repo"), Some("."));
        assert_eq!(flags.get("diff-source"), Some("working-tree"));
        assert!(flags.has("json"));
    }

    #[test]
    fn flags_reject_unexpected_positionals() {
        let error = Flags::parse(&args(&["--body", "hello", "world"]))
            .expect_err("positional should be rejected");

        assert!(error.to_string().contains("Unexpected positional"));
    }

    #[test]
    fn diff_source_accepts_cli_spellings() {
        assert_eq!(
            parse_diff_source("working-tree").expect("working-tree"),
            ReviewDiffSource::WorkingTree
        );
        assert_eq!(
            parse_diff_source("working_tree").expect("working_tree"),
            ReviewDiffSource::WorkingTree
        );
        assert_eq!(
            parse_diff_source("staged").expect("staged"),
            ReviewDiffSource::Staged
        );
        assert_eq!(
            parse_diff_source("unstaged").expect("unstaged"),
            ReviewDiffSource::Unstaged
        );
    }
}
