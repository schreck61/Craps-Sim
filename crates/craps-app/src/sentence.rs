// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The Scenario Sentence codec: the whole configuration as one line of prose.
//!
//! [`render_spans`] produces the clickable bold fragments (with per-fragment
//! stale diffing against the provenance config), [`render_text`] the plain
//! text that ⌘C copies, and [`parse`] reconstructs a [`SimConfig`] from
//! pasted text. The sentence is the save format, so the codec is lossless:
//! `parse(&render_text(&cfg)).unwrap() == cfg` for every configuration.
//!
//! One grammar note beyond the spec sketch: the odds policy matters even
//! when odds are not taken (the Explorer always uses it), so a non-default
//! policy with `take_odds` off renders parenthesized — `(2× odds)` reads
//! "configured but not taken". Absence means the default 3-4-5× not taken,
//! and `0× odds` encodes the degenerate `OddsPolicy::None`.

#![cfg_attr(not(test), allow(dead_code))]

use craps_engine::{
    hard_index, place_index, BetSelection, OddsPolicy, Progression, HARD_NUMS, PLACE_NUMS,
};

use crate::config::SimConfig;
use crate::ui::numerals::{money_text, MINUS, THIN_SPACE};

/// Which control a sentence fragment focuses when clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentId {
    Budget,
    Tables,
    Bets,
    Progression,
    Quit,
    Horizon,
    Field12,
    ComeOddsComeout,
    Props,
    TableMax,
    Engine,
}

/// One rendered piece of the sentence. `frag: Some(..)` marks a bold,
/// clickable fragment; `None` is connective tissue (" at ", " · ").
/// `stale` is true when the fragment's underlying fields differ from the
/// provenance config (renders with an amber underline).
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub frag: Option<FragmentId>,
    pub stale: bool,
}

/// The exact bidirectional progression vocabulary.
const PROGRESSION_WORDS: [(Progression, &str); 12] = [
    (Progression::Flat, "flat"),
    (Progression::FullPress, "full press"),
    (Progression::HalfPress, "half press"),
    (Progression::PressAndPull, "press and pull"),
    (Progression::Paroli3, "paroli 3"),
    (Progression::S1326, "1-3-2-6"),
    (Progression::Martingale, "martingale"),
    (Progression::GrandMartingale, "grand martingale"),
    (Progression::DAlembert, "d'alembert"),
    (Progression::ReverseDAlembert, "reverse d'alembert"),
    (Progression::Fibonacci, "fibonacci"),
    (Progression::OscarsGrind, "oscar's grind"),
];

/// Render the sentence. `provenance` is the config the on-screen results
/// were computed with; when Some, each fragment is diffed field-by-field
/// against it to set `stale`.
pub fn render_spans(cfg: &SimConfig, provenance: Option<&SimConfig>) -> Vec<Span> {
    let frag = |id: FragmentId, text: String| Span {
        text,
        frag: Some(id),
        stale: provenance.is_some_and(|p| fragment_differs(id, cfg, p)),
    };
    let glue = |text: &str| Span {
        text: text.to_owned(),
        frag: None,
        stale: false,
    };

    let mut spans = vec![
        frag(FragmentId::Budget, money_text(cfg.budget_cents, false)),
        glue(" at "),
        frag(FragmentId::Tables, tables_text(cfg)),
    ];

    let mut tail: Vec<(FragmentId, String)> = vec![
        (FragmentId::Bets, bets_text(cfg)),
        (
            FragmentId::Progression,
            progression_word(cfg.sel.progression).to_owned(),
        ),
        (FragmentId::Quit, quit_text(cfg)),
        (
            FragmentId::Horizon,
            format!(
                "{} h at {} rolls/h",
                trim_f64(cfg.target_hours),
                cfg.rolls_per_hour
            ),
        ),
        (
            FragmentId::Field12,
            format!(
                "field 12 pays {}",
                if cfg.field_12_triple {
                    "triple"
                } else {
                    "double"
                }
            ),
        ),
    ];
    if cfg.come_odds_work_on_comeout {
        tail.push((
            FragmentId::ComeOddsComeout,
            "come odds work on come-out".to_owned(),
        ));
    }
    tail.push((
        FragmentId::Props,
        format!("props {}", money_text(cfg.prop_bet_cents, false)),
    ));
    tail.push((
        FragmentId::TableMax,
        format!("max {}× min", cfg.table_max_mult),
    ));
    tail.push((FragmentId::Engine, engine_text(cfg)));

    for (id, text) in tail {
        spans.push(glue(" · "));
        spans.push(frag(id, text));
    }
    spans
}

