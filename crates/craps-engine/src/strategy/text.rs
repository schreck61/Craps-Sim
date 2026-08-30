// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The text form: the same tree, typed instead of clicked.
//!
//! Principle 2 of [`STRATEGY_DSL.md`](../../../../docs/STRATEGY_DSL.md) is
//! that there is one AST and two editors. This module is the second one.
//! [`render`] writes a [`Strategy`] out, [`parse`] reads it back, and the
//! law `parse(render(s)) == s` is property-tested over randomized rule sets,
//! exactly as the Scenario Sentence codec already is. A text form that could
//! say something the tree cannot, or lose something the tree holds, would
//! make the rule editor a lie.
//!
//! # One lexical rule worth knowing
//!
//! Binary operators need spaces around them. `dont-pass` and `hits-this-
//! shooter` are single words, so `a - b` is subtraction and `a-b` is an
//! identifier that does not exist. A negative literal is the exception:
//! `-200` immediately followed by digits is a number. This keeps hyphenated
//! names — which is how every bet in craps is actually spelled — without a
//! symbol table in the tokenizer.

use crate::bets::Progression;
use crate::strategy::ast::{AmountExpr, BinOp, Expr, Group, Read, Rule, Stmt, Strategy, Trigger};
use crate::strategy::view::{stream_of, STREAMS};
use crate::strategy::BetRef;

/// The grammar version. A saved strategy states which grammar it was
/// written against, and the parser refuses one it does not know rather than
/// guessing — a strategy must never quietly change meaning under a later
/// revision.
pub const LANGUAGE_VERSION: u32 = 1;

/// Why a strategy could not be read, and where.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParseError {
    /// 1-based line the offending token sits on.
    pub line: usize,
    /// The token itself, so the message can name it.
    pub token: String,
    pub what: String,
}

impl ParseError {
    pub fn message(&self) -> String {
        if self.token.is_empty() {
            format!("line {}: {}", self.line, self.what)
        } else {
            format!(
                "line {}: {} — found \"{}\"",
                self.line, self.what, self.token
            )
        }
    }
}

// ---------------------------------------------------------------- tokenizer

#[derive(Clone, Debug, PartialEq)]
struct Token {
    text: String,
    line: usize,
    /// Byte offset of the token's first character in the original source.
    ///
    /// Carried so a `for each` block can keep its own text verbatim —
    /// comments and spacing included. Reconstructing the block from the
    /// rules it produced would normalise both away, and a strategy that
    /// silently loses the author's comments on every save is not preserving
    /// anything worth the name.
    start: usize,
}

fn tokenize(src: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut line_start = 0usize;
    for (n, whole) in src.split_inclusive('\n').enumerate() {
        let line = n + 1;
        let raw = whole.trim_end_matches(['\n', '\r']);
        let text = raw.split('#').next().unwrap_or("");
        let cs: Vec<char> = text.chars().collect();
        // Byte offset of each character in the line, plus one past the end.
        let offs: Vec<usize> = text
            .char_indices()
            .map(|(b, _)| b)
            .chain(std::iter::once(text.len()))
            .collect();
        let byte = |i: usize| line_start + offs.get(i).copied().unwrap_or(text.len());
        let push = |out: &mut Vec<Token>, s: String, from: usize| {
            out.push(Token {
                text: s,
                line,
                start: byte(from),
            })
        };

        let mut i = 0;
        while i < cs.len() {
            let c = cs[i];
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            let from = i;
            // A quoted name is one token, quotes included, so a strategy may
            // be called anything a person would call it.
            if c == '"' {
                let mut s = String::from('"');
                i += 1;
                while i < cs.len() && cs[i] != '"' {
                    s.push(cs[i]);
                    i += 1;
                }
                s.push('"');
                i += 1;
                push(&mut out, s, from);
                continue;
            }
            // Two-character comparisons before one-character ones.
            if i + 1 < cs.len() {
                let pair: String = cs[i..i + 2].iter().collect();
                if matches!(pair.as_str(), "<=" | ">=" | "==" | "!=") {
                    push(&mut out, pair, from);
                    i += 2;
                    continue;
                }
            }
            if "():,=<>+*/{}".contains(c) {
                push(&mut out, c.to_string(), from);
                i += 1;
                continue;
            }
            // `-200` is a literal; `a - b` is subtraction. Hyphenated words
            // are handled by the identifier scan below, which is why the
            // operator needs its spaces.
            if c == '-' {
                if cs.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                    i += 1;
                    while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                        i += 1;
                    }
                    push(&mut out, cs[from..i].iter().collect(), from);
                } else {
                    push(&mut out, "-".into(), from);
                    i += 1;
                }
                continue;
            }
            if c == '$' || c.is_ascii_digit() {
                let money = c == '$';
                if money {
                    i += 1;
                }
                // Thousands separators belong to money and nowhere else: a
                // bare `9267,` inside `max(9267, x)` is a number and a comma.
                while i < cs.len()
                    && (cs[i].is_ascii_digit() || cs[i] == '.' || (money && cs[i] == ','))
                {
                    i += 1;
                }
                // `1-3-2-6` is the name of a progression, not three
                // subtractions, so a number that runs straight into a hyphen
                // and more digits keeps going.
                while i + 1 < cs.len() && cs[i] == '-' && cs[i + 1].is_ascii_digit() {
                    i += 1;
                    while i < cs.len() && cs[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                push(&mut out, cs[from..i].iter().collect(), from);
                continue;
            }
            if c.is_alphabetic() || c == '_' {
                while i < cs.len()
                    && (cs[i].is_alphanumeric()
                        || cs[i] == '_'
                        || cs[i] == '\''
                        || (cs[i] == '-'
                            && cs
                                .get(i + 1)
                                .is_some_and(|d| d.is_alphanumeric() || *d == '_')))
                {
                    i += 1;
                }
                push(&mut out, cs[from..i].iter().collect(), from);
                continue;
            }
            push(&mut out, c.to_string(), from);
            i += 1;
        }
        line_start += whole.len();
    }
    out
}

// ------------------------------------------------------------------- parser

struct Parser {
    toks: Vec<Token>,
    at: usize,
    vars: Vec<String>,
    /// The source, kept so a block can record its own text verbatim.
    src: String,
    /// Blocks recorded while reading, in the order they appeared. Only the
    /// outermost: a nested block is already inside its parent's verbatim
    /// text, and recording both would leave two records claiming the same
    /// starting rule.
    blocks: Vec<crate::strategy::ast::Block>,
    /// How many `for each` blocks are currently open.
    depth: usize,
    /// Names bound by an enclosing `for each`, innermost last.
    ///
    /// A binding is not memory: it is a number the parser substitutes while
    /// it reads the block, so `for each of 6, 8 as n` produces two rules
    /// that mention 6 and 8 and nothing that mentions `n`. That keeps the
    /// tree exactly what it would have been written by hand, which is what
    /// lets the round-trip law stay a law.
    bindings: Vec<(String, i64)>,
}

impl Parser {
    fn peek(&self) -> &str {
        self.toks.get(self.at).map_or("", |t| t.text.as_str())
    }

    fn peek_at(&self, n: usize) -> &str {
        self.toks.get(self.at + n).map_or("", |t| t.text.as_str())
    }

    fn line(&self) -> usize {
        self.toks
            .get(self.at)
            .or_else(|| self.toks.last())
            .map_or(1, |t| t.line)
    }

    fn err<T>(&self, what: &str) -> Result<T, ParseError> {
        Err(ParseError {
            line: self.line(),
            token: self.peek().to_owned(),
            what: what.to_owned(),
        })
    }

    fn next(&mut self) -> String {
        let t = self.peek().to_owned();
        self.at += 1;
        t
    }

    fn eat(&mut self, word: &str) -> bool {
        if self.peek().eq_ignore_ascii_case(word) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, word: &str) -> Result<(), ParseError> {
        if self.eat(word) {
            Ok(())
        } else {
            self.err(&format!("expected \"{word}\""))
        }
    }

    fn done(&self) -> bool {
        self.at >= self.toks.len()
    }

