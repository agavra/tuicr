use super::*;

impl App {
    pub(crate) fn gap_size(&self, gap_id: &GapId) -> Option<u32> {
        let file = self.diff_files.get(gap_id.file_idx)?;

        if gap_id.hunk_idx == file.hunks.len() {
            // End-of-file gap
            let last_hunk = file.hunks.last()?;
            let start = last_hunk.new_start + last_hunk.new_count;
            let end = *self.file_line_count_cache.get(&gap_id.file_idx)?;
            if start > end {
                Some(0)
            } else {
                Some(end - start + 1)
            }
        } else {
            let hunk = file.hunks.get(gap_id.hunk_idx)?;
            let prev_hunk = if gap_id.hunk_idx > 0 {
                file.hunks.get(gap_id.hunk_idx - 1)
            } else {
                None
            };
            Some(calculate_gap(
                prev_hunk.map(|h| (&h.new_start, &h.new_count)),
                hunk.new_start,
            ))
        }
    }

    /// Return the gap (between-hunk or pre-first-hunk region) in `file_idx`
    /// whose line range on `side` contains `target_lineno`, if any. Used by
    /// `go_to_source_line` to auto-expand collapsed context when the user
    /// jumps to a line that's hidden behind an expander.
    pub(in crate::app) fn find_gap_containing_lineno(
        &self,
        file_idx: usize,
        target_lineno: u32,
        side: LineSide,
    ) -> Option<GapId> {
        let file = self.diff_files.get(file_idx)?;
        for hunk_idx in 0..file.hunks.len() {
            let hunk = &file.hunks[hunk_idx];
            let prev_hunk = if hunk_idx > 0 {
                Some(&file.hunks[hunk_idx - 1])
            } else {
                None
            };
            let (start, end) = match side {
                LineSide::New => match prev_hunk {
                    None => (1, hunk.new_start.saturating_sub(1)),
                    Some(p) => (p.new_start + p.new_count, hunk.new_start.saturating_sub(1)),
                },
                LineSide::Old => match prev_hunk {
                    None => (1, hunk.old_start.saturating_sub(1)),
                    Some(p) => (p.old_start + p.old_count, hunk.old_start.saturating_sub(1)),
                },
            };
            if start <= end && target_lineno >= start && target_lineno <= end {
                return Some(GapId { file_idx, hunk_idx });
            }
        }

        // Check the EOF gap (after the last hunk).
        if let Some(last) = file.hunks.last()
            && let Some(&total) = self.file_line_count_cache.get(&file_idx)
        {
            let (start, end) = match side {
                LineSide::New => (last.new_start + last.new_count, total),
                LineSide::Old => {
                    let delta = (last.new_start + last.new_count) as i64
                        - (last.old_start + last.old_count) as i64;
                    let new_start = last.new_start + last.new_count;
                    let old_start = (new_start as i64 - delta) as u32;
                    let old_end = (total as i64 - delta) as u32;
                    (old_start, old_end)
                }
            };
            if start <= end && target_lineno >= start && target_lineno <= end {
                return Some(GapId {
                    file_idx,
                    hunk_idx: file.hunks.len(),
                });
            }
        }
        None
    }

    /// Choose an `ExpandDirection` and line-count limit so that expanding
    /// `gap_id` reveals exactly the lines between the cursor and
    /// `target_lineno` (on `side`). Cursor above the gap → `Down` from the
    /// previous hunk; cursor at-or-below the gap → `Up` from the next hunk.
    ///
    /// `expand_gap` operates in new-side coordinates, so an old-side target
    /// is translated to new-side using the offset that holds across an
    /// unchanged-context gap: `new - old = hunk.new_start - hunk.old_start`
    /// where `hunk` is the hunk immediately *after* the gap.
    pub(in crate::app) fn expand_plan_to_reach(
        &self,
        gap_id: &GapId,
        target_lineno: u32,
        side: LineSide,
    ) -> (ExpandDirection, Option<usize>) {
        let Some(file) = self.diff_files.get(gap_id.file_idx) else {
            return (ExpandDirection::Both, None);
        };
        // EOF gap: no next hunk, only expand downward from the last hunk.
        let is_eof_gap = gap_id.hunk_idx == file.hunks.len();
        if is_eof_gap {
            let Some(last) = file.hunks.last() else {
                return (ExpandDirection::Both, None);
            };
            let gap_start_new = last.new_start + last.new_count;
            let offset_new_minus_old =
                (last.new_start + last.new_count) as i64 - (last.old_start + last.old_count) as i64;
            let target_new = match side {
                LineSide::New => target_lineno as i64,
                LineSide::Old => target_lineno as i64 + offset_new_minus_old,
            };
            let top_len = self.expanded_top.get(gap_id).map_or(0, |v| v.len()) as i64;
            let inner_start = gap_start_new as i64 + top_len;
            let limit = (target_new - inner_start + 1).max(0) as usize;
            return (ExpandDirection::Down, Some(limit));
        }

        let hunk = &file.hunks[gap_id.hunk_idx];
        let prev_hunk = if gap_id.hunk_idx > 0 {
            Some(&file.hunks[gap_id.hunk_idx - 1])
        } else {
            None
        };
        let gap_start_new = match prev_hunk {
            None => 1,
            Some(p) => p.new_start + p.new_count,
        };
        let gap_end_new = hunk.new_start.saturating_sub(1);
        // offset := new - old, constant across the unchanged context gap
        let offset_new_minus_old = hunk.new_start as i64 - hunk.old_start as i64;
        let target_new = match side {
            LineSide::New => target_lineno as i64,
            LineSide::Old => target_lineno as i64 + offset_new_minus_old,
        };

        let cursor_below_gap = self
            .find_hunk_header_annotation_idx(gap_id)
            .is_some_and(|h| self.diff_state.cursor_line >= h);

        if cursor_below_gap {
            let bot_len = self.expanded_bottom.get(gap_id).map_or(0, |v| v.len()) as i64;
            let inner_end = gap_end_new as i64 - bot_len;
            let limit = (inner_end - target_new + 1).max(0) as usize;
            (ExpandDirection::Up, Some(limit))
        } else {
            let top_len = self.expanded_top.get(gap_id).map_or(0, |v| v.len()) as i64;
            let inner_start = gap_start_new as i64 + top_len;
            let limit = (target_new - inner_start + 1).max(0) as usize;
            (ExpandDirection::Down, Some(limit))
        }
    }