/// The plain-text sentence (spans concatenated) — what ⌘C copies.
pub fn render_text(cfg: &SimConfig) -> String {
    render_spans(cfg, None)
        .into_iter()
        .map(|s| s.text)
        .collect()
}

/// Reconstruct a SimConfig from a sentence. Tolerant of: ASCII hyphen for
/// the true minus, 'x' for '×', plain spaces for thin spaces, case
/// differences, and arbitrary whitespace runs. Unknown trailing text is an
/// error naming the offending token.
pub fn parse(text: &str) -> Result<SimConfig, String> {
    let norm = normalize(text);
    if norm.is_empty() {
        return Err("empty sentence".to_owned());
    }
    let chunks: Vec<&str> = norm.split('·').map(str::trim).collect();
    let mut at = 0usize;

    let (budget_cents, table_mins_cents) =
        parse_budget_tables(take(&chunks, &mut at, "budget and tables")?)?;
    let (mut sel, odds_policy) = parse_bets(take(&chunks, &mut at, "bets")?)?;
    sel.progression = parse_progression(take(&chunks, &mut at, "progression")?)?;
    let quit_mult = parse_quit(take(&chunks, &mut at, "quit rule")?)?;
    let (target_hours, rolls_per_hour) = parse_horizon(take(&chunks, &mut at, "horizon")?)?;
    let field_12_triple = parse_field12(take(&chunks, &mut at, "field-12 rule")?)?;
    let mut come_odds_work_on_comeout = false;
    if chunks.get(at).is_some_and(|c| c.starts_with("come odds")) {
        parse_comeout(chunks[at])?;
        come_odds_work_on_comeout = true;
        at += 1;
    }
    let prop_bet_cents = parse_props(take(&chunks, &mut at, "prop stake")?)?;
    let table_max_mult = parse_table_max(take(&chunks, &mut at, "table max")?)?;
    let (sessions, max_rolls, confidence, explore_sessions, explore_flat_only) =
        parse_engine(take(&chunks, &mut at, "engine settings")?)?;
    if at < chunks.len() {
        return Err(format!("unexpected trailing text '{}'", chunks[at]));
    }

    Ok(SimConfig {
        budget_cents,
        quit_mult,
        sessions,
        rolls_per_hour,
        target_hours,
        confidence,
        table_mins_cents,
        max_rolls,
        sel,
        odds_policy,
        field_12_triple,
        come_odds_work_on_comeout,
        prop_bet_cents,
        table_max_mult,
        explore_sessions,
        explore_flat_only,
    })
}

// --- rendering helpers ---------------------------------------------------

fn tables_text(cfg: &SimConfig) -> String {
    match cfg.table_mins_cents.as_slice() {
        [] => "no tables".to_owned(),
        [one] => format!("a {} table", money_text(*one, false)),
        many => {
            let mins: Vec<String> = many.iter().map(|&m| money_text(m, false)).collect();
            format!("{} tables", mins.join("/"))
        }
    }
}

fn bets_text(cfg: &SimConfig) -> String {
    let s = &cfg.sel;
    let mut parts: Vec<String> = Vec::new();
    if s.pass_line {
        parts.push("pass".to_owned());
    }
    if s.dont_pass {
        parts.push("don't pass".to_owned());
    }
    if s.come_max > 0 {
        parts.push(counted(s.come_max, "come"));
    }
    if s.dont_come_max > 0 {
        parts.push(counted(s.dont_come_max, "don't come"));
    }
    if s.take_odds {
        parts.push(format!("{} odds", odds_word(cfg.odds_policy)));
    } else if cfg.odds_policy != OddsPolicy::X345 {
        // Not taken, but the Explorer still uses the policy: parenthesized.
        parts.push(format!("({} odds)", odds_word(cfg.odds_policy)));
    }
    if s.field {
        parts.push("field".to_owned());
    }
    let place: Vec<String> = PLACE_NUMS
        .iter()
        .zip(s.place.iter())
        .filter(|(_, &on)| on)
        .map(|(n, _)| n.to_string())
        .collect();
    if !place.is_empty() {
        parts.push(format!("place {}", place.join("/")));
    }
    let hard: Vec<String> = HARD_NUMS
        .iter()
        .zip(s.hardways.iter())
        .filter(|(_, &on)| on)
        .map(|(n, _)| n.to_string())
        .collect();
    if !hard.is_empty() {
        parts.push(format!("hard {}", hard.join("/")));
    }
    if s.any_seven {
        parts.push("any 7".to_owned());
    }
    if s.any_craps {
        parts.push("any craps".to_owned());
    }
    if parts.is_empty() {
        "no bets".to_owned()
    } else {
        parts.join(" + ")
    }
}

