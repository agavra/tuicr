//! Pins what the default keys do (#983).
//!
//! `should_map_every_key_to_its_action` checks the action
//! that each key maps to in each context.
//! That covers an open prompt owning the keys,
//! and `i` and `/` depending on the focused pane.
//!
//! The tests after it press keys through [`handle_key`] and [`handle_paste`].
//! They cover what one key's action cannot show:
//!
//! - prefix keys
//! - counts
//! - the vim comment editor
//! - paste
//! - the keys read ahead of every map
//!
//! [`super::submit_flow_tests`] has `dd` on a forge's comments,
//! next to the fixtures for them.

use std::path::Path;

use crossterm::event::KeyCode::{
    BackTab, Backspace, Down, End, Enter, Esc, Home, Left, PageDown, PageUp, Right, Tab, Up,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::render_perf_tests::{app_with, file};
use crate::app::*;
use crate::handler::{handle_key, handle_paste, map_normal_path_key};
use crate::input::Action as A;
use crate::model::{Comment, LineSide};
use crate::persistence::storage;
use crate::vcs::traits::CommitInfo;

const NONE: KeyModifiers = KeyModifiers::NONE;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const ALT: KeyModifiers = KeyModifiers::ALT;
const SUPER: KeyModifiers = KeyModifiers::SUPER;
/// How the Windows console reports AltGr.
const ALT_GR: KeyModifiers = CTRL.union(ALT);

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// A character as a terminal sends it: a capital letter carries Shift.
fn ch(c: char) -> KeyEvent {
    let modifiers = if c.is_ascii_uppercase() { SHIFT } else { NONE };
    KeyEvent::new(KeyCode::Char(c), modifiers)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), CTRL)
}

fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), ALT)
}

fn code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, NONE)
}

