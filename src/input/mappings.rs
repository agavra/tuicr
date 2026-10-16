use std::collections::VecDeque;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

const MAPPING_TIMEOUT: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapMode {
    Normal,
    Visual,
    Insert,
    CommandLine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Key {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl Key {
    fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        let mut modifiers = modifiers
            & (KeyModifiers::SHIFT
                | KeyModifiers::CONTROL
                | KeyModifiers::ALT
                | KeyModifiers::SUPER);
        let code = match code {
            KeyCode::Char(c) if c.is_ascii_uppercase() => {
                modifiers.insert(KeyModifiers::SHIFT);
                code
            }
            KeyCode::Char(c)
                if c.is_ascii_lowercase() && modifiers.contains(KeyModifiers::SHIFT) =>
            {
                KeyCode::Char(c.to_ascii_uppercase())
            }
            KeyCode::Char(c) if !c.is_alphabetic() => {
                modifiers.remove(KeyModifiers::SHIFT);
                code
            }
            KeyCode::BackTab => {
                modifiers.remove(KeyModifiers::SHIFT);
                code
            }
            _ => code,
        };
        Self { code, modifiers }
    }

    fn to_event(self) -> KeyEvent {
        KeyEvent::new(self.code, self.modifiers)
    }

    fn to_release_event(self) -> KeyEvent {
        KeyEvent::new_with_kind(self.code, self.modifiers, KeyEventKind::Release)
    }
}

impl From<&KeyEvent> for Key {
    fn from(event: &KeyEvent) -> Self {
        Self::new(event.code, event.modifiers)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Mapping {
    mode: MapMode,
    lhs: Vec<Key>,
    rhs: Vec<Key>,
    nowait: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyMappings {
    mappings: Vec<Mapping>,
}

enum Resolution<'a> {
    Expand(&'a Mapping),
    Ambiguous(Option<&'a Mapping>),
    Unmapped,
}

impl KeyMappings {
    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }

    fn insert(&mut self, mapping: Mapping) {
        self.mappings
            .retain(|existing| existing.mode != mapping.mode || existing.lhs != mapping.lhs);
        self.mappings.push(mapping);
    }

    fn resolve(&self, mode: MapMode, typed: &[Key]) -> Resolution<'_> {
        let mut exact: Option<&Mapping> = None;
        let mut longer = false;
        for mapping in self.mappings.iter().filter(|mapping| mapping.mode == mode) {
            if mapping.lhs.len() > typed.len() {
                longer |= mapping.lhs.starts_with(typed);
            } else if typed.starts_with(&mapping.lhs)
                && exact.is_none_or(|best| mapping.lhs.len() > best.lhs.len())
            {
                exact = Some(mapping);
            }
        }
        match exact {
            Some(mapping) if !longer || mapping.nowait => Resolution::Expand(mapping),
            exact if longer => Resolution::Ambiguous(exact),
            _ => Resolution::Unmapped,
        }
    }
}

pub fn load(path: &Path, leader: char) -> (KeyMappings, Vec<String>) {
    match fs::read_to_string(path) {
        Ok(source) => parse(&source, leader),
        Err(err) if err.kind() == ErrorKind::NotFound => (KeyMappings::default(), Vec::new()),
        Err(err) => (
            KeyMappings::default(),
            vec![format!("Warning: Failed to read {}: {err}", path.display())],
        ),
    }
}

pub fn parse(source: &str, leader: char) -> (KeyMappings, Vec<String>) {
    let mut mappings = KeyMappings::default();
    let mut warnings = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('"') {
            continue;
        }
        if let Err(reason) = parse_line(line, leader, &mut mappings) {
            warnings.push(format!(
                "Warning: mappings.vim line {}: {reason}; ignoring it",
                index + 1
            ));
        }
    }
    (mappings, warnings)
}

fn parse_line(line: &str, leader: char, mappings: &mut KeyMappings) -> Result<(), String> {
    let (command, mut rest) = split_word(line);
    let modes = command_modes(command)?;
    let mut nowait = false;
    loop {
        let (argument, after) = split_word(rest);
        match argument.to_ascii_lowercase().as_str() {
            "<nowait>" => nowait = true,
            "<silent>" | "<unique>" => {}
            _ => break,
        }
        rest = after;
    }
    let (lhs, rhs) = split_word(rest);
    if rhs.is_empty() {
        return Err(format!("'{command}' needs the keys and what they map to"));
    }
    let lhs = parse_keys(lhs, leader)?;
    if lhs.is_empty() {
        return Err("<Nop> cannot be mapped".to_string());
    }
    let rhs = parse_keys(rhs, leader)?;
    for &mode in modes {
        mappings.insert(Mapping {
            mode,
            lhs: lhs.clone(),
            rhs: rhs.clone(),
            nowait,
        });
    }
    Ok(())
}

fn command_modes(command: &str) -> Result<&'static [MapMode], String> {
    match command {
        "noremap" => Ok(&[MapMode::Normal, MapMode::Visual]),
        "nnoremap" => Ok(&[MapMode::Normal]),
        "vnoremap" | "xnoremap" => Ok(&[MapMode::Visual]),
        "inoremap" => Ok(&[MapMode::Insert]),
        "cnoremap" => Ok(&[MapMode::CommandLine]),
        "noremap!" => Ok(&[MapMode::Insert, MapMode::CommandLine]),
        "map" | "nmap" | "vmap" | "xmap" | "imap" | "cmap" | "map!" => Err(format!(
            "'{command}' is recursive, only the noremap commands are supported"
        )),
        _ => Err(format!("unknown command '{command}'")),
    }
}

fn split_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    match text.find(char::is_whitespace) {
        Some(end) => (&text[..end], text[end..].trim()),
        None => (text, ""),
    }
}

