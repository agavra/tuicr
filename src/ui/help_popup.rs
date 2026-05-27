use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::App;
use crate::ui::styles;

pub fn render_help(frame: &mut Frame, app: &mut App) {
    let theme = &app.theme;
    // Center over the diff pane (matches the submit-modal anchoring) so the
    // file list doesn't tug the popup's visual centre off to one side. Fall
    // back to the full frame when no diff area is laid out yet.
    let anchor = app.diff_area.unwrap_or(frame.area());
    let area = centered_rect(80, 90, anchor);

    // Clear the area behind the popup
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Help (j/k to scroll) - Press ? or Esc to close ")
        .borders(Borders::ALL)
        .style(styles::popup_style(theme))
        .border_style(styles::border_style(theme, true));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let help_text = vec![
        Line::from(Span::styled(
            "Navigation",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  j/k       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Scroll down/up"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl-e/y  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Scroll view down/up"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl-d/u  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Half page down/up"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl-f/b  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Full page down/up"),
        ]),
        Line::from(vec![
            Span::styled(
                "  g/G       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Go to first/last file"),
        ]),
        Line::from(vec![
            Span::styled(
                "  {N}G      ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Go to source line N in current file"),
        ]),
        Line::from(vec![
            Span::styled(
                "  {/}       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Jump to prev/next file"),
        ]),
        Line::from(vec![
            Span::styled(
                "  [/]       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Jump to prev/next hunk"),
        ]),
        Line::from(vec![
            Span::styled(
                "  /         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Search within diff"),
        ]),
        Line::from(vec![
            Span::styled(
                "  n/N       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Next/prev search match"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Enter     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Expand/collapse context (20 lines)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  S-Enter   ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Expand/collapse all hidden context"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Tab/S-Tab ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle focus next/previous panel"),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}h/{}l     ", app.leader_key, app.leader_key),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Focus file list/diff"),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}k/{}j     ", app.leader_key, app.leader_key),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Move focus up/down between panes"),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}e        ", app.leader_key),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle file list visibility"),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}f        ", app.leader_key),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle single-file view (also `:focus` / `:f`)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  h/l       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Scroll diff left/right (or ←/→)"),
        ]),
        Line::from(vec![
            Span::styled("  h/← at 0 ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("Reveal + focus file list"),
        ]),
        Line::from(vec![
            Span::styled(
                "  l/→ in list",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Hide list + focus diff"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!("Single-file view (`:focus`, `:f`, {}f)", app.leader_key),
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(Span::raw(
            "  One file at a time. j/k walks across files, ]/[ walks across hunks. Default in `--all-files`.",
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Commit Selector (multi-commit reviews)",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  j/k       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Navigate commits"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Space/Enter",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Toggle commit selection (updates diff)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  (/)       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cycle through individual commits"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Esc       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Return focus to diff"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Review Target Selector",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  Tab/S-Tab ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Switch Local / Pull Requests tab"),
        ]),
        Line::from(vec![
            Span::styled(
                "  j/k       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Move row"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Space     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle local commit selection (no-op on PR tab)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Enter     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Open selected target or load more"),
        ]),
        Line::from(vec![
            Span::styled(
                "  /         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Local filter for current tab"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Esc/q     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Quit / return"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Submit Action Picker (PR mode)",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  j/k       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Navigate review events"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Enter     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Submit the selected event"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Esc       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cancel"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "File Tree",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  Space     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle expand directory"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Enter     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Expand dir / Jump to file"),
        ]),
        Line::from(vec![
            Span::styled(
                "  o         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Expand all directories"),
        ]),
        Line::from(vec![
            Span::styled(
                "  O         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Collapse all directories"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Comment Navigator",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  j/k       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Navigate comments"),
        ]),
        Line::from(vec![
            Span::styled(
                "  h/l       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Scroll comment rows left/right"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Enter     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Jump to selected comment"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Review Actions",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  r         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle file reviewed"),
        ]),
        Line::from(vec![
            Span::styled(
                "  c         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Add line comment"),
        ]),
        Line::from(vec![
            Span::styled(
                "  C         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Add file comment"),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}c        ", app.leader_key),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Add review comment"),
        ]),
        Line::from(vec![
            Span::styled(
                "  i         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Edit comment at cursor"),
        ]),
        Line::from(vec![
            Span::styled(
                "  dd        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Delete comment at cursor"),
        ]),
        Line::from(vec![
            Span::styled(
                "  y         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Yank: mouse selection if any, else review to clipboard"),
        ]),
        Line::from(vec![
            Span::styled(
                "  v/V       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Enter visual mode for range comments"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Visual Mode",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  j/k       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Extend selection down/up"),
        ]),
        Line::from(vec![
            Span::styled(
                "  c/Enter   ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Create comment for selected range"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Esc/v/V   ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cancel visual selection"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Comment Mode",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  Tab/S-Tab ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cycle comment type next/previous"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Enter     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Save comment"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl-S    ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Save comment"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Shift-Enter/Ctrl-J",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Insert newline"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl-A/E  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Line start/end"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl/Alt-Left/Right",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Word left/right"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Cmd-Left/Right",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Line start/end (macOS)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Esc/Ctrl-C",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cancel"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Commands",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  :{N}      ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Jump to new-side line N in current file"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :o{N}     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Jump to old-side line N in current file (matches deletions)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Tab       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Complete or cycle command names in the : prompt"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :w        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Save review session"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :e        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(
                "Reload diff files and comments (in PR mode: refetch PR; may switch session)",
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  :edit     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Open focused file in $EDITOR"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :clip     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Copy review to clipboard"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :set wrap ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Enable line wrap in diff view"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :set wrap!",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle line wrap in diff view"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :stage    ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Stage reviewed files"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :diff     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle unified/side-by-side diff view"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :focus    ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "Toggle single-file view (alias `:f`, {}f)",
                app.leader_key
            )),
        ]),
        Line::from(vec![
            Span::styled(
                "  :targets  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Open the review target selector"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :commits  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Open the selector on Local (commits, staged/unstaged)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :prs      ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Open the selector on Pull Requests"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :comments unresolved",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Show unresolved remote comments (PR mode, default)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :comments all",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Show all remote comments incl. resolved/outdated"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :comments hide",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Hide remote comments in PR mode"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :submit       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(
                "Pick a review event from a menu (Comment / Approve / Request changes / Draft)",
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  :submit comment",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Submit a COMMENT review (skips the picker, shows confirm modal)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :submit approve",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Submit an APPROVE review to GitHub"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :submit request-changes",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Submit a REQUEST_CHANGES review to GitHub"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :submit draft",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Push a pending (draft) review to GitHub"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :set commits",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Show inline commit selector"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :set nocommits",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Hide inline commit selector"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :set commits!",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Toggle inline commit selector"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :clear    ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Clear all comments and reviewed marks"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :clearc   ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Clear comments only"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :q        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Quit"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :wq       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Save and quit"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :version  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Show tuicr version"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :update   ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Check for updates"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  ?         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle this help"),
        ]),
    ];

    // Update help state with total lines and viewport height
    let total_lines = help_text.len();
    let viewport_height = inner.height as usize;
    app.help_state.total_lines = total_lines;
    app.help_state.viewport_height = viewport_height;

    // Calculate if we can scroll
    let can_scroll_up = app.help_state.scroll_offset > 0;
    let can_scroll_down = app.help_state.scroll_offset + viewport_height < total_lines;

    // Apply scroll offset
    let visible_lines: Vec<Line> = help_text
        .into_iter()
        .skip(app.help_state.scroll_offset)
        .take(viewport_height)
        .collect();

    let paragraph = Paragraph::new(visible_lines).style(styles::popup_style(theme));
    frame.render_widget(paragraph, inner);

    // Render scroll indicators
    let indicator_style = styles::help_indicator_style(theme);

    if can_scroll_up {
        let up_indicator = Paragraph::new(Line::from(Span::styled("▲ more", indicator_style)));
        let up_area = Rect {
            x: inner.x + inner.width.saturating_sub(8),
            y: inner.y,
            width: 7,
            height: 1,
        };
        frame.render_widget(up_indicator, up_area);
    }

    if can_scroll_down {
        let down_indicator = Paragraph::new(Line::from(Span::styled("▼ more", indicator_style)));
        let down_area = Rect {
            x: inner.x + inner.width.saturating_sub(8),
            y: inner.y + inner.height.saturating_sub(1),
            width: 7,
            height: 1,
        };
        frame.render_widget(down_indicator, down_area);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}