fn with(modifiers: KeyModifiers, code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

/// A named key with no modifier and with each single modifier.
fn every_modifier(code: KeyCode) -> [KeyEvent; 5] {
    [NONE, SHIFT, CTRL, ALT, SUPER].map(|modifiers| with(modifiers, code))
}

/// The keys each table is checked against:
/// printable ASCII, Ctrl and Alt with each letter, and the named keys under each modifier.
fn every_key() -> Vec<KeyEvent> {
    let named = [
        Enter, Esc, Tab, BackTab, Backspace, Up, Down, Left, Right, Home, End, PageUp, PageDown,
    ];
    let mut keys: Vec<KeyEvent> = (' '..='~').map(ch).collect();
    keys.extend(('a'..='z').flat_map(|c| [ctrl(c), alt(c)]));
    keys.extend(named.into_iter().flat_map(every_modifier));
    keys
}

// ---------------------------------------------------------------------------
// Key tables
// ---------------------------------------------------------------------------

type Row = (Vec<KeyEvent>, A);

fn bind<const N: usize>(keys: [KeyEvent; N], action: A) -> Row {
    (keys.to_vec(), action)
}

/// One state of the app and the action each key maps to in it.
struct Context {
    name: &'static str,
    app: App,
    rows: Vec<Row>,
    /// Prefix keys and count digits.
    /// Their action is internal, so the sequence tests pin what they do.
    skipped: Vec<KeyEvent>,
    /// The action of a key that no row lists.
    rest: fn(KeyEvent) -> A,
}

impl Context {
    fn new(name: &'static str, app: App, rows: Vec<Row>) -> Self {
        Self {
            name,
            app,
            rows,
            skipped: Vec::new(),
            rest: unbound,
        }
    }

    fn skipping(mut self, keys: &str) -> Self {
        self.skipped = keys.chars().map(ch).collect();
        self
    }

    fn rest(mut self, rest: fn(KeyEvent) -> A) -> Self {
        self.rest = rest;
        self
    }

    fn expected(&self, key: KeyEvent) -> A {
        let row = self.rows.iter().find(|(keys, _)| keys.contains(&key));
        row.map_or_else(|| (self.rest)(key), |(_, action)| action.clone())
    }
}

fn unbound(_: KeyEvent) -> A {
    A::None
}

/// A text prompt types a character sent alone or with Shift.
fn typed(key: KeyEvent) -> A {
    match key.code {
        KeyCode::Char(c) if key.modifiers == NONE || key.modifiers == SHIFT => A::InsertChar(c),
        _ => A::None,
    }
}

/// The comment box types a character whatever its modifiers.
fn typed_under_any_modifier(key: KeyEvent) -> A {
    match key.code {
        KeyCode::Char(c) => A::InsertChar(c),
        _ => A::None,
    }
}

#[test]
fn should_map_every_key_to_its_action() {
    // `handle_key` takes these two before it asks any map.
    let read_first = [ctrl('c'), ctrl('l')];
    let mut wrong = Vec::new();
    for context in contexts() {
        let listed = context.rows.iter().flat_map(|(keys, _)| keys.clone());
        for key in every_key().into_iter().chain(listed) {
            if context.skipped.contains(&key) || read_first.contains(&key) {
                continue;
            }
            let action = map_normal_path_key(&context.app, key);
            let expected = context.expected(key);
            if action != expected {
                wrong.push(format!(
                    "{}: {:?} {:?} maps to {action:?}, not {expected:?}",
                    context.name, key.modifiers, key.code
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A terminal reports a shifted symbol with or without Shift,
/// and the Windows console reports an AltGr symbol with Ctrl+Alt.
#[test]
fn should_map_a_symbol_the_same_with_shift_or_altgr() {
    for context in contexts() {
        let symbols = ('!'..='~').filter(char::is_ascii_punctuation);
        for c in symbols.filter(|c| !context.skipped.contains(&ch(*c))) {
            // The pickers open their filter on an unmodified `/` only.
            let exact = c == '/' && matches!(context.name, "theme picker" | "file picker");
            let expected = if exact {
                A::None
            } else {
                map_normal_path_key(&context.app, ch(c))
            };
            for modifiers in [SHIFT, ALT_GR] {
                let key = with(modifiers, KeyCode::Char(c));
                assert_eq!(
                    map_normal_path_key(&context.app, key),
                    expected,
                    "{}: {modifiers:?} {c}",
                    context.name
                );
            }
        }
    }
}

fn contexts() -> Vec<Context> {
    // `z`, `Z`, `d` and the leader start a sequence, and a digit starts a count.
    let review_prefixes = "zZd;0123456789";
    vec![
        Context::new("diff", review(), review_pane()).skipping(review_prefixes),
        Context::new("file tree", focused(FocusedPanel::FileList), file_tree())
            .skipping(review_prefixes),
        Context::new(
            "comment list",
            focused(FocusedPanel::Comments),
            review_pane(),
        )
        .skipping(review_prefixes),
        Context::new(
            "commit list",
            focused(FocusedPanel::CommitSelector),
            review_pane(),
        )
        .skipping(review_prefixes),
        Context::new("visual", in_mode(InputMode::VisualSelect), visual()),
        Context::new("help", in_mode(InputMode::Help), help()),
        Context::new(
            "message details",
            in_mode(InputMode::MessageDetails),
            message_details(),
        ),
        Context::new("summary", in_mode(InputMode::Summary), summary()).skipping("d"),
        Context::new("confirm", in_mode(InputMode::Confirm), confirm()),
        Context::new(
            "target selector",
            in_mode(InputMode::CommitSelect),
            target_selector(),
        )
        .skipping("d"),
        Context::new(
            "submit picker",
            in_mode(InputMode::SubmitActionPicker),
            submit_picker(),
        ),
        Context::new(
            "submit resolver",
            in_mode(InputMode::SubmitResolver),
            submit_resolver(),
        ),
        Context::new(
            "submit confirm",
            in_mode(InputMode::SubmitConfirm),
            submit_confirm(),
        ),
        Context::new(
            "theme picker",
            in_mode(InputMode::ThemePicker),
            picker(A::ThemePickerFilter),
        ),
        Context::new(
            "file picker",
            in_mode(InputMode::FilePicker),
            picker(A::FilePickerFilter),
        ),
        // Text inputs. An open prompt owns the keys, whichever pane has focus.
        Context::new("command line", in_mode(InputMode::Command), command_line()).rest(typed),
        Context::new("search", in_mode(InputMode::Search), prompt()).rest(typed),
        Context::new("file tree prompt", tree_prompt(), prompt()).rest(typed),
        Context::new("target filter", target_filter(), prompt()).rest(typed),
        Context::new("theme filter", theme_filter(), picker_filter()).rest(typed),
        Context::new("file filter", file_filter(), file_picker_filter()).rest(typed),
        Context::new("comment box", in_mode(InputMode::Comment), comment_box())
            .rest(typed_under_any_modifier),
    ]
}

/// The diff, the comment list and the commit list.
fn review_pane() -> Vec<Row> {
    vec![
        bind([ch('j'), code(Down)], A::CursorDown(1)),
        bind([ch('k'), code(Up)], A::CursorUp(1)),
        bind([ctrl('e')], A::ScrollViewDown(1)),
        bind([ctrl('y')], A::ScrollViewUp(1)),
        bind([ctrl('d')], A::HalfPageDown),
        bind([ctrl('u')], A::HalfPageUp),
        bind([ctrl('f'), code(PageDown)], A::PageDown),
        bind([ctrl('b'), code(PageUp)], A::PageUp),
        bind([ch('h'), code(Left)], A::ScrollLeft(4)),
        bind([ch('l'), code(Right)], A::ScrollRight(4)),
        bind([ch('g')], A::GoToTop),
        bind([ch('G')], A::GoToBottom),
        bind([ch('}')], A::NextFile),
        bind([ch('{')], A::PrevFile),
        bind([ch(']')], A::NextHunk),
        bind([ch('[')], A::PrevHunk),
        bind([ch('m')], A::NextComment),
        bind([ch('M')], A::PrevComment),
        bind([ch(')')], A::CycleCommitNext),
        bind([ch('(')], A::CycleCommitPrev),
        bind([code(Tab)], A::ToggleFocus),
        bind(every_modifier(BackTab), A::ToggleFocusReverse),
        bind([code(Enter)], A::SelectFile),
        bind([with(SHIFT, Enter)], A::SelectFileFull),
        bind([ch(' ')], A::ToggleExpand),
        bind([ch('o')], A::ExpandAll),
        bind([ch('O')], A::CollapseAll),
        bind([ch('r')], A::ToggleReviewed),
        bind([ch('R')], A::ToggleHunkReviewed),
        bind([ch('c')], A::AddLineComment),
        bind([ch('C')], A::AddFileComment),
        bind([ch('i')], A::EditComment),
        bind([ch('A')], A::EditCommentAtEnd),
        bind([ch('e')], A::EditFile),
        bind([ch('v'), ch('V'), ctrl('v'), alt('v')], A::EnterVisualMode),
        bind([ch('y')], A::ExportToClipboard),
        bind([ch('Y')], A::CopyCommentAtCursor),
        bind([ch('/')], A::EnterSearchMode),
        bind([ch('n')], A::SearchNext),
        bind([ch('N')], A::SearchPrev),
        bind([code(Esc)], A::ClearSearchHighlight),
        bind([ch(':')], A::EnterCommandMode),
        bind([ch('?')], A::ToggleHelp),
        bind([ch('q')], A::QuitHint),
    ]
}

/// The review pane's keys, with `i`, `e`, `I`, `E` and `/` taken by the tree's filters.
fn file_tree() -> Vec<Row> {
    let tree = vec![
        bind([ch('i')], A::FileTreeFilterInclude),
        bind([ch('e')], A::FileTreeFilterExclude),
        bind([ch('I')], A::FileTreeClearInclude),
        bind([ch('E')], A::FileTreeClearExclude),
        bind([ch('/')], A::FileTreeSearch),
    ];
    let mut rows = review_pane();
    rows.retain(|(keys, _)| !tree.iter().any(|(taken, _)| taken == keys));
    rows.extend(tree);
    rows
}

fn visual() -> Vec<Row> {
    vec![
        bind([ch('j'), code(Down)], A::CursorDown(1)),
        bind([ch('k'), code(Up)], A::CursorUp(1)),
        bind([ch('c'), code(Enter)], A::AddRangeComment),
        bind([ch('s')], A::AddRangeSuggestion),
        bind([ch('y')], A::ExportToClipboard),
        bind(
            [code(Esc), ch('v'), ch('V'), ctrl('v'), alt('v')],
            A::ExitMode,
        ),
        bind([ch('q')], A::QuitHint),
    ]
}

fn help() -> Vec<Row> {
    let mut rows = message_details();
    rows.extend([
        bind([ch('/')], A::EnterSearchMode),
        bind([ch('n')], A::SearchNext),
        bind([ch('N')], A::SearchPrev),
    ]);
    rows
}

/// The help popup's keys without its search.
fn message_details() -> Vec<Row> {
    vec![
        bind([code(Esc), ch('q'), ch('?')], A::ToggleHelp),
        bind([ch('j'), code(Down)], A::CursorDown(1)),
        bind([ch('k'), code(Up)], A::CursorUp(1)),
        bind([ch('h'), code(Left)], A::ScrollLeft(4)),
        bind([ch('l'), code(Right)], A::ScrollRight(4)),
        bind([ctrl('d')], A::HalfPageDown),
        bind([ctrl('u')], A::HalfPageUp),
        bind([ctrl('f'), code(PageDown)], A::PageDown),
        bind([ctrl('b'), code(PageUp)], A::PageUp),
        bind([ch('g')], A::GoToTop),
        bind([ch('G')], A::GoToBottom),
    ]
}

fn summary() -> Vec<Row> {
    vec![
        bind([code(Esc), ch('q')], A::ExitMode),
        bind([ch('j'), code(Down)], A::CursorDown(1)),
        bind([ch('k'), code(Up)], A::CursorUp(1)),
        bind([code(Enter)], A::SubmitInput),
        bind([ctrl('d')], A::HalfPageDown),
        bind([ctrl('u')], A::HalfPageUp),
        bind([ctrl('f'), code(PageDown)], A::PageDown),
        bind([ctrl('b'), code(PageUp)], A::PageUp),
        bind([ch('g')], A::GoToTop),
        bind([ch('G')], A::GoToBottom),
    ]
}

/// The dialog ignores modifiers.
fn confirm() -> Vec<Row> {
    let mut yes = vec![ch('y'), ch('Y'), ctrl('y'), alt('y')];
    yes.extend(every_modifier(Enter));
    let mut no = vec![ch('n'), ch('N'), ctrl('n'), alt('n')];
    no.extend(every_modifier(Esc));
    vec![(yes, A::ConfirmYes), (no, A::ConfirmNo)]
}

fn submit_confirm() -> Vec<Row> {
    let mut rows = confirm();
    rows.push(bind(
        [ch('r'), ch('R'), ctrl('r'), alt('r')],
        A::SubmitReloadPr,
    ));
    rows
}

fn target_selector() -> Vec<Row> {
    vec![
        bind([ch('j'), code(Down)], A::CommitSelectDown),
        bind([ch('k'), code(Up)], A::CommitSelectUp),
        bind([ch(' ')], A::ToggleCommitSelect),
        bind([code(Enter)], A::ConfirmCommitSelect),
        bind([code(Tab)], A::TargetSelectorTabNext),
        bind(every_modifier(BackTab), A::TargetSelectorTabPrev),
        bind([ch('/')], A::BeginTargetFilter),
        bind([ch('r')], A::TogglePrReviewRequestedFilter),
        bind([ch('a')], A::SessionsTabToggleShowEmpty),
        bind([ch(':')], A::EnterCommandMode),
        bind([code(Esc)], A::ExitMode),
        bind([ch('q')], A::QuitHint),
    ]
}

fn submit_picker() -> Vec<Row> {
    vec![
        bind([ch('j'), code(Down)], A::SubmitPickerDown),
        bind([ch('k'), code(Up)], A::SubmitPickerUp),
        bind([code(Enter)], A::SubmitPickerConfirm),
        bind([code(Esc)], A::ExitMode),
        bind([ch('q')], A::QuitHint),
    ]
}

fn submit_resolver() -> Vec<Row> {
    vec![
        bind([ch('j'), code(Down)], A::SubmitResolverDown),
        bind([ch('k'), code(Up)], A::SubmitResolverUp),
        bind([code(Enter), ch(' ')], A::SubmitResolverToggle),
        bind([ch('s')], A::SubmitResolverAdvance),
        bind([code(Esc)], A::ExitMode),
    ]
}

/// The theme picker and the file picker, which differ in the action that opens their filter.
fn picker(filter: A) -> Vec<Row> {
    vec![
        bind([ch('j'), code(Down)], A::CursorDown(1)),
        bind([ch('k'), code(Up)], A::CursorUp(1)),
        bind([code(Enter)], A::SubmitInput),
        bind([ch('/')], filter),
        bind([code(Esc)], A::ExitMode),
    ]
}

/// The search line, the file-tree prompts and the target filter.
fn prompt() -> Vec<Row> {
    let mut rows = picker_filter();
    rows.push(bind([ctrl('w'), with(ALT, Backspace)], A::DeleteWord));
    rows
}

fn command_line() -> Vec<Row> {
    let mut rows = prompt();
    rows.extend([
        bind([code(Tab)], A::CompleteCommand),
        bind(every_modifier(BackTab), A::CompleteCommandReverse),
    ]);
    rows
}

fn picker_filter() -> Vec<Row> {
    vec![
        bind([code(Esc)], A::ExitMode),
        bind([code(Enter)], A::SubmitInput),
        bind([code(Backspace)], A::DeleteChar),
        bind([ctrl('u')], A::ClearLine),
    ]
}

/// Letters are text in the filter, so only the arrows move the selection.
fn file_picker_filter() -> Vec<Row> {
    let mut rows = picker_filter();
    rows.extend([
        bind([code(Down)], A::CursorDown(1)),
        bind([code(Up)], A::CursorUp(1)),
    ]);
    rows
}

fn comment_box() -> Vec<Row> {
    vec![
        bind([code(Esc)], A::ExitMode),
        bind([code(Enter), with(CTRL, Enter), ctrl('s')], A::SubmitInput),
        bind(
            [with(SHIFT, Enter), with(ALT, Enter), ctrl('j'), ctrl('k')],
            A::InsertChar('\n'),
        ),
        bind(every_modifier(Tab), A::CycleCommentType),
        bind([ch('\t')], A::CycleCommentType),
        bind(every_modifier(BackTab), A::CycleCommentTypeReverse),
        bind([code(Up)], A::TextCursorUp),
        bind([code(Down)], A::TextCursorDown),
        bind([code(Left)], A::TextCursorLeft),
        bind([code(Right)], A::TextCursorRight),
        bind(
            [with(ALT, Left), with(CTRL, Left), alt('b')],
            A::TextCursorWordLeft,
        ),
        bind(
            [with(ALT, Right), with(CTRL, Right), alt('f')],
            A::TextCursorWordRight,
        ),
        bind([ctrl('a'), with(SUPER, Left)], A::TextCursorLineStart),
        bind(every_modifier(Home), A::TextCursorLineStart),
        bind([ctrl('e'), with(SUPER, Right)], A::TextCursorLineEnd),
        bind(every_modifier(End), A::TextCursorLineEnd),
        bind([code(Backspace)], A::DeleteChar),
        bind(
            [ctrl('w'), with(ALT, Backspace), with(SUPER, Backspace)],
            A::DeleteWord,
        ),
        bind([ctrl('u')], A::ClearLine),
    ]
}

// ---------------------------------------------------------------------------
// Prompts and focus
// ---------------------------------------------------------------------------

#[test]
fn should_type_into_an_open_prompt_whichever_pane_has_focus() {
    let drafts = [
        (tree_prompt(), tree_draft as fn(&App) -> Option<String>),
        (target_filter(), |app| app.pr_filter_draft.clone()),
        (theme_filter(), |app| app.theme_picker.draft.clone()),
        (file_filter(), |app| app.file_picker.draft.clone()),
    ];
    for (mut app, draft) in drafts {
        let cursor = app.diff_state.cursor_line;

        press(&mut app, "jq:");

        assert_eq!(draft(&app).as_deref(), Some("jq:"));
        assert_eq!(app.diff_state.cursor_line, cursor);
    }
}

#[test]
fn should_give_i_and_slash_to_the_focused_pane() {
    let mut diff = on_comment();
    press(&mut diff, "i");
    assert_eq!(diff.input_mode, InputMode::Comment);

    let mut diff = review();
    press(&mut diff, "/");
    assert_eq!(diff.input_mode, InputMode::Search);

    for (key, prompt) in [
        ('i', FileTreePrompt::Include),
        ('/', FileTreePrompt::Search),
    ] {
        let mut tree = focused(FocusedPanel::FileList);
        press(&mut tree, &key.to_string());
        assert_eq!(tree.input_mode, InputMode::Normal);
        assert_eq!(
            tree.file_tree_draft().map(|draft| draft.prompt),
            Some(prompt)
        );
    }
}

/// `y` copies the review in a review pane and the selection in a visual selection.
/// Only the second leaves the mode it was pressed in.
#[cfg(unix)]
#[test]
fn should_copy_the_selection_on_y_in_a_visual_selection() {
    let mut app = review();
    // A shell command, so the test never reaches the real clipboard.
    app.clipboard_command = Some("cat >/dev/null".into());
    press(&mut app, "vj");
    assert_eq!(app.input_mode, InputMode::VisualSelect);

    press(&mut app, "y");

    assert_eq!(app.input_mode, InputMode::Normal);
    let copied = app.message.as_ref().map(|message| message.content.as_str());
    assert_eq!(copied.map(|text| text.starts_with("Copied ")), Some(true));
}

// ---------------------------------------------------------------------------
// Prefix keys
// ---------------------------------------------------------------------------

#[test]
fn should_position_the_cursor_line_with_the_z_prefix() {
    let positions: [(&str, Direct); 3] = [
        ("zz", App::center_cursor),
        ("zt", App::cursor_to_top),
        ("zb", App::cursor_to_bottom),
    ];
    let mut offsets = Vec::new();
    for (keys, position) in positions {
        let mut typed = review();
        let mut direct = review();

        press(&mut typed, keys);
        position(&mut direct);

        assert_eq!(
            typed.diff_state.scroll_offset, direct.diff_state.scroll_offset,
            "{keys}"
        );
        offsets.push(typed.diff_state.scroll_offset);
    }
    offsets.sort_unstable();
    offsets.dedup();
    assert_eq!(offsets.len(), 3, "the fixture must tell the three apart");
}

#[test]
fn should_save_export_and_quit_on_zz() {
    let _reviews = isolated_reviews();
    let mut app = review();
    app.output_to_stdout = true;

    press(&mut app, "ZZ");

    assert!(app.should_quit);
    assert!(app.pending_stdout_output.is_some());
    assert!(!app.dirty);
}

#[test]
fn should_quit_without_exporting_on_zq() {
    let mut app = review();
    app.output_to_stdout = true;
    app.dirty = true;

    press(&mut app, "ZQ");

    assert!(app.should_quit);
    assert!(app.pending_stdout_output.is_none());
    assert!(app.dirty);
}

#[test]
fn should_delete_the_comment_at_the_cursor_on_dd() {
    let mut app = on_comment();
    let comments = app.session.comment_count();

    press(&mut app, "dd");

    assert_eq!(app.session.comment_count(), comments - 1);
}

#[test]
fn should_delete_nothing_when_the_second_d_is_modified() {
    let mut app = on_comment();
    let comments = app.session.comment_count();

    press(&mut app, "d");
    handle_key(&mut app, ctrl('d'));

    assert_eq!(app.session.comment_count(), comments);
}

#[test]
fn should_delete_the_selected_summary_comment_on_dd() {
    let mut app = summary_popup();
    let comments = app.session.comment_count();

    press(&mut app, "dd");

    assert_eq!(app.input_mode, InputMode::Summary);
    assert_eq!(app.session.comment_count(), comments - 1);
}

#[test]
fn should_ask_to_delete_the_session_on_dd_in_the_sessions_tab() {
    let _reviews = isolated_reviews();
    let mut app = review();
    app.save_current_session_merging_external()
        .expect("save session");
    app.load_sessions_tab();
    app.target_tab = TargetTab::Sessions;
    app.input_mode = InputMode::CommitSelect;
    let comments = app.session.comment_count();

    press(&mut app, "dd");

    assert_eq!(app.input_mode, InputMode::Confirm);
    assert_eq!(app.pending_confirm, Some(ConfirmAction::DeleteSession));
    assert_eq!(app.session.comment_count(), comments);
}

#[test]
fn should_run_the_leader_bindings() {
    let bindings: [(char, Direct); 6] = [
        ('e', App::toggle_file_list),
        ('c', App::enter_review_comment_mode),
        ('s', App::toggle_commit_selector),
        ('f', App::toggle_single_file_view),
        ('p', App::enter_file_picker_mode),
        ('n', |app| {
            app.relative_line_numbers = !app.relative_line_numbers
        }),
    ];
    let view = |app: &App| {
        (
            app.show_file_list,
            app.input_mode,
            app.show_commit_selector,
            app.is_single_file_view,
            app.relative_line_numbers,
        )
    };
    for (key, run) in bindings {
        let mut typed = review();
        let mut direct = review();

        press(&mut typed, &format!(";{key}"));
        run(&mut direct);

        assert_eq!(view(&typed), view(&direct), ";{key}");
        assert_ne!(view(&typed), view(&review()), ";{key}");
    }
}

#[test]
fn should_move_focus_with_the_leader_and_a_direction() {
    use FocusedPanel::{Comments, CommitSelector, Diff, FileList};
    let moves = [
        (Diff, 'h', FileList),
        (FileList, 'l', Diff),
        (Diff, 'k', CommitSelector),
        (Comments, 'k', FileList),
        (FileList, 'j', Comments),
        (CommitSelector, 'j', Diff),
    ];
    for (from, key, to) in moves {
        let mut app = focused(from);

        press(&mut app, &format!(";{key}"));

        assert_eq!(app.focused_panel, to, ";{key} from {from:?}");
    }
}

/// `;e` hides the file list.
/// It must not open the tree's exclude prompt, which a bare `e` does.
#[test]
fn should_read_the_leader_before_the_file_tree_keys() {
    let mut app = focused(FocusedPanel::FileList);

    press(&mut app, ";e");

    assert!(!app.show_file_list);
    assert!(!app.file_tree_prompt_editing());
}

/// After a prefix, a key that does not finish the sequence acts on its own,
/// and the count typed before the prefix is gone.
#[test]
fn should_drop_the_count_and_run_the_next_key_after_an_unfinished_prefix() {
    for prefix in ['z', 'Z', 'd', ';'] {
        let mut typed = review();
        let mut direct = review();

        press(&mut typed, &format!("3{prefix}}}"));
        direct.next_file();

        assert_eq!(position(&typed), position(&direct), "3{prefix}}}");
        assert_eq!(typed.pending_count, None);
    }
}

/// The summary lists its comments when it is drawn,
/// so a key that opens it or deletes from it asks for a frame before the next queued key.
#[test]
fn should_ask_for_a_frame_when_the_summary_list_changes() {
    let mut opening = in_mode(InputMode::Command);
    opening.command_buffer = "summary".into();
    assert!(handle_key(&mut opening, code(Enter)));
    assert_eq!(opening.input_mode, InputMode::Summary);

    let mut deleting = summary_popup();
    assert!(!handle_key(&mut deleting, ch('d')));
    assert!(handle_key(&mut deleting, ch('d')));

    assert!(!handle_key(&mut review(), ch('j')));
}

// ---------------------------------------------------------------------------
// Counts
// ---------------------------------------------------------------------------

#[test]
fn should_apply_a_count_to_the_next_motion() {
    let motions: [(&str, Direct); 6] = [
        ("5j", |app| app.cursor_down(5)),
        ("3k", |app| app.cursor_up(3)),
        ("2l", |app| app.scroll_right(8)),
        ("2}", |app| (0..2).for_each(|_| app.next_file())),
        ("2]", |app| (0..2).for_each(|_| app.next_hunk())),
        ("12G", |app| app.go_to_source_line(12, LineSide::New)),
    ];
    for (keys, run) in motions {
        let mut typed = review();
        let mut direct = review();
        let mut uncounted = review();

        press(&mut typed, keys);
        run(&mut direct);
        press(&mut uncounted, &keys[keys.len() - 1..]);

        assert_eq!(position(&typed), position(&direct), "{keys}");
        assert_ne!(position(&typed), position(&uncounted), "{keys}");
        assert_eq!(typed.pending_count, None, "{keys}");
    }
}

#[test]
fn should_drop_the_count_on_a_key_that_is_not_a_motion() {
    let mut app = review();
    let file = app
        .current_file_path()
        .expect("a file at the cursor")
        .clone();

    press(&mut app, "2r");

    // Two toggles would leave the file unreviewed.
    assert!(app.session.is_file_reviewed(&file));
    assert_eq!(app.pending_count, None);
}

#[test]
fn should_take_a_count_in_the_review_panes_only() {
    let mut command_line = review();
    press(&mut command_line, ":42");
    assert_eq!(command_line.command_buffer, "42");
    assert_eq!(command_line.pending_count, None);

    for mode in [InputMode::Help, InputMode::VisualSelect] {
        let mut app = in_mode(mode);
        press(&mut app, "3");
        assert_eq!(app.pending_count, None, "{mode:?}");
    }
}

// ---------------------------------------------------------------------------
// The vim comment editor
// ---------------------------------------------------------------------------

/// The plain comment box closes on Esc and types `x`.
/// The vim editor goes to its normal mode on Esc, where `x` deletes a character.
#[test]
fn should_send_comment_keys_to_the_vim_editor() {
    let mut app = vim_comment_box();

    press(&mut app, "hi");
    handle_key(&mut app, code(Esc));
    press(&mut app, "x");

    assert_eq!(app.input_mode, InputMode::Comment);
    assert!(app.comment_vim_in_normal_mode());
    assert_eq!(app.comment_buffer, "h");
}

#[test]
fn should_save_or_cancel_from_the_vim_editor() {
    let saves = [
        vec![ctrl('s')],
        vec![with(CTRL, Enter)],
        vec![with(ALT, Enter)],
        vec![code(Esc), code(Enter), code(Enter)],
        vec![code(Esc), ch(':'), ch('w'), code(Enter)],
    ];
    let cancels = [
        vec![with(ALT, Esc)],
        vec![code(Esc), code(Esc), code(Esc)],
        vec![code(Esc), ch('q'), ch('q')],
        vec![code(Esc), ch(':'), ch('q'), code(Enter)],
    ];
    let cases = saves
        .iter()
        .map(|keys| (keys, 1))
        .chain(cancels.iter().map(|keys| (keys, 0)));
    for (keys, saved) in cases {
        // Saving a comment writes the session, and merges the one on disk.
        let _reviews = isolated_reviews();
        let mut app = vim_comment_box();
        let comments = app.session.comment_count();
        press(&mut app, "hi");

        for key in keys {
            handle_key(&mut app, *key);
        }

        assert_eq!(app.input_mode, InputMode::Normal, "{keys:?}");
        assert_eq!(app.session.comment_count(), comments + saved, "{keys:?}");
    }
}

#[test]
fn should_insert_a_soft_tab_in_vim_insert_mode() {
    let mut app = vim_comment_box();

    handle_key(&mut app, code(Tab));

    assert_eq!(app.comment_buffer, " ".repeat(app.comment_tab_width));
}

// ---------------------------------------------------------------------------
// Paste
// ---------------------------------------------------------------------------

#[test]
fn should_paste_into_the_open_text_input() {
    let pasted = "one\r\ntwo";
    let single_line = Some("onetwo".to_string());

    let mut app = in_mode(InputMode::Comment);
    handle_paste(&mut app, pasted.into());
    assert_eq!(app.comment_buffer, "one\ntwo");

    // The next key writes the editor's text back, so the paste must be in the editor.
    let mut app = vim_comment_box();
    press(&mut app, "a");
    handle_paste(&mut app, "bc".into());
    press(&mut app, "d");
    assert_eq!(app.comment_buffer.len(), 4);
    assert!(app.comment_buffer.contains('b'));

    let mut app = in_mode(InputMode::Command);
    handle_paste(&mut app, pasted.into());
    assert_eq!(Some(app.command_buffer), single_line);

    let mut app = in_mode(InputMode::Search);
    handle_paste(&mut app, pasted.into());
    assert_eq!(Some(app.search_buffer), single_line);

    let mut app = tree_prompt();
    handle_paste(&mut app, pasted.into());
    assert_eq!(tree_draft(&app), single_line);

    let mut app = target_filter();
    handle_paste(&mut app, pasted.into());
    assert_eq!(app.pr_filter_draft, single_line);

    let mut app = theme_filter();
    handle_paste(&mut app, pasted.into());
    assert_eq!(app.theme_picker.draft, single_line);

    let mut app = file_filter();
    handle_paste(&mut app, pasted.into());
    assert_eq!(app.file_picker.draft, single_line);
}

/// Pasted text must not run as keys: `:q` would otherwise open the command line.
#[test]
fn should_ignore_a_paste_outside_a_text_input() {
    let modes = [
        InputMode::Normal,
        InputMode::VisualSelect,
        InputMode::Help,
        InputMode::CommitSelect,
        InputMode::ThemePicker,
        InputMode::FilePicker,
    ];
    for mode in modes {
        let mut app = in_mode(mode);
        let before = position(&app);

        handle_paste(&mut app, "jj:q\n".into());

        assert_eq!(app.input_mode, mode);
        assert_eq!(position(&app), before, "{mode:?}");
        assert!(app.command_buffer.is_empty(), "{mode:?}");
    }
}

// ---------------------------------------------------------------------------
// Keys read ahead of every map
// ---------------------------------------------------------------------------

#[test]
fn should_exit_on_a_second_ctrl_c_in_any_mode() {
    let modes = [
        InputMode::Normal,
        InputMode::Command,
        InputMode::Help,
        InputMode::Confirm,
    ];
    for mode in modes {
        let mut app = in_mode(mode);

        handle_key(&mut app, ctrl('c'));
        assert!(!app.should_quit, "{mode:?}");
        assert!(app.message.is_some(), "{mode:?}");

        handle_key(&mut app, ctrl('c'));
        assert!(app.should_quit, "{mode:?}");
    }
}

#[test]
fn should_forget_the_first_ctrl_c_on_another_key() {
    let mut app = review();

    handle_key(&mut app, ctrl('c'));
    press(&mut app, "j");
    assert!(app.message.is_none());

    handle_key(&mut app, ctrl('c'));
    assert!(!app.should_quit);
}

#[test]
fn should_close_the_comment_box_on_the_first_ctrl_c() {
    for mut app in [in_mode(InputMode::Comment), vim_comment_box()] {
        press(&mut app, "hi");

        handle_key(&mut app, ctrl('c'));

        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(!app.should_quit);
    }
}

#[test]
fn should_redraw_on_ctrl_l_without_typing_it() {
    let mut comment_box = in_mode(InputMode::Comment);
    handle_key(&mut comment_box, ctrl('l'));
    assert!(comment_box.take_full_redraw());
    assert!(comment_box.comment_buffer.is_empty());

    let mut vim = vim_comment_box();
    handle_key(&mut vim, ctrl('l'));
    assert!(vim.take_full_redraw());
    assert!(vim.comment_buffer.is_empty());

    let mut command_line = in_mode(InputMode::Command);
    handle_key(&mut command_line, ctrl('l'));
    assert!(command_line.take_full_redraw());
    assert!(command_line.command_buffer.is_empty());

    let mut diff = review();
    handle_key(&mut diff, ctrl('l'));
    assert!(diff.take_full_redraw());
}

#[test]
fn should_quit_on_q_only_with_q_quits() {
    for mode in [
        InputMode::Normal,
        InputMode::VisualSelect,
        InputMode::CommitSelect,
        InputMode::SubmitActionPicker,
    ] {
        let mut hinting = in_mode(mode);
        press(&mut hinting, "q");
        assert!(!hinting.should_quit, "{mode:?}");
        assert!(hinting.message.is_some(), "{mode:?}");

        let mut quitting = in_mode(mode);
        quitting.q_quits = true;
        press(&mut quitting, "q");
        assert!(quitting.should_quit, "{mode:?}");
    }
}

// ---------------------------------------------------------------------------
// States
// ---------------------------------------------------------------------------

/// What a test does to the app without the keyboard, to compare with the keys.
type Direct = fn(&mut App);

/// Press each character of `keys` in turn.
fn press(app: &mut App, keys: &str) {
    for c in keys.chars() {
        handle_key(app, ch(c));
    }
}

/// Where the diff cursor and its view are.
fn position(app: &App) -> (usize, usize, usize) {
    let diff = &app.diff_state;
    (diff.cursor_line, diff.scroll_offset, diff.scroll_x)
}

/// Keep session saves out of the directory other tests on this thread use.
fn isolated_reviews() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    storage::set_test_reviews_dir(Some(dir.path().to_path_buf()));
    dir
}

/// A review of three commits that change four files,
/// with a line comment, a file comment, and a review comment.
/// The diff pane is focused, with the cursor in the middle of the second file.
fn review() -> App {
    let files = [
        ("README.md", 8),
        ("src/app.rs", 60),
        ("src/keys.rs", 40),
        ("tests/it.rs", 8),
    ];
    let mut app = app_with(files.map(|(path, lines)| file(path, lines)).to_vec());

    // The sizes a 100x30 frame gives the panes.
    app.diff_state.viewport_height = 20;
    app.diff_state.viewport_width = 70;
    app.diff_state.max_content_width = 90;
    app.file_list_state.viewport_height = 10;

    // The inline commit selector, with every commit selected.
    let commits = ["aaa", "bbb", "ccc"];
    app.diff_source = DiffSource::CommitRange(commits.map(String::from).to_vec());
    app.review_commits = commits.map(commit).to_vec();
    app.show_commit_selector = true;
    app.commit_selection_range = Some((0, 2));

    let comment = |text: &str, side| Comment::new(text.into(), app.comment_type.clone(), side);
    let on_line = comment("a line comment", Some(LineSide::New));
    let on_file = comment("a file comment", None);
    let on_review = comment("a review comment", None);
    let file = app.session.files.get_mut(Path::new("src/app.rs"));
    let file = file.expect("fixture file");
    file.line_comments.insert(12, vec![on_line]);
    file.file_comments.push(on_file);
    app.session.review_comments.push(on_review);
    app.rebuild_annotations();
    app.expand_all_dirs();

    // Away from every edge, so each motion has room to show.
    app.next_file();
    app.cursor_down(30);
    app.scroll_right(4);
    app
}

fn focused(panel: FocusedPanel) -> App {
    let mut app = review();
    app.focused_panel = panel;
    app
}

fn in_mode(mode: InputMode) -> App {
    let mut app = review();
    app.input_mode = mode;
    app
}

/// The cursor on the file comment.
fn on_comment() -> App {
    let mut app = review();
    app.next_comment();
    app
}

fn tree_draft(app: &App) -> Option<String> {
    app.file_tree_draft().map(|draft| draft.buffer.clone())
}

/// The file tree's search prompt, open while the diff has focus.
fn tree_prompt() -> App {
    let mut app = review();
    app.begin_file_tree_prompt(FileTreePrompt::Search);
    app
}

/// The filter of the target selector's pull request tab.
fn target_filter() -> App {
    let mut app = in_mode(InputMode::CommitSelect);
    app.pr_filter_draft = Some(String::new());
    app
}

fn theme_filter() -> App {
    let mut app = review();
    app.enter_theme_picker_mode();
    app.begin_theme_picker_filter();
    app
}

fn file_filter() -> App {
    let mut app = review();
    app.enter_file_picker_mode();
    app.begin_file_picker_filter();
    app
}

/// An empty comment box on the cursor line, with the vim editor on.
fn vim_comment_box() -> App {
    let mut app = review();
    app.comment_vim_enabled = true;
    press(&mut app, "c");
    app
}

/// The summary popup.
/// `dd` acts on the list the popup drew, so it is drawn once.
fn summary_popup() -> App {
    let mut app = review();
    app.enter_summary_mode();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
    terminal
        .draw(|frame| crate::ui::summary_popup::render_summary(frame, &mut app, frame.area()))
        .expect("draw summary");
    app
}

fn commit(id: &str) -> CommitInfo {
    CommitInfo {
        id: id.into(),
        short_id: id.into(),
        branch_name: None,
        summary: format!("commit {id}"),
        body: None,
        author: "someone".into(),
        time: chrono::DateTime::UNIX_EPOCH,
    }
}