fn parse_keys(text: &str, leader: char) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(end) = rest.find('>')
            && end > 1
            && !rest[1..end].contains(|ch: char| ch == '<' || ch.is_whitespace())
        {
            let name = &rest[1..end];
            if !name.eq_ignore_ascii_case("nop") {
                let key = parse_special_key(name, leader)
                    .ok_or_else(|| format!("unknown key <{name}>"))?;
                keys.push(key);
            }
            rest = &rest[end + 1..];
        } else {
            keys.push(Key::new(KeyCode::Char(c), KeyModifiers::NONE));
            rest = &rest[c.len_utf8()..];
        }
    }
    Ok(keys)
}

fn parse_special_key(name: &str, leader: char) -> Option<Key> {
    if name.eq_ignore_ascii_case("leader") {
        return Some(Key::new(KeyCode::Char(leader), KeyModifiers::NONE));
    }
    let mut modifiers = KeyModifiers::NONE;
    let mut rest = name;
    while let Some((prefix, key)) = rest.split_once('-')
        && !key.is_empty()
        && let Some(modifier) = parse_modifier(prefix)
    {
        modifiers |= modifier;
        rest = key;
    }
    let mut chars = rest.chars();
    let code = match (chars.next(), chars.next()) {
        (Some(_), None) if modifiers.is_empty() => return None,
        (Some(c), None)
            if modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::SHIFT) =>
        {
            KeyCode::Char(c.to_ascii_lowercase())
        }
        (Some(c), None) => KeyCode::Char(c),
        _ => named_key(rest)?,
    };
    let code = match code {
        KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        code => code,
    };
    Some(Key::new(code, modifiers))
}

fn parse_modifier(prefix: &str) -> Option<KeyModifiers> {
    match prefix.to_ascii_lowercase().as_str() {
        "c" => Some(KeyModifiers::CONTROL),
        "s" => Some(KeyModifiers::SHIFT),
        "a" | "m" => Some(KeyModifiers::ALT),
        "d" => Some(KeyModifiers::SUPER),
        _ => None,
    }
}

fn named_key(name: &str) -> Option<KeyCode> {
    let code = match name.to_ascii_lowercase().as_str() {
        "cr" | "return" | "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "bs" | "backspace" => KeyCode::Backspace,
        "del" | "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "space" => KeyCode::Char(' '),
        "lt" => KeyCode::Char('<'),
        "bslash" => KeyCode::Char('\\'),
        "bar" => KeyCode::Char('|'),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        other => {
            let number: u8 = other.strip_prefix('f')?.parse().ok()?;
            return (1..=12).contains(&number).then_some(KeyCode::F(number));
        }
    };
    Some(code)
}