    fn number(&mut self) -> Result<i64, ParseError> {
        let t = self.peek().to_owned();
        if let Some(v) = self.binding(&t) {
            self.at += 1;
            return Ok(v);
        }
        if let Some(v) = parse_money(&t) {
            self.at += 1;
            return Ok(v);
        }
        self.err("expected a number")
    }

    /// The innermost binding of this name, if any.
    fn binding(&self, name: &str) -> Option<i64> {
        self.bindings
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| *v)
    }
}

/// `$44`, `$44.50`, `4400` — money carries a `$` and is read to the cent;
/// a bare number is already cents, which is the only unit this engine has.
fn parse_money(t: &str) -> Option<i64> {
    if let Some(rest) = t.strip_prefix('$') {
        let clean: String = rest.chars().filter(|c| *c != ',').collect();
        let (whole, frac) = match clean.split_once('.') {
            Some((w, f)) => (w, format!("{f:0<2}")),
            None => (clean.as_str(), "00".to_string()),
        };
        let w: i64 = whole.parse().ok()?;
        let f: i64 = frac.get(..2)?.parse().ok()?;
        return Some(w * 100 + if w < 0 { -f } else { f });
    }
    t.parse::<i64>().ok()
}

const PROGRESSION_WORDS: [(Progression, &str); 12] = [
    (Progression::Flat, "flat"),
    (Progression::FullPress, "full-press"),
    (Progression::HalfPress, "half-press"),
    (Progression::PressAndPull, "press-and-pull"),
    (Progression::Paroli3, "paroli-3"),
    (Progression::S1326, "1-3-2-6"),
    (Progression::Martingale, "martingale"),
    (Progression::GrandMartingale, "grand-martingale"),
    (Progression::DAlembert, "dalembert"),
    (Progression::ReverseDAlembert, "reverse-dalembert"),
    (Progression::Fibonacci, "fibonacci"),
    (Progression::OscarsGrind, "oscars-grind"),
];

fn progression_word(p: Progression) -> &'static str {
    PROGRESSION_WORDS
        .iter()
        .find(|(q, _)| *q == p)
        .map(|(_, w)| *w)
        .unwrap_or("flat")
}

impl Parser {
    /// `pass`, `dont pass`, `place 6`, `odds on come 6`, …
    fn bet_ref(&mut self) -> Result<BetRef, ParseError> {
        if self.eat("odds") {
            self.expect("on")?;
            return self.odds_target();
        }
        if self.eat("pass") {
            return Ok(BetRef::Pass);
        }
        if self.eat("dont") {
            if self.eat("pass") {
                return Ok(BetRef::DontPass);
            }
            if self.eat("come") {
                return Ok(BetRef::DontCome);
            }
            return self.err("expected \"pass\" or \"come\" after \"dont\"");
        }
        if self.eat("come") {
            return Ok(BetRef::Come);
        }
        if self.eat("field") {
            return Ok(BetRef::Field);
        }
        if self.eat("any") {
            if self.eat("seven") {
                return Ok(BetRef::AnySeven);
            }
            if self.eat("craps") {
                return Ok(BetRef::AnyCraps);
            }
            return self.err("expected \"seven\" or \"craps\" after \"any\"");
        }
        if self.eat("place") {
            let n = self.number()? as u8;
            return match crate::place_index(n) {
                Some(_) => Ok(BetRef::Place(n)),
                None => self.err("not a place number (4, 5, 6, 8, 9 or 10)"),
            };
        }
        if self.eat("hard") {
            let n = self.number()? as u8;
            return match crate::hard_index(n) {
                Some(_) => Ok(BetRef::Hardway(n)),
                None => self.err("not a hardway number (4, 6, 8 or 10)"),
            };
        }
        self.err("expected a bet")
    }

    fn odds_target(&mut self) -> Result<BetRef, ParseError> {
        if self.eat("pass") {
            return Ok(BetRef::PassOdds);
        }
        if self.eat("dont") {
            if self.eat("pass") {
                return Ok(BetRef::DontPassLay);
            }
            self.expect("come")?;
            let n = self.number()? as u8;
            return Ok(BetRef::DontComeLay(n));
        }
        if self.eat("come") {
            let n = self.number()? as u8;
            return Ok(BetRef::ComeOdds(n));
        }
        self.err("expected the bet the odds sit behind")
    }

    fn trigger(&mut self) -> Result<Trigger, ParseError> {
        for (word, t) in [
            ("session-start", Trigger::SessionStart),
            ("come-out", Trigger::ComeOut),
            ("point-established", Trigger::PointEstablished),
            ("point-made", Trigger::PointMade),
            ("seven-out", Trigger::SevenOut),
            ("roll", Trigger::Roll),
        ] {
            if self.eat(word) {
                return Ok(t);
            }
        }
        if self.eat("total") {
            self.expect("(")?;
            let n = self.number()? as u8;
            self.expect(")")?;
            return Ok(Trigger::Total(n));
        }
        // `on come point on 6:` — a come flat reaching a box number, which
        // is a different event from the table's point being established and
        // could only be approximated with memory before.
        if self.peek().eq_ignore_ascii_case("come") && self.peek_at(1).eq_ignore_ascii_case("point")
        {
            self.at += 2;
            self.expect("on")?;
            return Ok(Trigger::ComePointEstablished(self.number()? as u8));
        }
        if self.peek().eq_ignore_ascii_case("dont")
            && self.peek_at(1).eq_ignore_ascii_case("come")
            && self.peek_at(2).eq_ignore_ascii_case("point")
        {
            self.at += 3;
            self.expect("on")?;
            return Ok(Trigger::DontComePointEstablished(self.number()? as u8));
        }
        if self.eat("win") {
            self.expect("of")?;
            return Ok(Trigger::Win(self.bet_ref()?));
        }
        if self.eat("loss") {
            self.expect("of")?;
            return Ok(Trigger::Loss(self.bet_ref()?));
        }
        self.err("expected a trigger")
    }
}

