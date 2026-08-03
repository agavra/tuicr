use super::*;

impl App {
    /// Apply resolved `[generated]` settings and detect matching files.
    ///
    /// Called from startup after config load, because `App::build` runs before
    /// the config is applied and would otherwise detect against the defaults.
    pub fn apply_generated_config(&mut self, config: &GeneratedConfig) {
        self.collapse_generated = config.collapse();
        self.count_generated = config.count();
        // Annotations were built during `App::build`, before detection could
        // run, so any newly collapsed file is not reflected in them yet.
        if self.detect_generated_files() {
            self.rebuild_annotations();
        }
    }

    /// Whether `.gitattributes` has to be consulted at all.
    ///
    /// Both features off means the detected set stays empty, which is what
    /// makes the default configuration free: no repository is opened, and the
    /// render path's guards short-circuit before any lookup.
    fn generated_detection_enabled(&self) -> bool {
        self.collapse_generated || !self.count_generated
    }

    /// Repository root to resolve `.gitattributes` against, if there is one.
    ///
    /// PR mode's `root_path` is the synthetic `forge:host/owner/repo` identity
    /// rather than a directory, so it falls back to the local checkout the
    /// forge backend resolved — which is only set when the checkout matches
    /// the PR's target repository, so a foreign checkout can't mis-mark files.
    fn generated_attributes_root(&self) -> Option<PathBuf> {
        if self.vcs_info.root_path.is_absolute() {
            return Some(self.vcs_info.root_path.clone());
        }
        self.forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path())
    }

    /// Look up `.gitattributes` for every diff file not probed yet, returning
    /// whether that grew the generated set.
    ///
    /// Probed paths are remembered, so a repeat call over an unchanged file
    /// set costs one hash lookup per file and no libgit2 work. That memo is
    /// also the staleness test, which is what lets `rebuild_annotations` keep
    /// detection current without each of the fifteen `diff_files` assignments
    /// having to remember to trigger it.
    pub(in crate::app) fn detect_generated_files(&mut self) -> bool {
        if !self.generated_detection_enabled() {
            return false;
        }
        if self
            .diff_files
            .iter()
            .all(|file| self.generated_probed.contains(file.display_path()))
        {
            return false;
        }
        let Some(root) = self.generated_attributes_root() else {
            return false;
        };

        let unprobed: Vec<PathBuf> = self
            .diff_files
            .iter()
            .map(|file| file.display_path())
            .filter(|path| !self.generated_probed.contains(*path))
            .cloned()
            .collect();

        let detected = crate::profile::time("generated.detect", || {
            crate::generated::detect_generated(&root, &unprobed)
        });

        self.generated_probed.extend(unprobed);
        let changed = !detected.is_empty();
        self.generated_files.extend(detected);
        changed
    }

    /// Forget which paths were probed so the next detection re-reads
    /// `.gitattributes`. Called on explicit reload, so editing the attributes
    /// file takes effect without restarting tuicr.
    pub(in crate::app) fn invalidate_generated_detection(&mut self) {
        self.generated_probed.clear();
        self.generated_files.clear();
    }

    /// Whether to surface this path as code-generated.
    ///
    /// Gated on the same condition as detection rather than on the detected
    /// set being non-empty, so the decoration is a function of the current
    /// settings and not of what the session has detected in the past. The set
    /// is deliberately *not* discarded when the feature is switched off — that
    /// is what makes re-enabling free — so keying the decoration off it would
    /// make `:set generated` / `:set nogenerated` asymmetric: the labels, the
    /// dimmed tree rows, and the counter would all survive a toggle back off
    /// and never return to how the session started.
    #[inline]
    pub fn is_generated_file(&self, path: &Path) -> bool {
        self.generated_detection_enabled() && self.generated_files.contains(path)
    }

    /// Number of diff files surfaced as code-generated.
    pub fn generated_file_count(&self) -> usize {
        if !self.generated_detection_enabled() {
            return 0;
        }
        self.diff_files
            .iter()
            .filter(|file| self.generated_files.contains(file.display_path()))
            .count()
    }

    /// `(reviewed, total)` file counts for the review progress indicator.
    ///
    /// Generated files drop out of both halves when `[generated] count` is
    /// off — out of the numerator too, or marking one reviewed would push the
    /// count past its own total.
    pub fn review_progress(&self) -> (usize, usize) {
        if self.count_generated || self.generated_files.is_empty() {
            return (self.reviewed_count(), self.file_count());
        }
        let mut reviewed = 0;
        let mut total = 0;
        for file in &self.diff_files {
            let path = file.display_path();
            if self.generated_files.contains(path) {
                continue;
            }
            total += 1;
            if self.session.is_file_reviewed(path) {
                reviewed += 1;
            }
        }
        (reviewed, total)
    }

    /// Set the runtime collapse toggle, running detection if it was skipped
    /// while both features were off.
    pub fn set_collapse_generated(&mut self, collapse: bool) {
        self.collapse_generated = collapse;
        self.detect_generated_files();
        self.rebuild_annotations();

        let count = self.generated_file_count();
        if !collapse {
            self.set_message("Showing generated files");
        } else if count == 0 {
            self.set_warning("No files marked generated in .gitattributes");
        } else {
            self.set_message(format!("Collapsing {count} generated file(s)"));
        }
    }

    pub fn toggle_collapse_generated(&mut self) {
        self.set_collapse_generated(!self.collapse_generated);
    }
}