fn counted(n: u8, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn quit_text(cfg: &SimConfig) -> String {
    match cfg.quit_mult {
        Some(m) => format!("quit at {}×", trim_f64(m)),
        None => "no quit".to_owned(),
    }
}

fn engine_text(cfg: &SimConfig) -> String {
    format!(
        "engine {} sessions, cap {} rolls, {}% confidence, explorer {}/combo{}",
        cfg.sessions,
        cfg.max_rolls,
        trim_f64(cfg.confidence),
        cfg.explore_sessions,
        if cfg.explore_flat_only { " flat" } else { "" }
    )
}

fn odds_word(p: OddsPolicy) -> &'static str {
    match p {
        OddsPolicy::None => "0×",
        OddsPolicy::X1 => "1×",
        OddsPolicy::X2 => "2×",
        OddsPolicy::X345 => "3-4-5×",
        OddsPolicy::X5 => "5×",
        OddsPolicy::X10 => "10×",
    }
}

fn progression_word(p: Progression) -> &'static str {
    PROGRESSION_WORDS
        .iter()
        .find(|(q, _)| *q == p)
        .map(|(_, w)| *w)
        .expect("every progression has a word")
}

/// An f64 with up to four decimals, trailing zeros (and dot) trimmed —
/// enough that quit multiples and confidences round-trip exactly.
fn trim_f64(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

// --- stale diffing --------------------------------------------------------

fn fragment_differs(id: FragmentId, a: &SimConfig, b: &SimConfig) -> bool {
    match id {
        FragmentId::Budget => a.budget_cents != b.budget_cents,
        FragmentId::Tables => a.table_mins_cents != b.table_mins_cents,
        FragmentId::Bets => bets_differ(a, b),
        FragmentId::Progression => a.sel.progression != b.sel.progression,
        FragmentId::Quit => a.quit_mult != b.quit_mult,
        FragmentId::Horizon => {
            a.target_hours != b.target_hours || a.rolls_per_hour != b.rolls_per_hour
        }
        FragmentId::Field12 => a.field_12_triple != b.field_12_triple,
        FragmentId::ComeOddsComeout => a.come_odds_work_on_comeout != b.come_odds_work_on_comeout,
        FragmentId::Props => a.prop_bet_cents != b.prop_bet_cents,
        FragmentId::TableMax => a.table_max_mult != b.table_max_mult,
        FragmentId::Engine => {
            a.sessions != b.sessions
                || a.max_rolls != b.max_rolls
                || a.confidence != b.confidence
                || a.explore_sessions != b.explore_sessions
                || a.explore_flat_only != b.explore_flat_only
        }
    }
}

/// Everything the Bets fragment reflects: the selection minus its
/// progression, plus the odds policy when odds are taken.
fn bets_differ(a: &SimConfig, b: &SimConfig) -> bool {
    let effective = |c: &SimConfig| {
        if c.sel.take_odds {
            c.odds_policy
        } else {
            OddsPolicy::None
        }
    };
    let (x, y) = (&a.sel, &b.sel);
    x.pass_line != y.pass_line
        || x.dont_pass != y.dont_pass
        || x.come_max != y.come_max
        || x.dont_come_max != y.dont_come_max
        || x.take_odds != y.take_odds
        || x.field != y.field
        || x.place != y.place
        || x.hardways != y.hardways
        || x.any_seven != y.any_seven
        || x.any_craps != y.any_craps
        || effective(a) != effective(b)
}

// --- parsing helpers ------------------------------------------------------

/// Lowercase, drop thin spaces (digit grouping), map `−`→`-`, `×`→`x`,
/// `’`→`'`, and collapse every whitespace run to one plain space.
fn normalize(text: &str) -> String {
    let mut flat = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            THIN_SPACE => {}
            MINUS => flat.push('-'),
            '×' => flat.push('x'),
            '\u{2019}' => flat.push('\''),
            c if c.is_whitespace() => flat.push(' '),
            c => flat.extend(c.to_lowercase()),
        }
    }
    let mut out = String::with_capacity(flat.len());
    let mut prev_space = true; // also trims the front
    for ch in flat.chars() {
        if ch == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

fn take<'a>(chunks: &[&'a str], at: &mut usize, what: &str) -> Result<&'a str, String> {
    let chunk = chunks
        .get(*at)
        .copied()
        .ok_or_else(|| format!("sentence ended early — missing the {what} part"))?;
    *at += 1;
    Ok(chunk)
}