    fn find_hunk_header_annotation_idx(&self, gap_id: &GapId) -> Option<usize> {
        self.line_annotations
            .iter()
            .enumerate()
            .find_map(|(idx, a)| match a {
                AnnotatedLine::HunkHeader { file_idx, hunk_idx }
                    if *file_idx == gap_id.file_idx && *hunk_idx == gap_id.hunk_idx =>
                {
                    Some(idx)
                }
                _ => None,
            })
    }

    /// Get the line boundaries (start_line, end_line) of a gap.
    fn gap_boundaries(&self, gap_id: &GapId) -> Option<(u32, u32)> {
        let file = self.diff_files.get(gap_id.file_idx)?;

        if gap_id.hunk_idx == file.hunks.len() {
            // End-of-file gap: starts after last hunk, ends at file end
            let last_hunk = file.hunks.last()?;
            let start = last_hunk.new_start + last_hunk.new_count;
            let end = *self.file_line_count_cache.get(&gap_id.file_idx)?;
            if start > end {
                None
            } else {
                Some((start, end))
            }
        } else {
            let hunk = file.hunks.get(gap_id.hunk_idx)?;
            let prev_hunk = if gap_id.hunk_idx > 0 {
                file.hunks.get(gap_id.hunk_idx - 1)
            } else {
                None
            };
            let (start, end) = match prev_hunk {
                None => (1, hunk.new_start.saturating_sub(1)),
                Some(prev) => (
                    prev.new_start + prev.new_count,
                    hunk.new_start.saturating_sub(1),
                ),
            };
            if start > end {
                None
            } else {
                Some((start, end))
            }
        }
    }

    /// Look up an expanded context line by sequential index across top + bottom.
    pub(in crate::app) fn get_expanded_line(
        &self,
        gap_id: &GapId,
        idx: usize,
    ) -> Option<&DiffLine> {
        let top = self.expanded_top.get(gap_id);
        let top_len = top.map_or(0, |v| v.len());
        if idx < top_len {
            top?.get(idx)
        } else {
            self.expanded_bottom.get(gap_id)?.get(idx - top_len)
        }
    }