impl Parser {
    fn expr(&mut self) -> Result<Expr, ParseError> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.and_expr()?;
        while self.eat("or") {
            a = Expr::bin(BinOp::Or, a, self.and_expr()?);
        }
        Ok(a)
    }

    fn and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.cmp_expr()?;
        while self.eat("and") {
            a = Expr::bin(BinOp::And, a, self.cmp_expr()?);
        }
        Ok(a)
    }

    fn cmp_expr(&mut self) -> Result<Expr, ParseError> {
        let a = self.add_expr()?;
        for (word, op) in [
            ("<=", BinOp::Le),
            (">=", BinOp::Ge),
            ("==", BinOp::Eq),
            ("!=", BinOp::Ne),
            ("<", BinOp::Lt),
            (">", BinOp::Gt),
        ] {
            if self.eat(word) {
                return Ok(Expr::bin(op, a, self.add_expr()?));
            }
        }
        Ok(a)
    }

    fn add_expr(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.mul_expr()?;
        loop {
            if self.eat("+") {
                a = Expr::bin(BinOp::Add, a, self.mul_expr()?);
            } else if self.eat("-") {
                a = Expr::bin(BinOp::Sub, a, self.mul_expr()?);
            } else {
                return Ok(a);
            }
        }
    }

    fn mul_expr(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.unary()?;
        loop {
            if self.eat("*") {
                a = Expr::bin(BinOp::Mul, a, self.unary()?);
            } else if self.eat("/") {
                a = Expr::bin(BinOp::Div, a, self.unary()?);
            } else {
                return Ok(a);
            }
        }
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        if self.eat("not") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        if self.eat("(") {
            let e = self.expr()?;
            self.expect(")")?;
            return Ok(e);
        }
        if self.eat("min") || self.eat("max") {
            // `min(a, b)` reads better than an infix operator nobody knows.
            let op = if self.toks[self.at - 1].text.eq_ignore_ascii_case("min") {
                BinOp::Min
            } else {
                BinOp::Max
            };
            self.expect("(")?;
            let a = self.expr()?;
            self.expect(",")?;
            let b = self.expr()?;
            self.expect(")")?;
            return Ok(Expr::bin(op, a, b));
        }
        if parse_money(self.peek()).is_some() {
            return Ok(Expr::Const(self.number()?));
        }
        // Reads that take a bet, and reads that take a number.
        for (word, make) in [
            ("stake", 0usize),
            ("up", 1),
            ("wins", 2),
            ("losses", 3),
            ("streak", 4),
            ("paid", 5),
        ] {
            if self.peek().eq_ignore_ascii_case(word) && self.peek_at(1) == "(" {
                self.at += 2;
                let b = self.bet_ref()?;
                self.expect(")")?;
                return Ok(Expr::Read(match make {
                    0 => Read::Stake(b),
                    1 => Read::Up(b),
                    2 => Read::Wins(b),
                    3 => Read::Losses(b),
                    4 => Read::Streak(b),
                    _ => Read::Paid(b),
                }));
            }
        }
        for (word, make) in [
            ("hits", 0usize),
            ("hits-this-shooter", 1),
            ("come-point", 2),
            ("dont-come-point", 3),
        ] {
            if self.peek().eq_ignore_ascii_case(word) && self.peek_at(1) == "(" {
                self.at += 2;
                let n = self.number()? as u8;
                self.expect(")")?;
                return Ok(Expr::Read(match make {
                    0 => Read::Hits(n),
                    1 => Read::HitsThisShooter(n),
                    2 => Read::ComePoint(n),
                    _ => Read::DontComePoint(n),
                }));
            }
        }
        for (word, r) in [
            ("point", Read::Point),
            ("come-out", Read::ComeOut),
            ("last-total", Read::LastTotal),
            ("roll", Read::Roll),
            ("rolls-this-shooter", Read::RollsThisShooter),
            ("shooter", Read::Shooter),
            ("cash", Read::Cash),
            ("wealth", Read::Wealth),
            ("profit", Read::Profit),
            ("peak-profit", Read::PeakProfit),
            ("drawdown", Read::Drawdown),
            ("handle", Read::Handle),
            ("live-come", Read::LiveCome),
            ("live-dont-come", Read::LiveDontCome),
            ("on-table-face", Read::OnTableFace),
        ] {
            if self.eat(word) {
                return Ok(Expr::Read(r));
            }
        }
        let name = self.peek().to_owned();
        if let Some(v) = self.binding(&name) {
            self.at += 1;
            return Ok(Expr::Const(v));
        }
        if let Some(i) = self.vars.iter().position(|v| *v == name) {
            self.at += 1;
            return Ok(Expr::Var(i as u16));
        }
        self.err("expected a value")
    }

    /// Words that can only begin a statement or a rule, and so can never be
    /// the start of an amount. Seeing one means the bet was written without
    /// a stake.
    fn at_statement_boundary(&self) -> bool {
        const STARTERS: [&str; 10] = [
            "on", "for", "}", "bet", "press", "regress", "down", "working", "leave", "set",
        ];
        self.done() || STARTERS.iter().any(|w| self.peek().eq_ignore_ascii_case(w))
    }

    /// An amount, or nothing — and nothing means whatever this stream's
    /// pressing calls for, which under a flat progression is the base
    /// stake. `bet pass` is how a player says it, and making them write
    /// `bet pass pressed` on a table where nothing presses would be the
    /// language describing the engine rather than the game.
    fn amount(&mut self) -> Result<AmountExpr, ParseError> {
        if self.at_statement_boundary() {
            return Ok(AmountExpr::Pressed);
        }
        if self.eat("base") {
            return Ok(AmountExpr::Base);
        }
        if self.eat("pressed") {
            return Ok(AmountExpr::Pressed);
        }
        // `max` is the odds keyword and also the two-argument function, so
        // it only means "take the most the policy allows" when nothing
        // follows it to be the maximum *of*.
        if self.peek().eq_ignore_ascii_case("max") && self.peek_at(1) != "(" {
            self.at += 1;
            return Ok(AmountExpr::MaxOdds);
        }
        let e = self.expr()?;
        if self.eat("units") || self.eat("u") {
            return Ok(AmountExpr::Units(e));
        }
        let _ = self.eat("cents");
        Ok(AmountExpr::Cents(e))
    }

    /// A group of bets named as one — `place inside`, `all hardways`.
    ///
    /// Sugar, expanded here into one statement per member, because "place
    /// the inside numbers" is how the bet is spoken and a language that
    /// made the user write four rules for it would be describing the engine
    /// rather than the game. Nothing renders back as a group: the tree
    /// holds the members, and the round-trip law is about the tree.
    fn group(&mut self) -> Option<&'static [BetRef]> {
        if self.peek().eq_ignore_ascii_case("place") {
            for (word, g) in [("inside", Group::Inside), ("outside", Group::Outside)] {
                if self.peek_at(1).eq_ignore_ascii_case(word) {
                    self.at += 2;
                    return Some(g.members());
                }
            }
            return None;
        }
        if self.peek().eq_ignore_ascii_case("all") {
            for (word, g) in [
                ("place", Group::AllPlace),
                ("hardways", Group::AllHardways),
                ("hard", Group::AllHardways),
            ] {
                if self.peek_at(1).eq_ignore_ascii_case(word) {
                    self.at += 2;
                    return Some(g.members());
                }
            }
        }
        None
    }

    /// Rules until the end of input, or until a `}` closes a block.
    fn rules_until_end(&mut self) -> Result<Vec<Rule>, ParseError> {
        self.rules_from(0)
    }

    /// `base` is how many rules already precede these in the strategy, so a
    /// nested block can record where its own rules land.
    fn rules_from(&mut self, base: usize) -> Result<Vec<Rule>, ParseError> {
        let mut out = Vec::new();
        while !self.done() && self.peek() != "}" {
            if self.peek().eq_ignore_ascii_case("for") {
                let so_far = base + out.len();
                out.extend(self.for_each(so_far)?);
            } else {
                out.push(self.rule()?);
            }
        }
        Ok(out)
    }

    fn rule(&mut self) -> Result<Rule, ParseError> {
        self.expect("on")?;
        let trigger = self.trigger()?;
        let guard = if self.eat("when") {
            Some(self.expr()?)
        } else {
            None
        };
        self.expect(":")?;

        let mut body = Vec::new();
        // A body runs until the next rule begins. `on`, `for` and `}` are the
        // only words that can start one and never start a statement, which is
        // what keeps the indentation decorative rather than load-bearing.
        while !self.done()
            && !self.peek().eq_ignore_ascii_case("on")
            && !self.peek().eq_ignore_ascii_case("for")
            && self.peek() != "}"
        {
            body.extend(self.stmt()?);
        }
        if body.is_empty() {
            return self.err("a rule with no actions does nothing");
        }
        let mut r = Rule::new(trigger, body);
        r.guard = guard;
        Ok(r)
    }

    /// `for each of 4, 5, 6, 8, 9, 10 as n { … }`
    ///
    /// Bounded iteration over a list written out in full — the only loop
    /// this language has, and the reason it still terminates by
    /// construction. The block is read once per value with `n` bound to it,
    /// so what comes out is the rules somebody would otherwise have typed
    /// six times.
    ///
    /// It is sugar, and it does not survive rendering: the tree holds the
    /// expanded rules, the same way a group of bets does, because the tree
    /// is what the round-trip law is about.
    fn for_each(&mut self, rules_so_far: usize) -> Result<Vec<Rule>, ParseError> {
        self.expect("for")?;
        self.expect("each")?;
        self.expect("of")?;
        let mut values = vec![self.number()?];
        while self.eat(",") {
            values.push(self.number()?);
        }
        self.expect("as")?;
        let name = self.next();
        if name.is_empty() || parse_money(&name).is_some() {
            return Err(ParseError {
                line: self.line(),
                token: name,
                what: "expected a name to bind each value to".into(),
            });
        }
        if self.vars.contains(&name) {
            return Err(ParseError {
                line: self.line(),
                token: name,
                what: "already the name of a memory slot; a binding is a \
                       number, not memory, so give it its own name"
                    .into(),
            });
        }
        // Just past the brace, not at the first token inside it: comments
        // are not tokens, so starting at the first rule would drop any note
        // the author wrote at the top of the block.
        let body_from = self
            .toks
            .get(self.at)
            .map(|t| t.start + 1)
            .unwrap_or_default();
        self.expect("{")?;
        self.depth += 1;
        let body_start = self.at;

        let mut out = Vec::new();
        let mut per_iteration = 0usize;
        for (k, v) in values.iter().enumerate() {
            self.at = body_start;
            self.bindings.push((name.clone(), *v));
            let rules = self.rules_until_end();
            self.bindings.pop();
            let rules = rules?;
            if k == 0 {
                per_iteration = rules.len();
            }
            out.extend(rules);
            if self.peek() != "}" {
                return self.err("expected \"}\" to close the block");
            }
        }
        // The body's own text, from just after `{` to just before `}`.
        let body_to = self.toks[self.at].start;
        // Stored without the newlines that hug the braces, so writing it
        // back out as `{\n…\n}` reproduces exactly this text and rendering
        // stays idempotent. Everything between — comments, indentation,
        // blank lines the author put there — is kept.
        let body = self.src[body_from..body_to]
            .trim_matches('\n')
            .trim_end()
            .to_owned();
        self.expect("}")?;
        self.depth -= 1;
        if out.is_empty() {
            return self.err("a block with no rules does nothing");
        }
        if self.depth > 0 {
            return Ok(out);
        }
        self.blocks.push(crate::strategy::ast::Block {
            name,
            values,
            body,
            start: rules_so_far,
            len: per_iteration,
        });
        Ok(out)
    }

    fn stmt(&mut self) -> Result<Vec<Stmt>, ParseError> {
        if self.eat("bet") {
            if let Some(members) = self.group() {
                let a = self.amount()?;
                return Ok(members.iter().map(|b| Stmt::Bet(*b, a.clone())).collect());
            }
            let b = self.bet_ref()?;
            return Ok(vec![Stmt::Bet(b, self.amount()?)]);
        }
        if self.eat("press") {
            if let Some(members) = self.group() {
                self.expect("to")?;
                let a = self.amount()?;
                return Ok(members.iter().map(|b| Stmt::Press(*b, a.clone())).collect());
            }
            let b = self.bet_ref()?;
            self.expect("to")?;
            return Ok(vec![Stmt::Press(b, self.amount()?)]);
        }
        if self.eat("regress") {
            if let Some(members) = self.group() {
                self.expect("to")?;
                let a = self.amount()?;
                return Ok(members
                    .iter()
                    .map(|b| Stmt::Regress(*b, a.clone()))
                    .collect());
            }
            let b = self.bet_ref()?;
            self.expect("to")?;
            return Ok(vec![Stmt::Regress(b, self.amount()?)]);
        }
        if self.eat("down") {
            if let Some(members) = self.group() {
                return Ok(members.iter().map(|b| Stmt::Down(*b)).collect());
            }
            return Ok(vec![Stmt::Down(self.bet_ref()?)]);
        }
        if self.eat("working") {
            let members: Vec<BetRef> = match self.group() {
                Some(m) => m.to_vec(),
                None => vec![self.bet_ref()?],
            };
            let on = if self.eat("on") {
                true
            } else if self.eat("off") {
                false
            } else {
                return self.err("expected \"on\" or \"off\"");
            };
            return Ok(members.iter().map(|b| Stmt::Working(*b, on)).collect());
        }
        if self.eat("leave") {
            // A reason may be given; it is for the reader, not the engine.
            if self.peek().starts_with('"') {
                self.at += 1;
            }
            return Ok(vec![Stmt::Leave]);
        }
        if self.eat("set") {
            let name = self.next();
            let Some(i) = self.vars.iter().position(|v| *v == name) else {
                return Err(ParseError {
                    line: self.line(),
                    token: name,
                    what: "no such memory slot; declare it with \"var\" first".into(),
                });
            };
            self.expect("=")?;
            return Ok(vec![Stmt::Set(i as u16, self.expr()?)]);
        }
        self.err("expected an action")
    }
}