fn parse_int<T: std::str::FromStr>(s: &str, what: &str) -> Result<T, String> {
    s.parse::<T>()
        .map_err(|_| format!("expected {what}, found '{s}'"))
}

fn parse_float(s: &str, what: &str) -> Result<f64, String> {
    s.parse::<f64>()
        .map_err(|_| format!("expected {what}, found '{s}'"))
}

/// `$500`, `$12.50`, `-$23.40` (normalized minus) → cents.
fn parse_money(s: &str) -> Result<i64, String> {
    let err = || format!("expected a dollar amount like $500 or $12.50, found '{s}'");
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let rest = rest.strip_prefix('$').ok_or_else(err)?;
    let (dollars, cents) = match rest.split_once('.') {
        Some((d, c)) => (d, c),
        None => (rest, ""),
    };
    if dollars.is_empty() || !dollars.bytes().all(|b| b.is_ascii_digit()) {
        return Err(err());
    }
    let mut value = dollars
        .parse::<i64>()
        .map_err(|_| err())?
        .checked_mul(100)
        .ok_or_else(err)?;
    match cents.len() {
        0 => {}
        2 if cents.bytes().all(|b| b.is_ascii_digit()) => {
            value += cents.parse::<i64>().map_err(|_| err())?;
        }
        _ => return Err(err()),
    }
    Ok(if neg { -value } else { value })
}

