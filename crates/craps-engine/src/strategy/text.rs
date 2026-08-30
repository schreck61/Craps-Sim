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
}

fn tokenize(src: &str) -> Vec<Token> {
    let mut out = Vec::new();
    for (n, raw) in src.lines().enumerate() {
        let line = n + 1;
        let text = raw.split('#').next().unwrap_or("");
        let bytes: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            // A quoted name is one token, quotes included, so a strategy may
            // be called anything a person would call it.
            if c == '"' {
                let mut s = String::from('"');
                i += 1;
                while i < bytes.len() && bytes[i] != '"' {
                    s.push(bytes[i]);
                    i += 1;
                }
                s.push('"');
                i += 1;
                out.push(Token { text: s, line });
                continue;
            }
            // Two-character comparisons before one-character ones.
            if i + 1 < bytes.len() {
                let pair: String = bytes[i..i + 2].iter().collect();
                if matches!(pair.as_str(), "<=" | ">=" | "==" | "!=") {
                    out.push(Token { text: pair, line });
                    i += 2;
                    continue;
                }
            }
            if "():,=<>+*/".contains(c) {
                out.push(Token {
                    text: c.to_string(),
                    line,
                });
                i += 1;
                continue;
            }
            // `-200` is a literal; `a - b` is subtraction. Hyphenated words
            // are handled by the identifier scan below, which is why the
            // operator needs its spaces.
            if c == '-' {
                if bytes.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                    let start = i;
                    i += 1;
                    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == '.') {
                        i += 1;
                    }
                    out.push(Token {
                        text: bytes[start..i].iter().collect(),
                        line,
                    });
                } else {
                    out.push(Token {
                        text: "-".into(),
                        line,
                    });
                    i += 1;
                }
                continue;
            }
            if c == '$' || c.is_ascii_digit() {
                let start = i;
                let money = c == '$';
                if money {
                    i += 1;
                }
                // Thousands separators belong to money and nowhere else: a
                // bare `9267,` inside `max(9267, x)` is a number and a comma.
                while i < bytes.len()
                    && (bytes[i].is_ascii_digit() || bytes[i] == '.' || (money && bytes[i] == ','))
                {
                    i += 1;
                }
                // `1-3-2-6` is the name of a progression, not three
                // subtractions, so a number that runs straight into a hyphen
                // and more digits keeps going.
                while i + 1 < bytes.len() && bytes[i] == '-' && bytes[i + 1].is_ascii_digit() {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                out.push(Token {
                    text: bytes[start..i].iter().collect(),
                    line,
                });
                continue;
            }
            if c.is_alphabetic() || c == '_' {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_alphanumeric()
                        || bytes[i] == '_'
                        || bytes[i] == '\''
                        || (bytes[i] == '-'
                            && bytes
                                .get(i + 1)
                                .is_some_and(|d| d.is_alphanumeric() || *d == '_')))
                {
                    i += 1;
                }
                out.push(Token {
                    text: bytes[start..i].iter().collect(),
                    line,
                });
                continue;
            }
            out.push(Token {
                text: c.to_string(),
                line,
            });
            i += 1;
        }
    }
    out
}

// ------------------------------------------------------------------- parser

struct Parser {
    toks: Vec<Token>,
    at: usize,
    vars: Vec<String>,
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
        if let Some(v) = parse_money(&t) {
            self.at += 1;
            return Ok(v);
        }
        self.err("expected a number")
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
                    _ => Read::Streak(b),
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
        const STARTERS: [&str; 8] = [
            "on", "bet", "press", "regress", "down", "working", "leave", "set",
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
        at: 0,
        vars: Vec::new(),
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

    let mut rules = Vec::new();
    while !p.done() {
        p.expect("on")?;
        let trigger = p.trigger()?;
        let guard = if p.eat("when") { Some(p.expr()?) } else { None };
        p.expect(":")?;

        let mut body = Vec::new();
        // A body runs until the next rule begins. `on` is the only word that
        // can start a rule and never starts a statement, which is what makes
        // the indentation decorative rather than load-bearing.
        while !p.done() && !p.peek().eq_ignore_ascii_case("on") {
            body.extend(p.stmt()?);
        }
        if body.is_empty() {
            return p.err("a rule with no actions does nothing");
        }
        let mut r = Rule::new(trigger, body);
        r.guard = guard;
        rules.push(r);
    }

    if rules.is_empty() {
        return p.err("a strategy with no rules never bets");
    }

    Ok(Strategy {
        name,
        vars: p.vars,
        rules,
        progressions,
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

    for r in &s.rules {
        out += &format!("\non {}", trigger_text(r.trigger));
        if let Some(g) = &r.guard {
            out += &format!(" when {}", expr_text(g, &s.vars));
        }
        out += ":\n";
        for st in &r.body {
            out += &format!("    {}\n", stmt_text(st, &s.vars));
        }
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
            };
            round_trip(&s);
        }
    }
}