/// Read a strategy from its text form.
pub fn parse(src: &str) -> Result<Strategy, ParseError> {
    let mut p = Parser {
        toks: tokenize(src),
        src: src.to_owned(),
        blocks: Vec::new(),
        depth: 0,
        at: 0,
        vars: Vec::new(),
        bindings: Vec::new(),
    };

    p.expect("strategy")?;
    let name_tok = p.next();
    let name = name_tok
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(&name_tok)
        .to_owned();
    p.expect("language")?;
    let version = p.number()?;
    if version != LANGUAGE_VERSION as i64 {
        return Err(ParseError {
            line: p.line(),
            token: version.to_string(),
            what: format!(
                "this is grammar version {version}; this engine reads version {LANGUAGE_VERSION}"
            ),
        });
    }

    // Declarations first: memory, then pressing. Both before any rule, so a
    // rule can never refer to something declared after it.
    while p.peek().eq_ignore_ascii_case("var") {
        p.at += 1;
        let name = p.next();
        if p.eat("=") {
            let _ = p.number()?; // slots start at zero; the initial value is documentation
        }
        p.vars.push(name);
    }

    let mut progressions = [Progression::Flat; STREAMS];
    while p.peek().eq_ignore_ascii_case("press") && !p.peek_at(1).is_empty() {
        // `press <progression>` and `press <progression> for <bet>` are
        // declarations; `press <bet> to <amount>` is a statement and only
        // appears inside a rule body, which this loop has not reached.
        //
        // The per-stream form says `for` and not `on` because `on` is what
        // starts a rule: `press martingale on dont pass` and
        // `press martingale` followed by `on seven-out:` cannot be told
        // apart without knowing every trigger word, and a grammar that
        // needs that lookahead breaks the day a trigger is added.
        let Some((prog, _)) = PROGRESSION_WORDS
            .iter()
            .find(|(_, w)| p.peek_at(1).eq_ignore_ascii_case(w))
            .map(|(prog, w)| (*prog, *w))
        else {
            break;
        };
        p.at += 2;
        if p.eat("for") {
            let bet = p.bet_ref()?;
            match stream_of(bet) {
                Some(i) => progressions[i] = prog,
                None => return p.err("odds press with the flat behind them"),
            }
        } else {
            progressions = [prog; STREAMS];
        }
    }

    let rules = p.rules_from(0)?;
    let blocks = std::mem::take(&mut p.blocks);

    if rules.is_empty() {
        return p.err("a strategy with no rules never bets");
    }

    Ok(Strategy {
        name,
        vars: p.vars,
        rules,
        progressions,
        blocks,
    })
}

// ----------------------------------------------------------------- renderer

fn bet_text(b: BetRef) -> String {
    match b {
        BetRef::Pass => "pass".into(),
        BetRef::DontPass => "dont pass".into(),
        BetRef::Come => "come".into(),
        BetRef::DontCome => "dont come".into(),
        BetRef::PassOdds => "odds on pass".into(),
        BetRef::DontPassLay => "odds on dont pass".into(),
        BetRef::ComeOdds(n) => format!("odds on come {n}"),
        BetRef::DontComeLay(n) => format!("odds on dont come {n}"),
        BetRef::Place(n) => format!("place {n}"),
        BetRef::Hardway(n) => format!("hard {n}"),
        BetRef::Field => "field".into(),
        BetRef::AnySeven => "any seven".into(),
        BetRef::AnyCraps => "any craps".into(),
    }
}

fn read_text(r: Read) -> String {
    match r {
        Read::Point => "point".into(),
        Read::ComeOut => "come-out".into(),
        Read::LastTotal => "last-total".into(),
        Read::Roll => "roll".into(),
        Read::RollsThisShooter => "rolls-this-shooter".into(),
        Read::Shooter => "shooter".into(),
        Read::Cash => "cash".into(),
        Read::Wealth => "wealth".into(),
        Read::Profit => "profit".into(),
        Read::PeakProfit => "peak-profit".into(),
        Read::Drawdown => "drawdown".into(),
        Read::Handle => "handle".into(),
        Read::LiveCome => "live-come".into(),
        Read::LiveDontCome => "live-dont-come".into(),
        Read::OnTableFace => "on-table-face".into(),
        Read::Stake(b) => format!("stake({})", bet_text(b)),
        Read::Up(b) => format!("up({})", bet_text(b)),
        Read::Wins(b) => format!("wins({})", bet_text(b)),
        Read::Losses(b) => format!("losses({})", bet_text(b)),
        Read::Streak(b) => format!("streak({})", bet_text(b)),
        Read::Paid(b) => format!("paid({})", bet_text(b)),
        Read::Hits(n) => format!("hits({n})"),
        Read::HitsThisShooter(n) => format!("hits-this-shooter({n})"),
        Read::ComePoint(n) => format!("come-point({n})"),
        Read::DontComePoint(n) => format!("dont-come-point({n})"),
    }
}