fn parse_budget_tables(chunk: &str) -> Result<(i64, Vec<i64>), String> {
    let (budget, rest) = chunk
        .split_once(" at ")
        .ok_or_else(|| format!("expected '<budget> at <minimums> tables', found '{chunk}'"))?;
    let budget_cents = parse_money(budget.trim())?;
    let rest = rest.trim();
    let mins_part = rest
        .strip_suffix("tables")
        .or_else(|| rest.strip_suffix("table"))
        .ok_or_else(|| format!("expected 'tables' to end '{rest}'"))?
        .trim();
    let mins_part = mins_part.strip_prefix("a ").unwrap_or(mins_part).trim();
    if mins_part == "no" {
        return Ok((budget_cents, Vec::new()));
    }
    let mut mins = mins_part
        .split('/')
        .map(|m| parse_money(m.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    // Canonical order, no duplicates — pasted sentences may be hand-edited.
    mins.sort_unstable();
    mins.dedup();
    Ok((budget_cents, mins))
}

/// The bets fragment → a selection (progression left at a placeholder)
/// plus the odds policy. `take_odds` is set from the odds component's form:
/// bare = taken, parenthesized = configured but not taken, absent = the
/// default 3-4-5× not taken.
fn parse_bets(chunk: &str) -> Result<(BetSelection, OddsPolicy), String> {
    let mut sel = BetSelection {
        pass_line: false,
        dont_pass: false,
        come_max: 0,
        dont_come_max: 0,
        take_odds: false,
        field: false,
        place: [false; 6],
        hardways: [false; 4],
        any_seven: false,
        any_craps: false,
        progression: Progression::Flat,
    };
    let mut policy = OddsPolicy::X345;
    if chunk == "no bets" {
        return Ok((sel, policy));
    }
    for raw in chunk.split('+') {
        let comp = raw.trim();
        match comp {
            "" => return Err(format!("dangling '+' in the bets list '{chunk}'")),
            "pass" => sel.pass_line = true,
            "don't pass" => sel.dont_pass = true,
            "field" => sel.field = true,
            "any 7" | "any seven" => sel.any_seven = true,
            "any craps" => sel.any_craps = true,
            _ => {
                if let Some(rest) = comp.strip_prefix("place ") {
                    for n in rest.split('/') {
                        let num: u8 = parse_int(n.trim(), "a place number")?;
                        let idx = place_index(num)
                            .ok_or_else(|| format!("'{num}' is not a place number"))?;
                        sel.place[idx] = true;
                    }
                } else if let Some(rest) = comp.strip_prefix("hard ") {
                    for n in rest.split('/') {
                        let num: u8 = parse_int(n.trim(), "a hardway number")?;
                        let idx = hard_index(num)
                            .ok_or_else(|| format!("'{num}' is not a hardway number"))?;
                        sel.hardways[idx] = true;
                    }
                } else if let Some(inner) = comp.strip_prefix('(').and_then(|c| c.strip_suffix(')'))
                {
                    policy = parse_odds(inner.trim())?;
                    sel.take_odds = false;
                } else if comp.ends_with("odds") {
                    policy = parse_odds(comp)?;
                    sel.take_odds = true;
                } else if let Some((count, kind)) = comp
                    .split_once(' ')
                    .filter(|(count, _)| count.bytes().all(|b| b.is_ascii_digit()))
                {
                    let n: u8 = parse_int(count, "a come-bet count")?;
                    match kind.trim() {
                        "come" | "comes" => sel.come_max = n,
                        "don't come" | "don't comes" => sel.dont_come_max = n,
                        _ => return Err(format!("unknown bet '{comp}'")),
                    }
                } else {
                    return Err(format!("unknown bet '{comp}'"));
                }
            }
        }
    }
    Ok((sel, policy))
}

fn parse_odds(s: &str) -> Result<OddsPolicy, String> {
    let mult = s
        .strip_suffix("odds")
        .ok_or_else(|| format!("unknown odds '{s}'"))?
        .trim();
    match mult {
        "0x" | "no" => Ok(OddsPolicy::None),
        "1x" => Ok(OddsPolicy::X1),
        "2x" => Ok(OddsPolicy::X2),
        "3-4-5x" => Ok(OddsPolicy::X345),
        "5x" => Ok(OddsPolicy::X5),
        "10x" => Ok(OddsPolicy::X10),
        other => Err(format!("unknown odds multiple '{other}'")),
    }
}

fn parse_progression(chunk: &str) -> Result<Progression, String> {
    PROGRESSION_WORDS
        .iter()
        .find(|(_, w)| *w == chunk)
        .map(|(p, _)| *p)
        .ok_or_else(|| format!("unknown progression '{chunk}'"))
}

fn parse_quit(chunk: &str) -> Result<Option<f64>, String> {
    if chunk == "no quit" {
        return Ok(None);
    }
    let rest = chunk
        .strip_prefix("quit at ")
        .ok_or_else(|| format!("expected 'quit at N×' or 'no quit', found '{chunk}'"))?
        .trim();
    let num = rest.strip_suffix('x').unwrap_or(rest).trim();
    Ok(Some(parse_float(num, "a quit multiple")?))
}

fn parse_horizon(chunk: &str) -> Result<(f64, u32), String> {
    let err = || format!("expected '<hours> h at <rate> rolls/h', found '{chunk}'");
    let (hours, rest) = chunk.split_once(" h at ").ok_or_else(err)?;
    let target_hours = parse_float(hours.trim(), "a number of hours")?;
    let rate = rest.trim().strip_suffix("rolls/h").ok_or_else(err)?.trim();
    Ok((target_hours, parse_int(rate, "a rolls-per-hour rate")?))
}

fn parse_field12(chunk: &str) -> Result<bool, String> {
    match chunk {
        "field 12 pays double" => Ok(false),
        "field 12 pays triple" => Ok(true),
        other => Err(format!(
            "expected 'field 12 pays double' or 'field 12 pays triple', found '{other}'"
        )),
    }
}

fn parse_comeout(chunk: &str) -> Result<(), String> {
    if chunk.replace(['-', ' '], "") == "comeoddsworkoncomeout" {
        Ok(())
    } else {
        Err(format!(
            "expected 'come odds work on come-out', found '{chunk}'"
        ))
    }
}

fn parse_props(chunk: &str) -> Result<i64, String> {
    let rest = chunk
        .strip_prefix("props ")
        .ok_or_else(|| format!("expected 'props <stake>', found '{chunk}'"))?;
    parse_money(rest.trim())
}

fn parse_table_max(chunk: &str) -> Result<i64, String> {
    let err = || format!("expected 'max N× min', found '{chunk}'");
    let rest = chunk
        .strip_prefix("max ")
        .and_then(|c| c.strip_suffix("min"))
        .ok_or_else(err)?
        .trim();
    let mult = rest.strip_suffix('x').unwrap_or(rest).trim();
    parse_int(mult, "a table-max multiple")
}

fn parse_engine(chunk: &str) -> Result<(u32, u64, f64, u32, bool), String> {
    let err = |part: &str| {
        format!(
            "expected 'engine N sessions, cap N rolls, N% confidence, explorer N/combo', \
             found '{part}'"
        )
    };
    let rest = chunk.strip_prefix("engine ").ok_or_else(|| err(chunk))?;
    let (rest, explore_flat_only) = match rest.strip_suffix(" flat") {
        Some(r) => (r, true),
        None => (rest, false),
    };
    let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(err(chunk));
    }
    let sessions = parts[0]
        .strip_suffix("sessions")
        .ok_or_else(|| err(parts[0]))?
        .trim();
    let sessions: u32 = parse_int(sessions, "a session count")?;
    let cap = parts[1]
        .strip_prefix("cap ")
        .and_then(|p| p.strip_suffix("rolls"))
        .ok_or_else(|| err(parts[1]))?
        .trim();
    let max_rolls: u64 = parse_int(cap, "a roll cap")?;
    let conf = parts[2]
        .strip_suffix("confidence")
        .ok_or_else(|| err(parts[2]))?
        .trim();
    let conf = conf.strip_suffix('%').unwrap_or(conf).trim();
    let confidence = parse_float(conf, "a confidence percentage")?;
    let explore = parts[3]
        .strip_prefix("explorer ")
        .and_then(|p| p.strip_suffix("/combo"))
        .ok_or_else(|| err(parts[3]))?
        .trim();
    let explore_sessions: u32 = parse_int(explore, "an explorer session count")?;
    Ok((
        sessions,
        max_rolls,
        confidence,
        explore_sessions,
        explore_flat_only,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Splitmix-ish LCG so the varied configs are deterministic without the
    /// rand crate.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }

        fn flag(&mut self) -> bool {
            (self.next() >> 63) == 1
        }
    }

    fn round_trip(cfg: &SimConfig) {
        let text = render_text(cfg);
        let back = parse(&text).unwrap_or_else(|e| panic!("parse failed on '{text}': {e}"));
        assert_eq!(&back, cfg, "round-trip mismatch for '{text}'");
    }

    fn varied(i: usize, lcg: &mut Lcg) -> SimConfig {
        const QUITS: [Option<f64>; 5] = [None, Some(1.1), Some(1.5), Some(2.0), Some(3.75)];
        const HOURS: [f64; 3] = [0.5, 4.0, 12.25];
        const CONF: [f64; 3] = [90.0, 95.0, 99.0];
        const BUDGETS: [i64; 5] = [30_000, 30_050, 500_000, 1_234_567, 7_500];
        const SESSIONS: [u32; 4] = [100, 20_000, 100_000, 1_000_000];
        const RATES: [u32; 3] = [60, 100, 144];
        const CAPS: [u64; 3] = [1_000, 200_000, 10_000_000];
        const PROPS: [i64; 3] = [100, 500, 2_575];
        const MAXES: [i64; 3] = [50, 500, 1_000];
        const EXPLORES: [u32; 3] = [500, 5_000, 250_000];
        let mins: Vec<i64> = match i % 5 {
            0 => vec![500],
            1 => vec![1_500],
            2 => vec![500, 1_500, 2_500],
            3 => vec![500, 1_000, 1_500, 2_500, 5_000, 10_000],
            _ => vec![2_500, 10_000, 100_000],
        };
        let mut place = [false; 6];
        for p in &mut place {
            *p = lcg.flag();
        }
        let mut hardways = [false; 4];
        for h in &mut hardways {
            *h = lcg.flag();
        }
        SimConfig {
            budget_cents: BUDGETS[(i + 2) % 5],
            quit_mult: QUITS[i % 5],
            sessions: SESSIONS[i % 4],
            rolls_per_hour: RATES[(i / 9) % 3],
            target_hours: HOURS[i % 3],
            confidence: CONF[(i / 3) % 3],
            table_mins_cents: mins,
            max_rolls: CAPS[(i / 2) % 3],
            sel: BetSelection {
                pass_line: lcg.flag(),
                dont_pass: lcg.flag(),
                come_max: (i % 7) as u8,
                dont_come_max: ((i / 7) % 7) as u8,
                take_odds: (i / 6).is_multiple_of(2),
                field: lcg.flag(),
                place,
                hardways,
                any_seven: lcg.flag(),
                any_craps: lcg.flag(),
                progression: Progression::ALL[i % 12],
            },
            odds_policy: OddsPolicy::ALL[i % 6],
            field_12_triple: lcg.flag(),
            come_odds_work_on_comeout: lcg.flag(),
            prop_bet_cents: PROPS[(i / 5) % 3],
            table_max_mult: MAXES[(i / 4) % 3],
            explore_sessions: EXPLORES[(i / 8) % 3],
            explore_flat_only: i % 2 == 1,
        }
    }

    #[test]
    fn the_law_round_trips_the_default_config() {
        let cfg = SimConfig::default();
        assert_eq!(
            render_text(&cfg),
            "$300 at $5/$10/$15/$25/$50/$100 tables · pass · flat · no quit · \
             4 h at 100 rolls/h · field 12 pays double · props $5 · max 500× min · \
             engine 100000 sessions, cap 200000 rolls, 95% confidence, explorer 5000/combo"
        );
        round_trip(&cfg);
    }

    #[test]
    fn the_law_round_trips_varied_configs() {
        let mut lcg = Lcg(0x9E37_79B9_7F4A_7C15);
        for i in 0..240 {
            round_trip(&varied(i, &mut lcg));
        }
    }

    #[test]
    fn the_law_round_trips_edge_cases() {
        // Zero bets at the default policy: "no bets".
        let mut cfg = SimConfig {
            sel: BetSelection {
                pass_line: false,
                ..BetSelection::default()
            },
            ..SimConfig::default()
        };
        assert!(render_text(&cfg).contains("no bets"));
        round_trip(&cfg);

        // Zero bets, non-default policy not taken: parenthesized, lossless.
        cfg.odds_policy = OddsPolicy::X10;
        assert!(render_text(&cfg).contains("(10× odds)"));
        round_trip(&cfg);

        // Degenerate: odds taken under OddsPolicy::None renders 0×.
        let cfg = SimConfig {
            sel: BetSelection {
                take_odds: true,
                ..BetSelection::default()
            },
            odds_policy: OddsPolicy::None,
            ..SimConfig::default()
        };
        assert!(render_text(&cfg).contains("0× odds"));
        round_trip(&cfg);

        // Singular forms: one table, one come, one don't come.
        let cfg = SimConfig {
            table_mins_cents: vec![1_500],
            sel: BetSelection {
                come_max: 1,
                dont_come_max: 1,
                ..BetSelection::default()
            },
            ..SimConfig::default()
        };
        let text = render_text(&cfg);
        assert!(text.contains("a $15 table"));
        assert!(text.contains("1 come + 1 don't come"));
        round_trip(&cfg);

        // Negative budget exercises the true minus; empty mins render too.
        let cfg = SimConfig {
            budget_cents: -2_550,
            table_mins_cents: Vec::new(),
            ..SimConfig::default()
        };
        assert!(render_text(&cfg).contains(&format!("{MINUS}$25.50")));
        round_trip(&cfg);

        // Both optional flags on at once.
        let cfg = SimConfig {
            come_odds_work_on_comeout: true,
            explore_flat_only: true,
            ..SimConfig::default()
        };
        round_trip(&cfg);
    }

    #[test]
    fn parse_tolerates_ascii_and_sloppy_input() {
        let messy = "  $300.50  AT $15/$100 TABLES · PASS + DON'T PASS + 2 COMES + \
                     1 DON'T COME + 3-4-5x ODDS + PLACE 6/8 + HARD 8 + ANY 7 · \
                     Half   Press ·  QUIT AT 2x · 0.5 H AT 100 ROLLS/H · \
                     FIELD 12 PAYS TRIPLE · COME ODDS WORK ON COME-OUT · PROPS $5 · \
                     MAX 500x MIN · ENGINE 100000 SESSIONS, CAP 200000 ROLLS, \
                     95% CONFIDENCE, EXPLORER 5000/COMBO FLAT";
        let cfg = parse(messy).expect("messy sentence should parse");
        let want = SimConfig {
            budget_cents: 30_050,
            quit_mult: Some(2.0),
            target_hours: 0.5,
            table_mins_cents: vec![1_500, 10_000],
            sel: BetSelection {
                pass_line: true,
                dont_pass: true,
                come_max: 2,
                dont_come_max: 1,
                take_odds: true,
                place: {
                    let mut p = [false; 6];
                    p[place_index(6).unwrap()] = true;
                    p[place_index(8).unwrap()] = true;
                    p
                },
                hardways: {
                    let mut h = [false; 4];
                    h[hard_index(8).unwrap()] = true;
                    h
                },
                any_seven: true,
                progression: Progression::HalfPress,
                ..BetSelection::default()
            },
            field_12_triple: true,
            come_odds_work_on_comeout: true,
            explore_flat_only: true,
            ..SimConfig::default()
        };
        assert_eq!(cfg, want);

        // ASCII hyphen accepted wherever the canonical text has a true minus.
        let neg = SimConfig {
            budget_cents: -5_000,
            ..SimConfig::default()
        };
        let canonical = render_text(&neg);
        assert!(canonical.contains(MINUS));
        let ascii = canonical.replace(MINUS, "-");
        assert_eq!(parse(&ascii).unwrap(), neg);
    }

    #[test]
    fn parse_names_the_offending_token() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());

        let err = parse("total garbage").unwrap_err();
        assert!(err.contains("total garbage"), "error was: {err}");

        let trailing = format!("{} · gibberish", render_text(&SimConfig::default()));
        let err = parse(&trailing).unwrap_err();
        assert!(err.contains("gibberish"), "error was: {err}");

        let junk_in_engine = format!("{} junk", render_text(&SimConfig::default()));
        let err = parse(&junk_in_engine).unwrap_err();
        assert!(err.contains("junk"), "error was: {err}");

        let bad_bet = parse(
            "$300 at a $15 table · yo eleven · flat · no quit · 4 h at 100 rolls/h · \
             field 12 pays double · props $5 · max 500× min · engine 100 sessions, \
             cap 1000 rolls, 95% confidence, explorer 500/combo",
        )
        .unwrap_err();
        assert!(bad_bet.contains("yo eleven"), "error was: {bad_bet}");
    }

    #[test]
    fn spans_cover_the_text_and_diff_stale_fragments() {
        let cfg = SimConfig::default();
        let spans = render_spans(&cfg, None);

        let concat: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(concat, render_text(&cfg));

        let ids: Vec<FragmentId> = spans.iter().filter_map(|s| s.frag).collect();
        assert_eq!(
            ids,
            [
                FragmentId::Budget,
                FragmentId::Tables,
                FragmentId::Bets,
                FragmentId::Progression,
                FragmentId::Quit,
                FragmentId::Horizon,
                FragmentId::Field12,
                FragmentId::Props,
                FragmentId::TableMax,
                FragmentId::Engine,
            ]
        );

        let glue: Vec<&str> = spans
            .iter()
            .filter(|s| s.frag.is_none())
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(glue[0], " at ");
        assert!(glue[1..].iter().all(|&g| g == " · "));

        // No provenance: nothing is stale.
        assert!(spans.iter().all(|s| !s.stale));

        // Provenance differing only in budget: exactly the Budget span is stale.
        let mut prov = cfg.clone();
        prov.budget_cents += 10_000;
        let spans = render_spans(&cfg, Some(&prov));
        for s in &spans {
            assert_eq!(
                s.stale,
                s.frag == Some(FragmentId::Budget),
                "span '{}' has wrong staleness",
                s.text
            );
        }

        // The optional fragment appears (and diffs) when the flag is on.
        let mut on = cfg.clone();
        on.come_odds_work_on_comeout = true;
        let spans = render_spans(&on, Some(&cfg));
        let comeout = spans
            .iter()
            .find(|s| s.frag == Some(FragmentId::ComeOddsComeout))
            .expect("come-out fragment renders when true");
        assert_eq!(comeout.text, "come odds work on come-out");
        assert!(comeout.stale);
    }

    #[test]
    fn spec_phrasing_goals() {
        let text = render_text(&SimConfig::default());
        assert!(text.contains("pass"));
        assert!(text.contains("flat"));

        let cfg = SimConfig {
            field_12_triple: true,
            ..SimConfig::default()
        };
        assert!(render_text(&cfg).contains("field 12 pays triple"));
    }
}