    /// Expand a gap in the given direction.
    /// If `limit` is Some(n), expand up to n lines. If None, expand all remaining.
    pub fn expand_gap(
        &mut self,
        gap_id: GapId,
        direction: ExpandDirection,
        limit: Option<usize>,
    ) -> Result<()> {
        // Ensure file line count is cached for EOF gaps
        self.ensure_file_line_count_cached(gap_id.file_idx);

        let (gap_start, gap_end) = self
            .gap_boundaries(&gap_id)
            .ok_or_else(|| TuicrError::CorruptedSession(format!("Invalid gap: {:?}", gap_id)))?;

        let file = &self.diff_files[gap_id.file_idx];
        let old_path = file.old_path.clone();
        let new_path = file.new_path.clone();
        let file_status = file.status;

        let top_len = self.expanded_top.get(&gap_id).map_or(0, |v| v.len()) as u32;
        let bot_len = self.expanded_bottom.get(&gap_id).map_or(0, |v| v.len()) as u32;

        // The unexpanded region runs from (gap_start + top_len) to (gap_end - bot_len)
        let inner_start = gap_start + top_len;
        let inner_end = gap_end.saturating_sub(bot_len);

        if inner_start > inner_end {
            return Ok(()); // Fully expanded
        }

        // Compute the delta between new-side and old-side line numbers for this
        // gap. Expanded context is fetched in new-side coordinates; the old-side
        // number is `new_lineno - delta`.
        let delta = {
            let file = &self.diff_files[gap_id.file_idx];
            if gap_id.hunk_idx == 0 {
                0i64
            } else {
                let prev = &file.hunks[gap_id.hunk_idx - 1];
                let old_end = prev.old_start as i64 + prev.old_count as i64;
                let new_end = prev.new_start as i64 + prev.new_count as i64;
                new_end - old_end
            }
        };

        let fetch = |start: u32, end: u32| -> Result<Vec<DiffLine>> {
            let mut lines = self.context_provider().fetch_context_lines(
                old_path.as_ref(),
                new_path.as_ref(),
                file_status,
                start,
                end,
            )?;
            for line in &mut lines {
                if let Some(n) = line.new_lineno {
                    line.old_lineno = Some((n as i64 - delta) as u32);
                }
            }
            Ok(lines)
        };

        match direction {
            ExpandDirection::Down => {
                let n = limit.unwrap_or(usize::MAX) as u32;
                let fetch_end = inner_start.saturating_add(n - 1).min(inner_end);
                let new_lines = fetch(inner_start, fetch_end)?;
                self.expanded_top
                    .entry(gap_id.clone())
                    .or_default()
                    .extend(new_lines);
            }
            ExpandDirection::Up => {
                let n = limit.unwrap_or(usize::MAX) as u32;
                let fetch_start = inner_end.saturating_sub(n - 1).max(inner_start);
                let new_lines = fetch(fetch_start, inner_end)?;
                // Prepend: new lines go before existing bottom lines
                let existing = self.expanded_bottom.remove(&gap_id).unwrap_or_default();
                let mut combined = new_lines;
                combined.extend(existing);
                self.expanded_bottom.insert(gap_id.clone(), combined);
            }
            ExpandDirection::Both => {
                // Fetch everything remaining
                let new_lines = fetch(inner_start, inner_end)?;
                self.expanded_top
                    .entry(gap_id.clone())
                    .or_default()
                    .extend(new_lines);
            }
        }

        self.rebuild_annotations();
        Ok(())
    }

    /// Resolve the right `ContextProvider` for the current diff source.
    /// In PR mode (with a forge backend present), expansion goes through the
    /// forge; otherwise it goes through the local VCS backend.
    pub(in crate::app) fn ref_commit(&self) -> Option<&str> {
        match &self.diff_source {
            DiffSource::CommitRange(commits) => {
                // When the inline commit selector narrows to a subrange,
                // review_commits is newest-first so index `start` is the
                // newest selected commit — that's the snapshot to read from.
                if let Some((start, _)) = self.commit_selection_range {
                    self.review_commits
                        .get(start)
                        .map(|c| c.id.as_str())
                        .or_else(|| commits.last().map(|s| s.as_str()))
                } else {
                    commits.last().map(|s| s.as_str())
                }
            }
            _ => None,
        }
    }

    pub(in crate::app) fn context_provider(&self) -> Box<dyn ContextProvider + '_> {
        if let (DiffSource::PullRequest(pr), Some(backend)) =
            (&self.diff_source, self.forge_backend.as_ref())
        {
            Box::new(ForgeContextProvider {
                forge: backend.as_ref(),
                repository: pr.key.repository.clone(),
                base_sha: pr.base_sha.clone(),
                head_sha: pr.key.head_sha.clone(),
            })
        } else {
            Box::new(VcsContextProvider {
                vcs: self.vcs.as_ref(),
                ref_commit: self.ref_commit().map(|s| s.to_string()),
            })
        }
    }

    /// Collapse an expanded gap
    pub fn collapse_gap(&mut self, gap_id: GapId) {
        self.expanded_top.remove(&gap_id);
        self.expanded_bottom.remove(&gap_id);
        self.rebuild_annotations();
    }

    /// Clear all expanded gaps (called when reloading diffs)
    pub fn clear_expanded_gaps(&mut self) {
        self.expanded_top.clear();
        self.expanded_bottom.clear();
        self.file_line_count_cache.clear();
    }

    pub(in crate::app) fn eof_gap_enabled(&self) -> bool {
        matches!(
            self.diff_source,
            DiffSource::WorkingTree
                | DiffSource::Unstaged
                | DiffSource::StagedAndUnstaged
                | DiffSource::StagedUnstagedAndCommits(_)
                | DiffSource::CommitRange(_)
                | DiffSource::PullRequest(_)
        )
    }

    /// What the cursor is on in a gap region
    pub fn get_gap_at_cursor(&self) -> Option<GapCursorHit> {
        let target = self.diff_state.cursor_line;
        match self.line_annotations.get(target) {
            Some(AnnotatedLine::Expander { gap_id, direction }) => {
                Some(GapCursorHit::Expander(gap_id.clone(), *direction))
            }
            Some(AnnotatedLine::HiddenLines { gap_id, .. }) => {
                Some(GapCursorHit::HiddenLines(gap_id.clone()))
            }
            Some(AnnotatedLine::ExpandedContext { gap_id, .. }) => {
                Some(GapCursorHit::ExpandedContent(gap_id.clone()))
            }
            _ => None,
        }
    }
}