fn op_text(o: BinOp) -> &'static str {
    match o {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::Min | BinOp::Max => unreachable!("rendered as a call"),
    }
}

/// How tightly an operator binds, so an expression can be written the way a
/// person would write it instead of drowned in brackets.
fn prec(o: BinOp) -> u8 {
    match o {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => 3,
        BinOp::Add | BinOp::Sub => 4,
        BinOp::Mul | BinOp::Div => 5,
        // Rendered as calls; they never need brackets of their own.
        BinOp::Min | BinOp::Max => u8::MAX,
    }
}

#[inline]
fn is_comparison(o: BinOp) -> bool {
    matches!(
        o,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne
    )
}

/// Expressions render with the brackets they need and no others.
///
/// The first version wrote every operator fully parenthesized on the
/// grounds that the tree is the truth and the text is its serialization.
/// Then the Bench put those rules on screen for a person to read, and
/// `on roll when ((point != 0) and up(pass)):` is not what anyone would
/// write. Precedence is the parser's already; this only has to agree with
/// it, and the round-trip law is what checks that it does.
fn expr_text(e: &Expr, vars: &[String]) -> String {
    expr_prec(e, vars, 0)
}

fn expr_prec(e: &Expr, vars: &[String], parent: u8) -> String {
    match e {
        Expr::Const(v) => v.to_string(),
        Expr::Var(i) => vars
            .get(*i as usize)
            .cloned()
            .unwrap_or_else(|| format!("var{i}")),
        Expr::Read(r) => read_text(*r),
        // `not` and unary minus bind tighter than any binary operator, so
        // an operand that is one needs brackets.
        Expr::Not(a) => format!("not {}", expr_prec(a, vars, u8::MAX)),
        // A negated literal keeps its brackets: `-20000` would read back as
        // a negative constant, which is a different tree from a negation
        // applied to a positive one, and the law is about the tree.
        Expr::Neg(a) => match a.as_ref() {
            Expr::Const(v) => format!("-({v})"),
            other => format!("-{}", expr_prec(other, vars, u8::MAX)),
        },
        Expr::Bin(BinOp::Min, a, b) => {
            format!("min({}, {})", expr_prec(a, vars, 0), expr_prec(b, vars, 0))
        }
        Expr::Bin(BinOp::Max, a, b) => {
            format!("max({}, {})", expr_prec(a, vars, 0), expr_prec(b, vars, 0))
        }
        Expr::Bin(o, a, b) => {
            let p = prec(*o);
            // Arithmetic and the connectives are left-associative, so only a
            // right operand of equal precedence needs bracketing: `a - (b -
            // c)` is not `a - b - c`. Comparisons are non-associative — the
            // parser reads at most one per expression — so a comparison on
            // either side of a comparison needs its own brackets, or
            // `a < b < c` comes back as something the grammar cannot read.
            let left = if is_comparison(*o) { p + 1 } else { p };
            let text = format!(
                "{} {} {}",
                expr_prec(a, vars, left),
                op_text(*o),
                expr_prec(b, vars, p + 1)
            );
            if p < parent {
                format!("({text})")
            } else {
                text
            }
        }
    }
}

fn amount_text(a: &AmountExpr, vars: &[String]) -> String {
    match a {
        AmountExpr::Base => "base".into(),
        AmountExpr::Pressed => "pressed".into(),
        AmountExpr::MaxOdds => "max".into(),
        AmountExpr::Units(e) => format!("{} units", expr_text(e, vars)),
        // A bare amount is already cents — the unit this engine has — so
        // saying so adds a word and no information.
        AmountExpr::Cents(e) => expr_text(e, vars),
    }
}

/// Join a statement's words, dropping an amount that renders to nothing.
fn join(verb: &str, subject: String, amount: String) -> String {
    if amount.is_empty() {
        format!("{verb} {subject}")
    } else {
        format!("{verb} {subject} {amount}")
    }
}

fn trigger_text(t: Trigger) -> String {
    match t {
        Trigger::SessionStart => "session-start".into(),
        Trigger::ComeOut => "come-out".into(),
        Trigger::PointEstablished => "point-established".into(),
        Trigger::PointMade => "point-made".into(),
        Trigger::SevenOut => "seven-out".into(),
        Trigger::Roll => "roll".into(),
        Trigger::Total(n) => format!("total({n})"),
        Trigger::ComePointEstablished(n) => format!("come point on {n}"),
        Trigger::DontComePointEstablished(n) => format!("dont come point on {n}"),
        Trigger::Win(b) => format!("win of {}", bet_text(b)),
        Trigger::Loss(b) => format!("loss of {}", bet_text(b)),
    }
}

fn stmt_text(s: &Stmt, vars: &[String]) -> String {
    match s {
        // Only `bet` drops the default: `press place 6 to` with nothing
        // after it is a dangling sentence, whatever the parser makes of it.
        Stmt::Bet(b, a) => join(
            "bet",
            bet_text(*b),
            match a {
                AmountExpr::Pressed => String::new(),
                other => amount_text(other, vars),
            },
        ),
        Stmt::Press(b, a) => join(
            "press",
            format!("{} to", bet_text(*b)),
            amount_text(a, vars),
        ),
        Stmt::Regress(b, a) => join(
            "regress",
            format!("{} to", bet_text(*b)),
            amount_text(a, vars),
        ),
        Stmt::Down(b) => format!("down {}", bet_text(*b)),
        Stmt::Working(b, on) => format!(
            "working {} {}",
            bet_text(*b),
            if *on { "on" } else { "off" }
        ),
        Stmt::Leave => "leave".into(),
        Stmt::Set(i, e) => format!(
            "set {} = {}",
            vars.get(*i as usize)
                .cloned()
                .unwrap_or_else(|| format!("var{i}")),
            expr_text(e, vars)
        ),
    }
}

/// Whether a block still describes the rules sitting where it produced
/// them — asked, never remembered.
///
/// Re-reads the block's own text once per value and compares. A block left
/// alone still holds; one whose iterations have been edited apart does not,
/// and stops being a block at that moment without anything having to notice.
pub fn block_holds(s: &Strategy, b: &crate::strategy::ast::Block) -> bool {
    let span = b.len * b.values.len();
    if b.len == 0 || b.start + span > s.rules.len() {
        return false;
    }
    for (k, v) in b.values.iter().enumerate() {
        let src = format!(
            "strategy \"x\" language {LANGUAGE_VERSION}\nfor each of {v} as {} {{\n{}\n}}\n",
            b.name, b.body
        );
        let Ok(mut one) = parse(&src) else {
            return false;
        };
        one.vars.clone_from(&s.vars);
        // Re-parsing without the strategy's memory declared would fail on
        // any rule that touches it, so parse again with them in scope.
        let src = format!(
            "strategy \"x\" language {LANGUAGE_VERSION}\n{}\nfor each of {v} as {} {{\n{}\n}}\n",
            s.vars
                .iter()
                .map(|n| format!("var {n} = 0"))
                .collect::<Vec<_>>()
                .join("\n"),
            b.name,
            b.body
        );
        let Ok(one) = parse(&src) else {
            return false;
        };
        if one.rules.len() != b.len {
            return false;
        }
        let here = &s.rules[b.start + k * b.len..b.start + (k + 1) * b.len];
        if one.rules != here {
            return false;
        }
    }
    true
}

/// Drop blocks that no longer describe their rules.
///
/// Called before rendering, so a strategy whose iterations were edited
/// apart writes itself out as the rules it now is — and so
/// `parse(render(s)) == s` stays a law rather than failing on a record that
/// had already stopped being true.
pub fn prune_blocks(s: &mut Strategy) {
    let kept: Vec<_> = s
        .blocks
        .iter()
        .filter(|b| block_holds(s, b))
        .cloned()
        .collect();
    s.blocks = kept;
}