#[derive(Debug)]
enum Entry {
    Key {
        event: KeyEvent,
        remap: bool,
        released: bool,
    },
    Other(Event),
}

impl Entry {
    fn into_event(self) -> Event {
        match self {
            Entry::Key { event, .. } => Event::Key(event),
            Entry::Other(event) => event,
        }
    }
}

fn physical_key(code: KeyCode) -> KeyCode {
    match code {
        KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
        KeyCode::BackTab => KeyCode::Tab,
        code => code,
    }
}

#[derive(Debug, Default)]
pub struct Typeahead {
    mappings: KeyMappings,
    entries: VecDeque<Entry>,
    last_key_at: Option<Instant>,
    held: Vec<(KeyCode, Vec<Key>)>,
}

impl Typeahead {
    pub fn new(mappings: KeyMappings) -> Self {
        Self {
            mappings,
            ..Self::default()
        }
    }

    pub fn is_active(&self) -> bool {
        !self.mappings.is_empty()
    }

    pub fn push(&mut self, event: Event, now: Instant) {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Release => self.release(key),
            Event::Key(event) => {
                self.last_key_at = Some(now);
                self.entries.push_back(Entry::Key {
                    event,
                    remap: true,
                    released: false,
                });
            }
            event => self.entries.push_back(Entry::Other(event)),
        }
    }

    fn release(&mut self, key: KeyEvent) {
        let code = physical_key(key.code);
        let pending = self.entries.iter_mut().rev().find_map(|entry| match entry {
            Entry::Key {
                event,
                remap: true,
                released,
            } if !*released && physical_key(event.code) == code => Some(released),
            _ => None,
        });
        if let Some(released) = pending {
            *released = true;
            return;
        }
        let releases = match self.held.iter().position(|(held, _)| *held == code) {
            Some(index) => self
                .held
                .swap_remove(index)
                .1
                .into_iter()
                .map(Key::to_release_event)
                .collect(),
            None => vec![key],
        };
        let at = self
            .entries
            .iter()
            .position(|entry| matches!(entry, Entry::Key { remap: true, .. }))
            .unwrap_or(self.entries.len());
        for (offset, release) in releases.into_iter().enumerate() {
            self.entries
                .insert(at + offset, Entry::Other(Event::Key(release)));
        }
    }

    fn hold(&mut self, code: KeyCode, rhs: Option<Vec<Key>>) {
        let code = physical_key(code);
        self.held.retain(|(held, _)| *held != code);
        if let Some(rhs) = rhs {
            self.held.push((code, rhs));
        }
    }

    fn pop_literal(&mut self) -> Option<Event> {
        let entry = self.entries.pop_front()?;
        if let Entry::Key {
            event,
            remap: true,
            released,
        } = &entry
        {
            self.hold(event.code, None);
            if *released {
                let release = Key::from(event).to_release_event();
                self.entries.push_front(Entry::Other(Event::Key(release)));
            }
        }
        Some(entry.into_event())
    }

    pub fn pop(&mut self, mode: Option<MapMode>, now: Instant) -> Option<Event> {
        loop {
            let remappable = matches!(self.entries.front()?, Entry::Key { remap: true, .. });
            let Some(mode) = mode.filter(|_| remappable) else {
                return self.pop_literal();
            };
            let typed: Vec<Key> = self
                .entries
                .iter()
                .map_while(|entry| match entry {
                    Entry::Key {
                        event, remap: true, ..
                    } => Some(Key::from(event)),
                    _ => None,
                })
                .collect();
            let complete = typed.len() < self.entries.len()
                || self
                    .last_key_at
                    .is_some_and(|at| now.saturating_duration_since(at) >= MAPPING_TIMEOUT);
            let mapping = match self.mappings.resolve(mode, &typed) {
                Resolution::Expand(mapping) => mapping,
                Resolution::Ambiguous(Some(mapping)) if complete => mapping,
                Resolution::Ambiguous(_) if !complete => return None,
                _ => return self.pop_literal(),
            };
            let (consumed, rhs) = (mapping.lhs.len(), mapping.rhs.clone());
            let trigger = self.entries.drain(..consumed).next_back();
            let mut released_early = false;
            if let Some(Entry::Key {
                event, released, ..
            }) = trigger
            {
                self.hold(event.code, (!released).then(|| rhs.clone()));
                released_early = released;
            }
            let mut expansion: Vec<Entry> = rhs
                .iter()
                .map(|key| Entry::Key {
                    event: key.to_event(),
                    remap: false,
                    released: false,
                })
                .collect();
            if released_early {
                expansion.extend(
                    rhs.iter()
                        .map(|key| Entry::Other(Event::Key(key.to_release_event()))),
                );
            }
            for entry in expansion.into_iter().rev() {
                self.entries.push_front(entry);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEADER: char = ';';

    fn plain(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn typeahead(source: &str) -> Typeahead {
        let (mappings, warnings) = parse(source, LEADER);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        Typeahead::new(mappings)
    }

    fn type_keys(typeahead: &mut Typeahead, keys: &[KeyEvent], now: Instant) {
        for key in keys {
            typeahead.push(Event::Key(*key), now);
        }
    }

    fn drain(typeahead: &mut Typeahead, mode: MapMode, now: Instant) -> Vec<Event> {
        std::iter::from_fn(|| typeahead.pop(Some(mode), now)).collect()
    }

    fn keys(events: &[KeyEvent]) -> Vec<Event> {
        events.iter().copied().map(Event::Key).collect()
    }

    fn warnings_for(source: &str) -> Vec<String> {
        parse(source, LEADER).1
    }

    #[test]
    fn should_expand_ctrl_mapping_into_leader_sequence() {
        let mut typeahead = typeahead("nnoremap <C-h> <leader>h");
        let now = Instant::now();
        type_keys(&mut typeahead, &[ctrl('h')], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[plain(';'), plain('h')])
        );
    }

    #[test]
    fn should_match_uppercase_ctrl_notation_against_lowercase_ctrl_key() {
        let mut typeahead = typeahead("nnoremap <C-H> x");
        let now = Instant::now();
        type_keys(&mut typeahead, &[ctrl('h')], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[plain('x')])
        );
    }

    #[test]
    fn should_not_remap_the_right_hand_side() {
        let mut typeahead = typeahead("nnoremap a b\nnnoremap b c");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('a')], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[plain('b')])
        );
    }

    #[test]
    fn should_pass_unmapped_keys_through_untouched() {
        let mut typeahead = typeahead("nnoremap a b");
        let now = Instant::now();
        let repeat =
            KeyEvent::new_with_kind(KeyCode::Char('x'), KeyModifiers::NONE, KeyEventKind::Repeat);
        type_keys(&mut typeahead, &[repeat], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            vec![Event::Key(repeat)]
        );
    }

    #[test]
    fn should_only_apply_mapping_in_its_mode() {
        let mut typeahead = typeahead("nnoremap a b");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('a')], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Insert, now),
            keys(&[plain('a')])
        );
    }

    #[test]
    fn should_pass_keys_through_when_mode_has_no_mappings() {
        let mut typeahead = typeahead("nnoremap a b");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('a')], now);
        assert_eq!(typeahead.pop(None, now), Some(Event::Key(plain('a'))));
    }

    #[test]
    fn should_apply_noremap_to_normal_and_visual_modes() {
        let (mappings, _) = parse("noremap a b", LEADER);
        for (mode, expected) in [
            (MapMode::Normal, 'b'),
            (MapMode::Visual, 'b'),
            (MapMode::Insert, 'a'),
            (MapMode::CommandLine, 'a'),
        ] {
            let mut typeahead = Typeahead::new(mappings.clone());
            let now = Instant::now();
            type_keys(&mut typeahead, &[plain('a')], now);
            assert_eq!(
                drain(&mut typeahead, mode, now),
                keys(&[plain(expected)]),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn should_wait_for_the_next_key_when_a_longer_mapping_could_match() {
        let mut typeahead = typeahead("nnoremap a y\nnnoremap ab x");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('a')], now);
        assert_eq!(typeahead.pop(Some(MapMode::Normal), now), None);
        type_keys(&mut typeahead, &[plain('b')], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[plain('x')])
        );
    }

    #[test]
    fn should_fall_back_to_the_shorter_mapping_after_the_timeout() {
        let mut typeahead = typeahead("nnoremap a y\nnnoremap ab x");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('a')], now);
        assert_eq!(typeahead.pop(Some(MapMode::Normal), now), None);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now + MAPPING_TIMEOUT),
            keys(&[plain('y')])
        );
    }

    #[test]
    fn should_release_an_unfinished_sequence_literally_after_the_timeout() {
        let mut typeahead = typeahead("nnoremap ab x");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('a')], now);
        assert_eq!(typeahead.pop(Some(MapMode::Normal), now), None);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now + MAPPING_TIMEOUT),
            keys(&[plain('a')])
        );
    }

    #[test]
    fn should_reexamine_the_remaining_keys_after_a_failed_sequence() {
        let mut typeahead = typeahead("nnoremap ab x");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('a'), plain('a'), plain('b')], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[plain('a'), plain('x')])
        );
    }

    #[test]
    fn should_not_wait_for_longer_mappings_with_nowait() {
        let mut typeahead = typeahead("nnoremap <nowait> a y\nnnoremap ab x");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('a')], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[plain('y')])
        );
    }

    #[test]
    fn should_swallow_keys_mapped_to_nop() {
        let mut typeahead = typeahead("nnoremap q <Nop>");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('q'), plain('j')], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[plain('j')])
        );
    }

    #[test]
    fn should_keep_events_in_order_when_a_non_key_event_interrupts_a_sequence() {
        let mut typeahead = typeahead("inoremap jk <Esc>");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('j')], now);
        typeahead.push(Event::Paste("text".to_string()), now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Insert, now),
            vec![Event::Key(plain('j')), Event::Paste("text".to_string())]
        );
    }

    #[test]
    fn should_let_a_later_mapping_replace_an_earlier_one() {
        let mut typeahead = typeahead("nnoremap a b\nnnoremap a c");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('a')], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[plain('c')])
        );
    }

    #[test]
    fn should_parse_special_key_names() {
        let mut typeahead =
            typeahead("nnoremap <S-Tab> <Space><lt><CR><Bar>\nnnoremap G <Esc><F2><PageDown>");
        let now = Instant::now();
        type_keys(
            &mut typeahead,
            &[
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT),
            ],
            now,
        );
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[
                plain(' '),
                plain('<'),
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                plain('|'),
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
            ])
        );
    }

    #[test]
    fn should_type_command_line_text_from_the_right_hand_side() {
        let mut typeahead = typeahead("nnoremap <leader>S :submit<CR>");
        let now = Instant::now();
        type_keys(
            &mut typeahead,
            &[
                plain(';'),
                KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT),
            ],
            now,
        );
        let typed: Vec<Event> = ":submit"
            .chars()
            .map(|c| Event::Key(plain(c)))
            .chain([Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))])
            .collect();
        assert_eq!(drain(&mut typeahead, MapMode::Normal, now), typed);
    }

    #[test]
    fn should_skip_comments_and_blank_lines() {
        assert!(warnings_for("\" a comment\n\n   \nnnoremap a b\n").is_empty());
    }

    #[test]
    fn should_warn_about_recursive_mappings_with_the_line_number() {
        assert_eq!(
            warnings_for("nnoremap a b\nnmap c d"),
            vec![
                "Warning: mappings.vim line 2: 'nmap' is recursive, only the noremap commands are supported; ignoring it"
                    .to_string()
            ]
        );
    }

    #[test]
    fn should_warn_about_unknown_key_names() {
        assert_eq!(
            warnings_for("nnoremap <C-hh> x"),
            vec!["Warning: mappings.vim line 1: unknown key <C-hh>; ignoring it".to_string()]
        );
    }

    #[test]
    fn should_warn_about_mappings_without_a_right_hand_side() {
        assert_eq!(
            warnings_for("nnoremap a"),
            vec![
                "Warning: mappings.vim line 1: 'nnoremap' needs the keys and what they map to; ignoring it"
                    .to_string()
            ]
        );
    }

    #[test]
    fn should_only_be_active_with_mappings() {
        assert!(typeahead("nnoremap a b").is_active());
        assert!(!Typeahead::default().is_active());
    }

    fn release(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Release)
    }

    #[test]
    fn should_release_the_mapped_keys_when_the_trigger_is_released() {
        let mut typeahead = typeahead("nnoremap x j");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('x')], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[plain('j')])
        );
        typeahead.push(
            Event::Key(release(KeyCode::Char('x'), KeyModifiers::NONE)),
            now,
        );
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[release(KeyCode::Char('j'), KeyModifiers::NONE)])
        );
    }

    #[test]
    fn should_translate_the_release_even_after_the_modifier_was_let_go() {
        let mut typeahead = typeahead("nnoremap <C-n> j");
        let now = Instant::now();
        type_keys(&mut typeahead, &[ctrl('n')], now);
        drain(&mut typeahead, MapMode::Normal, now);
        typeahead.push(
            Event::Key(release(KeyCode::Char('n'), KeyModifiers::NONE)),
            now,
        );
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[release(KeyCode::Char('j'), KeyModifiers::NONE)])
        );
    }

    #[test]
    fn should_pass_releases_of_unmapped_keys_through() {
        let mut typeahead = typeahead("nnoremap x j");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('a')], now);
        drain(&mut typeahead, MapMode::Normal, now);
        let released = release(KeyCode::Char('a'), KeyModifiers::NONE);
        typeahead.push(Event::Key(released), now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[released])
        );
    }

    #[test]
    fn should_release_the_mapped_keys_after_the_timeout_when_released_early() {
        let mut typeahead = typeahead("nnoremap x j\nnnoremap xx k");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('x')], now);
        typeahead.push(
            Event::Key(release(KeyCode::Char('x'), KeyModifiers::NONE)),
            now,
        );
        assert_eq!(drain(&mut typeahead, MapMode::Normal, now), Vec::new());
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now + MAPPING_TIMEOUT),
            keys(&[plain('j'), release(KeyCode::Char('j'), KeyModifiers::NONE)])
        );
    }

    #[test]
    fn should_release_a_literal_key_right_after_its_press_when_released_early() {
        let mut typeahead = typeahead("nnoremap ab x");
        let now = Instant::now();
        let release_a = release(KeyCode::Char('a'), KeyModifiers::NONE);
        type_keys(&mut typeahead, &[plain('a')], now);
        typeahead.push(Event::Key(release_a), now);
        assert_eq!(drain(&mut typeahead, MapMode::Normal, now), Vec::new());
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now + MAPPING_TIMEOUT),
            keys(&[plain('a'), release_a])
        );
    }

    #[test]
    fn should_drop_the_releases_of_keys_consumed_by_a_mapping() {
        let mut typeahead = typeahead("nnoremap ab x");
        let now = Instant::now();
        type_keys(&mut typeahead, &[plain('a')], now);
        typeahead.push(
            Event::Key(release(KeyCode::Char('a'), KeyModifiers::NONE)),
            now,
        );
        type_keys(&mut typeahead, &[plain('b')], now);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[plain('x')])
        );
        typeahead.push(
            Event::Key(release(KeyCode::Char('b'), KeyModifiers::NONE)),
            now,
        );
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, now),
            keys(&[release(KeyCode::Char('x'), KeyModifiers::NONE)])
        );
    }

    #[test]
    fn should_not_translate_later_releases_after_an_early_release() {
        let mut typeahead = typeahead("nnoremap a y\nnnoremap ab x");
        let now = Instant::now();
        let release_a = release(KeyCode::Char('a'), KeyModifiers::NONE);
        type_keys(&mut typeahead, &[plain('a')], now);
        typeahead.push(Event::Key(release_a), now);
        let later = now + MAPPING_TIMEOUT;
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, later),
            keys(&[plain('y'), release(KeyCode::Char('y'), KeyModifiers::NONE)])
        );
        type_keys(&mut typeahead, &[plain('a'), plain('b')], later);
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, later),
            keys(&[plain('x')])
        );
        typeahead.push(Event::Key(release_a), later);
        typeahead.push(
            Event::Key(release(KeyCode::Char('b'), KeyModifiers::NONE)),
            later,
        );
        assert_eq!(
            drain(&mut typeahead, MapMode::Normal, later),
            keys(&[release_a, release(KeyCode::Char('x'), KeyModifiers::NONE)])
        );
    }

    #[test]
    fn should_load_nothing_when_the_file_is_missing() {
        let (mappings, warnings) = load(Path::new("/nonexistent/tuicr/mappings.vim"), LEADER);
        assert!(mappings.is_empty());
        assert!(warnings.is_empty());
    }
}
