use super::*;

impl App {
    /// Apply `[generated]` and rebuild the annotations made with startup defaults
    /// when settings or detected files change.
    pub fn apply_generated_config(&mut self, config: &GeneratedConfig) {
        let previous = (self.collapse_generated, self.count_generated);
        self.collapse_generated = config.collapse();
        self.count_generated = config.count();
        let settings_changed = previous != (self.collapse_generated, self.count_generated);
        if self.detect_generated_files() || settings_changed {
            self.rebuild_annotations();
        }
    }

    /// Whether `.gitattributes` has to be consulted at all.
    ///
    /// Skip attribute lookup when neither setting needs generated-file metadata.
    fn generated_detection_enabled(&self) -> bool {
        self.collapse_generated || !self.count_generated
    }

    /// Resolve attributes in the checkout, or the forge's verified local clone in PR mode.
    fn generated_attributes_root(&self) -> Option<PathBuf> {
        if self.vcs_info.root_path.is_absolute() {
            return Some(self.vcs_info.root_path.clone());
        }
        self.forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path())
    }

    /// Probe newly loaded paths once. Annotation rebuilds cover every diff assignment
    /// without repeating VCS work for unchanged file sets.
    pub(in crate::app) fn detect_generated_files(&mut self) -> bool {
        if !self.generated_detection_enabled() {
            return false;
        }
        let root = self.generated_attributes_root();
        let root_changed = self.generated_root != root;
        if root_changed {
            self.invalidate_generated_detection();
            self.generated_root = root.clone();
        }
        if self
            .diff_files
            .iter()
            .all(|file| self.generated_probed.contains(file.display_path()))
        {
            return root_changed;
        }
        let Some(root) = root else {
            return root_changed;
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
        let changed = root_changed || !detected.is_empty();
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

    /// Gate cached detection on current settings. Retain the cache while disabled
    /// so re-enabling needs no VCS lookup.
    #[inline]
    pub fn is_generated_file(&self, path: &Path) -> bool {
        self.generated_detection_enabled() && self.generated_files.contains(path)
    }

    /// Number of diff files surfaced as code-generated.
    pub fn generated_file_count(&self) -> usize {
        if !self.generated_detection_enabled() {
            return 0;
        }
        let filtered = self.file_filter_active();
        self.diff_files
            .iter()
            .filter(|file| {
                (!filtered || self.file_matches_patterns(file))
                    && self.generated_files.contains(file.display_path())
            })
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
        let filtered = self.file_filter_active();
        let mut reviewed = 0;
        let mut total = 0;
        // Same population as `file_count`: files surviving the `i`/`e`
        // patterns, so the fraction stays coherent while a filter is active.
        for file in &self.diff_files {
            let path = file.display_path();
            if self.generated_files.contains(path)
                || (filtered && !self.file_matches_patterns(file))
            {
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