/// Write a strategy out in the form [`parse`] reads back.
pub fn render(s: &Strategy) -> String {
    let mut out = format!("strategy \"{}\" language {LANGUAGE_VERSION}\n", s.name);

    if !s.vars.is_empty() {
        out.push('\n');
        for v in &s.vars {
            out += &format!("var {v} = 0\n");
        }
    }

    // One line if every stream presses the same way, which is what a
    // checkbox player means; otherwise one line per stream that differs.
    let all_same = s.progressions.iter().all(|p| *p == s.progressions[0]);
    if all_same {
        if s.progressions[0] != Progression::Flat {
            out += &format!("\npress {}\n", progression_word(s.progressions[0]));
        }
    } else {
        out.push('\n');
        out += &format!("press {}\n", progression_word(Progression::Flat));
        for (i, p) in s.progressions.iter().enumerate() {
            if *p != Progression::Flat {
                if let Some(b) = stream_bet(i) {
                    out += &format!("press {} for {}\n", progression_word(*p), bet_text(b));
                }
            }
        }
    }

    // Rules in order, except where a block still describes a run of them —
    // then the block, in the words it was written in.
    let mut held = s.clone();
    prune_blocks(&mut held);
    let mut i = 0usize;
    while i < s.rules.len() {
        if let Some(b) = held.blocks.iter().find(|b| b.start == i) {
            out += &format!(
                "\nfor each of {} as {} {{\n{}\n}}\n",
                b.values
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                b.name,
                b.body
            );
            i += b.len * b.values.len();
            continue;
        }
        let r = &s.rules[i];
        out += &format!("\non {}", trigger_text(r.trigger));
        if let Some(g) = &r.guard {
            out += &format!(" when {}", expr_text(g, &s.vars));
        }
        out += ":\n";
        for st in &r.body {
            out += &format!("    {}\n", stmt_text(st, &s.vars));
        }
        i += 1;
    }
    out
}

/// One rule, on its own, as the Bench and the editor show it.
///
/// The whole-strategy [`render`] is the save format; this is the same words
/// for a single row, so a highlighted rule reads exactly as it was written.
pub fn render_rule(s: &Strategy, index: usize) -> String {
    let Some(r) = s.rules.get(index) else {
        return String::new();
    };
    let mut out = format!("on {}", trigger_text(r.trigger));
    if let Some(g) = &r.guard {
        out += &format!(" when {}", expr_text(g, &s.vars));
    }
    out += ": ";
    out += &r
        .body
        .iter()
        .map(|st| stmt_text(st, &s.vars))
        .collect::<Vec<_>>()
        .join("; ");
    out
}

/// A bet's name, as the language spells it.
pub fn bet_name(b: BetRef) -> String {
    bet_text(b)
}

/// The bet that names a stream, for writing per-stream pressing back out.
fn stream_bet(i: usize) -> Option<BetRef> {
    Some(match i {
        0 => BetRef::Pass,
        1 => BetRef::DontPass,
        2 => BetRef::Come,
        3 => BetRef::DontCome,
        4 => BetRef::Field,
        5..=10 => BetRef::Place(crate::PLACE_NUMS[i - 5]),
        11..=14 => BetRef::Hardway(crate::HARD_NUMS[i - 11]),
        15 => BetRef::AnySeven,
        16 => BetRef::AnyCraps,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bets::{BetSelection, OddsPolicy, Rules};
    use crate::strategy::{compile, from_selection};

    fn rules() -> Rules {
        Rules {
            odds_policy: OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
        }
    }

    /// The law. Everything else in this module exists to keep it true.
    fn round_trip(s: &Strategy) {
        // A block record that has stopped being true is not part of what
        // the text says, so the law is stated over the pruned strategy.
        let mut s = s.clone();
        prune_blocks(&mut s);
        let s = &s;
        let text = render(s);
        let back = parse(&text)
            .unwrap_or_else(|e| panic!("{}\n--- could not read back ---\n{text}", e.message()));
        assert_eq!(
            *s, back,
            "the tree did not survive the round trip\n--- rendered ---\n{text}"
        );
        // And the text form is itself stable: rendering what was parsed
        // gives the same bytes, so a saved strategy does not churn.
        assert_eq!(text, render(&back), "rendering is not idempotent");
    }

    #[test]
    fn every_checkbox_player_round_trips() {
        let r = rules();
        for (name, sel) in crate::sweep::explore_strategies() {
            for prog in crate::bets::Progression::ALL {
                let sel = BetSelection {
                    progression: prog,
                    ..sel.clone()
                };
                let s = from_selection(&sel, &r);
                let text = render(&s);
                let back = parse(&text)
                    .unwrap_or_else(|e| panic!("{name} + {prog:?}: {}\n{text}", e.message()));
                assert_eq!(s, back, "{name} + {prog:?}\n{text}");
            }
        }
    }

    /// A strategy read from text must not merely parse — it must play the
    /// same session. This is the round-trip law extended through the
    /// compiler and the engine to the money.
    #[test]
    fn a_parsed_strategy_plays_the_same_session() {
        let r = rules();
        for (name, sel) in crate::sweep::explore_strategies() {
            let original = from_selection(&sel, &r);
            let reparsed = parse(&render(&original)).unwrap();
            let (a, b) = (compile(&original).unwrap(), compile(&reparsed).unwrap());
            assert_eq!(a.hash, b.hash, "{name}: different compiled form");
            for seed in 0..200u64 {
                let x = crate::session::run_program_session(
                    &a, &r, 1000, 30_000, None, 200_000, 400, seed,
                );
                let y = crate::session::run_program_session(
                    &b, &r, 1000, 30_000, None, 200_000, 400, seed,
                );
                assert_eq!(
                    (x.ruin.rolls, x.horizon.final_cents),
                    (y.ruin.rolls, y.horizon.final_cents),
                    "{name} seed {seed}"
                );
            }
        }
    }

    #[test]
    fn hand_written_text_reads() {
        let src = r#"
strategy "44 Inside, regressed" language 1

var hits = 0

press half-press for place 6

# Cover the inside numbers once a point is on.
on point-established:
    bet place 5 2 units
    bet place 6 2 units
    bet place 8 2 units
    bet place 9 2 units

on win of place 6:
    set hits = hits + 1
    press place 6 to stake(place 6) * 2

on roll when hits >= 4:
    down place 5
    down place 9
    working place 6 off

on seven-out:
    set hits = 0

on roll when profit <= -$200 or profit >= $150:
    leave "enough"
"#;
        let s = parse(src).expect("should read");
        assert_eq!(s.name, "44 Inside, regressed");
        assert_eq!(s.vars, vec!["hits".to_string()]);
        assert_eq!(s.rules.len(), 5);
        assert_eq!(
            s.progressions[crate::strategy::view::S_PLACE + 2],
            Progression::HalfPress,
            "per-stream pressing survived"
        );
        assert_eq!(s.progressions[0], Progression::Flat, "and nothing else did");
        round_trip(&s);
        compile(&s).expect("and it compiles");
    }

    /// The loop the specification promised in §4 and never had. Six
    /// near-identical rules — the shape that made 3-Point Molly nine rules,
    /// six of which differed only in a number — become one block.
    #[test]
    fn a_block_writes_a_rule_once_and_produces_one_per_number() {
        let s = parse(
            "strategy \"molly\" language 1\n\
             on come-out:\n    bet pass\n\
             on roll when point != 0 and live-come < 2:\n    bet come\n\
             for each of 4, 5, 6, 8, 9, 10 as n {\n\
                 on roll when come-point(n):\n    bet odds on come n max\n\
             }\n",
        )
        .unwrap_or_else(|e| panic!("{}", e.message()));
        assert_eq!(s.rules.len(), 8, "two written plus six expanded");

        // The expansion is exactly what somebody would have typed by hand.
        let long = parse(
            "strategy \"molly\" language 1\n\
             on come-out:\n    bet pass\n\
             on roll when point != 0 and live-come < 2:\n    bet come\n\
             on roll when come-point(4):\n    bet odds on come 4 max\n\
             on roll when come-point(5):\n    bet odds on come 5 max\n\
             on roll when come-point(6):\n    bet odds on come 6 max\n\
             on roll when come-point(8):\n    bet odds on come 8 max\n\
             on roll when come-point(9):\n    bet odds on come 9 max\n\
             on roll when come-point(10):\n    bet odds on come 10 max\n",
        )
        .unwrap();
        // The rules are the same rules. The strategies differ in one way
        // and only one: the block records that a person wrote them once,
        // which is exactly the thing that must survive a save and a reload.
        assert_eq!(s.rules, long.rules, "a block is the rules it produces");
        assert!(long.blocks.is_empty());
        assert_eq!(s.blocks.len(), 1);

        // And it comes back out as a block, not as the six rules it stands
        // for — which is the difference between sugar that is preserved and
        // sugar that is merely accepted.
        round_trip(&s);
        assert!(
            render(&s).contains("for each of 4, 5, 6, 8, 9, 10 as n"),
            "the block did not survive rendering:\n{}",
            render(&s)
        );
    }

    #[test]
    fn a_binding_reaches_every_place_a_number_can_go() {
        let s = parse(
            "strategy \"b\" language 1\n\
             for each of 6, 8 as n {\n\
                 on win of place n when hits-this-shooter(n) <= 2:\n\
                     press place n to stake(place n) * 2\n\
                 on total(n):\n    bet hard n\n\
                 on roll when point != n:\n    bet place n\n\
             }\n",
        )
        .unwrap_or_else(|e| panic!("{}", e.message()));
        assert_eq!(s.rules.len(), 6);
        // The binding reached every position: check the tree, because the
        // text now shows the block rather than what it expanded to.
        let expanded = render(&Strategy {
            blocks: Vec::new(),
            ..s.clone()
        });
        for want in [
            "on win of place 6 when hits-this-shooter(6) <= 2",
            "press place 6 to stake(place 6) * 2",
            "on total(8)",
            "bet hard 8",
            "on roll when point != 8",
        ] {
            assert!(expanded.contains(want), "missing {want:?} in\n{expanded}");
        }
        // And the written form is what comes back.
        assert!(render(&s).contains("for each of 6, 8 as n"));
        round_trip(&s);
    }

    #[test]
    fn blocks_nest_and_bindings_shadow_innermost_first() {
        let s = parse(
            "strategy \"n\" language 1\n\
             for each of 6, 8 as a {\n\
                 for each of 4, 10 as b {\n\
                     on roll when point != a and point != b:\n    bet place a\n\
                 }\n\
             }\n",
        )
        .unwrap_or_else(|e| panic!("{}", e.message()));
        assert_eq!(s.rules.len(), 4, "two outer times two inner");
        // Only the outer block is recorded; the inner one lives inside its
        // text. Two records claiming rule 0 would render as the inner block
        // followed by loose rules.
        assert_eq!(s.blocks.len(), 1);
        assert_eq!(s.blocks[0].name, "a");
        let expanded = render(&Strategy {
            blocks: Vec::new(),
            ..s.clone()
        });
        assert!(expanded.contains("point != 6 and point != 4"), "{expanded}");
        assert!(
            expanded.contains("point != 8 and point != 10"),
            "{expanded}"
        );
        round_trip(&s);
    }

    #[test]
    fn a_malformed_block_is_refused_in_words() {
        for (src, want) in [
            (
                "strategy \"x\" language 1\nfor each of 6, 8 as n {\non roll:\n bet place n\n",
                "}",
            ),
            (
                "strategy \"x\" language 1\nvar n = 0\nfor each of 6 as n {\non roll:\n bet place n\n}\n",
                "memory slot",
            ),
            (
                "strategy \"x\" language 1\nfor each of 6 as 7 {\non roll:\n bet pass\n}\n",
                "bind",
            ),
            (
                "strategy \"x\" language 1\nfor each of 6 as n {\n}\n",
                "no rules",
            ),
        ] {
            let e = parse(src).unwrap_err();
            assert!(
                e.message().contains(want),
                "expected {want:?}, got: {}",
                e.message()
            );
        }
    }

    /// The two gaps the ergonomics assessment found in the vocabulary,
    /// and what they let a strategy say now.
    /// The whole point of recording a block: it survives being written,
    /// saved, read back and looked at — and stops existing the moment its
    /// iterations stop agreeing, without anything having to notice.
    #[test]
    fn sugar_survives_a_look_and_dissolves_on_a_real_edit() {
        let src = "strategy \"s\" language 1\n\
                   for each of 6, 8 as n {\n\
                       # press it twice, then take the winnings\n\
                       on win of place n:\n\
                           press place n to stake(place n) * 2\n\
                   }\n";
        let s = parse(src).unwrap_or_else(|e| panic!("{}", e.message()));
        assert_eq!(s.rules.len(), 2);
        assert_eq!(s.blocks.len(), 1);
        assert!(block_holds(&s, &s.blocks[0]));

        // Written out and read back: still a block, comment included.
        let text = render(&s);
        assert!(text.contains("for each of 6, 8 as n"), "{text}");
        assert!(
            text.contains("# press it twice, then take the winnings"),
            "the author's comment did not survive:\n{text}"
        );
        assert_eq!(parse(&text).unwrap(), s);

        // Unfolded and left alone — nothing changed, so it is still a block.
        let mut looked = s.clone();
        let rules = looked.rules.clone();
        looked.rules = rules;
        assert!(block_holds(&looked, &looked.blocks[0]));
        assert!(render(&looked).contains("for each"));

        // Edited apart: one iteration now presses differently.
        let mut edited = s.clone();
        edited.rules[1].body = vec![Stmt::Regress(BetRef::Place(8), AmountExpr::Base)];
        assert!(
            !block_holds(&edited, &edited.blocks[0]),
            "two iterations that differ are not one rule"
        );
        let text = render(&edited);
        assert!(
            !text.contains("for each"),
            "the block outlived its truth:\n{text}"
        );
        assert!(text.contains("regress place 8 to base"), "{text}");

        // And the stale record is pruned, so the law still holds over it.
        let mut pruned = edited.clone();
        prune_blocks(&mut pruned);
        assert!(pruned.blocks.is_empty());
        assert_eq!(parse(&render(&pruned)).unwrap(), pruned);
    }

    #[test]
    fn a_come_point_and_a_payout_can_be_named() {
        let s = parse(
            "strategy \"v\" language 1\n\
             for each of 4, 5, 6, 8, 9, 10 as n {\n\
                 on come point on n:\n    bet odds on come n max\n\
                 on dont come point on n:\n    bet odds on dont come n max\n\
             }\n\
             on win of place 6:\n\
                 press place 6 to stake(place 6) + paid(place 6) / 2\n",
        )
        .unwrap_or_else(|e| panic!("{}", e.message()));
        assert_eq!(s.rules.len(), 13);
        round_trip(&s);
        let p = crate::strategy::compile(&s).unwrap();
        // Reading a payout is win/loss history, so the mask says so.
        assert!(p.features.has(crate::strategy::FeatureMask::STREAKS));
    }

    #[test]
    fn a_come_point_trigger_needs_a_box_number() {
        let s = parse("strategy \"v\" language 1\non come point on 7:\n    bet pass\n").unwrap();
        let e = crate::strategy::compile(&s).unwrap_err();
        assert!(e.message().contains("not a box number"), "{}", e.message());
    }

    #[test]
    fn a_group_is_one_phrase_and_several_bets() {
        let s = parse(
            "strategy \"g\" language 1\n\
             on point-established:\n\
                 bet place inside 2 units\n\
             on roll when profit > $100:\n\
                 down all place\n\
                 working all hardways off\n",
        )
        .unwrap();
        assert_eq!(s.rules[0].body.len(), 4, "inside is 5, 6, 8 and 9");
        assert_eq!(
            s.rules[1].body.len(),
            10,
            "six place bets and four hardways"
        );
        // Groups do not render back as groups — the tree holds the members,
        // and the law is about the tree.
        round_trip(&s);
    }

    #[test]
    fn money_reads_to_the_cent() {
        assert_eq!(parse_money("$44"), Some(4400));
        assert_eq!(parse_money("$44.50"), Some(4450));
        assert_eq!(parse_money("$1,200.05"), Some(120_005));
        assert_eq!(parse_money("1200"), Some(1200));
        assert_eq!(parse_money("-200"), Some(-200));
        assert_eq!(parse_money("pass"), None);
    }

    #[test]
    fn a_wrong_grammar_version_is_refused_not_guessed() {
        let e = parse("strategy \"x\" language 99\non roll:\n bet pass base\n").unwrap_err();
        assert!(e.message().contains("version 99"), "got: {}", e.message());
    }

    #[test]
    fn errors_name_the_offending_token() {
        let cases = [
            (
                "strategy \"x\" language 1\non wobble:\n bet pass base\n",
                "wobble",
            ),
            (
                "strategy \"x\" language 1\non roll:\n bet flurb base\n",
                "flurb",
            ),
            (
                "strategy \"x\" language 1\non roll:\n bet place 7 base\n",
                "base",
            ),
            (
                "strategy \"x\" language 1\non roll:\n set nope = 1\n",
                "nope",
            ),
        ];
        for (src, token) in cases {
            let e = parse(src).unwrap_err();
            assert!(
                e.message().contains(token),
                "expected the message to name \"{token}\", got: {}",
                e.message()
            );
            assert!(e.line >= 2, "and to place it: {}", e.message());
        }
    }

    #[test]
    fn comments_and_layout_are_decorative() {
        let tight = "strategy \"x\" language 1 on roll: bet pass base";
        let loose =
            "strategy \"x\" language 1\n\n# a note\non roll:      # another\n\n    bet pass base\n";
        assert_eq!(parse(tight).unwrap(), parse(loose).unwrap());
    }

    /// Malformed input never panics and always says something useful.
    ///
    /// Not a fuzzer — the parser is not attack surface for a local
    /// single-user app, and a fuzzing harness would be machinery nobody
    /// runs. What matters is that a person mistyping a strategy gets a
    /// sentence rather than a crashed window, and that is checkable
    /// directly.
    #[test]
    fn malformed_input_is_refused_rather_than_fatal() {
        let good = "strategy \"s\" language 1\non come-out:\n    bet pass\n";
        let mut cases: Vec<String> = vec![
            String::new(),
            " ".into(),
            "\n\n\n".into(),
            "strategy".into(),
            "strategy \"unclosed".into(),
            "strategy \"s\" language".into(),
            "strategy \"s\" language one".into(),
            "strategy \"s\" language 1".into(),
            "strategy \"s\" language 1\non".into(),
            "strategy \"s\" language 1\non roll".into(),
            "strategy \"s\" language 1\non roll:".into(),
            "strategy \"s\" language 1\non roll: bet".into(),
            "strategy \"s\" language 1\non roll: bet place".into(),
            "strategy \"s\" language 1\non roll: bet place 7".into(),
            "strategy \"s\" language 1\non roll when: bet pass".into(),
            "strategy \"s\" language 1\non roll when (((: bet pass".into(),
            "strategy \"s\" language 1\non roll when 1 +: bet pass".into(),
            "strategy \"s\" language 1\non roll: press pass".into(),
            "strategy \"s\" language 1\non roll: working pass".into(),
            "strategy \"s\" language 1\non win of odds on pass: bet pass".into(),
            "strategy \"s\" language 1\nvar x = \non roll: bet pass".into(),
            "\u{0}\u{1}\u{7f}".into(),
            "strategy \"s\" language 1\non roll: bet pass ".to_owned() + &"(".repeat(500),
            "strategy \"s\" language 1\non roll: bet pass 999999999999999999999".into(),
        ];
        // Every truncation of a valid strategy, which is what a half-typed
        // one looks like.
        for k in 0..good.len() {
            cases.push(good[..k].to_owned());
        }
        // And every single-byte deletion, which is what a typo looks like.
        for k in 0..good.len() {
            let mut c = good.to_owned();
            c.remove(k);
            cases.push(c);
        }
        for src in cases {
            match parse(&src) {
                Ok(s) => {
                    // Anything that parses must also render and read back.
                    let text = render(&s);
                    assert!(parse(&text).is_ok(), "re-read failed for {src:?}");
                }
                Err(e) => {
                    let m = e.message();
                    assert!(m.starts_with("line "), "{src:?} -> {m}");
                    assert!(e.line >= 1, "{src:?} -> {m}");
                }
            }
        }
    }

    /// Randomized trees, so the law is tested against shapes nobody thought
    /// to write by hand.
    #[test]
    fn the_law_holds_over_randomized_strategies() {
        struct Lcg(u64);
        impl Lcg {
            fn next(&mut self) -> u64 {
                self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
                self.0 >> 33
            }
            fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
                xs[(self.next() as usize) % xs.len()]
            }
        }
        let mut g = Lcg(12345);
        const BETS: [BetRef; 9] = [
            BetRef::Pass,
            BetRef::DontPass,
            BetRef::Come,
            BetRef::Field,
            BetRef::Place(6),
            BetRef::Place(10),
            BetRef::Hardway(8),
            BetRef::AnySeven,
            BetRef::PassOdds,
        ];
        const TRIGGERS: [Trigger; 7] = [
            Trigger::ComeOut,
            Trigger::Roll,
            Trigger::PointEstablished,
            Trigger::PointMade,
            Trigger::SevenOut,
            Trigger::Total(7),
            Trigger::Win(BetRef::Place(6)),
        ];
        const OPS: [BinOp; 8] = [
            BinOp::Add,
            BinOp::Sub,
            BinOp::Mul,
            BinOp::Div,
            BinOp::Min,
            BinOp::Max,
            BinOp::Lt,
            BinOp::And,
        ];
        const READS: [Read; 6] = [
            Read::Point,
            Read::Profit,
            Read::LiveCome,
            Read::Hits(8),
            Read::Streak(BetRef::Pass),
            Read::Stake(BetRef::Place(6)),
        ];

        for case in 0..400 {
            let vars = vec!["a".to_string(), "b".to_string()];
            let expr = |g: &mut Lcg, depth: u32| -> Expr {
                fn build(g: &mut Lcg, depth: u32) -> Expr {
                    if depth == 0 || g.next().is_multiple_of(3) {
                        return match g.next() % 3 {
                            0 => Expr::Const((g.next() % 20_000) as i64 - 10_000),
                            1 => Expr::Read(g.pick(&READS)),
                            _ => Expr::Var((g.next() % 2) as u16),
                        };
                    }
                    match g.next() % 6 {
                        0 => Expr::Not(Box::new(build(g, depth - 1))),
                        1 => Expr::Neg(Box::new(build(g, depth - 1))),
                        _ => Expr::bin(g.pick(&OPS), build(g, depth - 1), build(g, depth - 1)),
                    }
                }
                build(g, depth)
            };

            let n_rules = 1 + (g.next() % 4) as usize;
            let mut rules = Vec::new();
            for _ in 0..n_rules {
                let n_stmts = 1 + (g.next() % 3) as usize;
                let mut body = Vec::new();
                for _ in 0..n_stmts {
                    let b = g.pick(&BETS);
                    let amount = match g.next() % 5 {
                        0 => AmountExpr::Base,
                        1 => AmountExpr::Pressed,
                        2 => AmountExpr::MaxOdds,
                        3 => AmountExpr::Units(expr(&mut g, 1)),
                        _ => AmountExpr::Cents(expr(&mut g, 2)),
                    };
                    body.push(match g.next() % 7 {
                        0 => Stmt::Bet(b, amount),
                        1 => Stmt::Press(b, amount),
                        2 => Stmt::Regress(b, amount),
                        3 => Stmt::Down(b),
                        4 => Stmt::Working(BetRef::Place(6), g.next().is_multiple_of(2)),
                        5 => Stmt::Leave,
                        _ => Stmt::Set((g.next() % 2) as u16, expr(&mut g, 2)),
                    });
                }
                let mut r = Rule::new(g.pick(&TRIGGERS), body);
                if g.next().is_multiple_of(2) {
                    r.guard = Some(expr(&mut g, 3));
                }
                rules.push(r);
            }
            let mut progressions = [Progression::Flat; STREAMS];
            match g.next() % 3 {
                0 => {}
                1 => progressions = [g.pick(&Progression::ALL); STREAMS],
                _ => {
                    let i = (g.next() as usize) % STREAMS;
                    progressions[i] = g.pick(&Progression::ALL[1..]);
                }
            }
            let s = Strategy {
                name: format!("case {case}"),
                vars,
                rules,
                progressions,
                blocks: Vec::new(),
            };
            round_trip(&s);
        }
    }
}
