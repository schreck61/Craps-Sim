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
        self.to_string()
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.token.is_empty() {
            write!(f, "line {}: {}", self.line, self.what)
        } else {
            write!(
                f,
                "line {}: {} — found \"{}\"",
                self.line, self.what, self.token
            )
        }
    }
}

impl std::error::Error for ParseError {}

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
                //
                // And even in money a comma is only a separator when it
                // actually separates thousands — three digits with no fourth
                // behind them. Taking every comma made `min($5, x)` read as
                // the single token `$5,` and swallow the argument list's own
                // punctuation, which is a sentence a person would write.
                let thousands = |i: usize| {
                    money
                        && cs[i] == ','
                        && cs.len() > i + 3
                        && cs[i + 1..i + 4].iter().all(char::is_ascii_digit)
                        && !cs.get(i + 4).is_some_and(char::is_ascii_digit)
                };
                while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.' || thousands(i)) {
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
    /// How deep the expression walk currently is.
    ///
    /// Descent costs a native stack frame per level, and strategy text
    /// arrives by paste and by hand-edited file — so a thousand open
    /// parentheses would take the process down before any of the compiler's
    /// bounds could refuse them. This counter is what makes the parser
    /// itself total.
    expr_depth: usize,
}

/// How deeply an expression may nest before the parser refuses it.
///
/// Generous on purpose: parentheses cost nesting without costing operand
/// depth, so this sits well above the compiler's [`STACK_DEPTH`] and still
/// far below anything that could exhaust a stack.
///
/// [`STACK_DEPTH`]: crate::strategy::program
const MAX_EXPR_DEPTH: usize = 64;

/// How deeply `for each` blocks may nest.
const MAX_BLOCK_DEPTH: usize = 8;

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

    /// A number in a hole the grammar constrains — a box number, a dice
    /// total.
    ///
    /// Two things this does that a bare `as u8` did not. It refuses a value
    /// that does not fit rather than wrapping it into one that does, so
    /// `place 260` is an error instead of a place 4 nobody asked for. And it
    /// names the number in the message, because the offending token is the
    /// one the author typed, not whatever word happened to follow it.
    fn checked_number(&mut self, ok: impl Fn(u8) -> bool, what: &str) -> Result<u8, ParseError> {
        let line = self.line();
        let token = self.peek().to_owned();
        let v = self.number()?;
        match u8::try_from(v) {
            Ok(n) if ok(n) => Ok(n),
            _ => Err(ParseError {
                line,
                token,
                what: what.to_owned(),
            }),
        }
    }

    /// A box number: the six a place bet, a come point, or odds can sit on.
    fn box_number(&mut self) -> Result<u8, ParseError> {
        self.checked_number(
            |n| crate::place_index(n).is_some(),
            "not a box number (4, 5, 6, 8, 9 or 10)",
        )
    }

    /// A dice total, which is what the two dice can add up to.
    fn dice_total(&mut self) -> Result<u8, ParseError> {
        self.checked_number(|n| (2..=12).contains(&n), "not a dice total (2 through 12)")
    }

    /// Guard one level of expression descent.
    fn descend(&mut self) -> Result<(), ParseError> {
        self.expr_depth += 1;
        if self.expr_depth > MAX_EXPR_DEPTH {
            self.expr_depth -= 1;
            return self.err("this expression nests too deeply to read");
        }
        Ok(())
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
            Some((w, f)) => (w, f),
            None => (clean.as_str(), ""),
        };
        // A third decimal place is not a rounding question, it is a figure
        // this table cannot take: `$4.999` silently became $4.99, which is
        // money the author did not write.
        if frac.len() > 2 {
            return None;
        }
        let w: i64 = whole.parse().ok()?;
        let f: i64 = format!("{frac:0<2}").parse().ok()?;
        // Dollars past a hundredth of the type overflow the cents they
        // convert to. Under the test profile that panicked; in release it
        // wrapped to a negative constant, which is a stake nobody wrote and
        // a strategy nobody could debug.
        w.checked_mul(100)?.checked_add(if w < 0 { -f } else { f })
    } else {
        t.parse::<i64>().ok()
    }
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

/// Exhaustive on purpose: a fallback here would render a progression this
/// table has not learned to spell as `flat`, and the strategy would come
/// back off disk playing a different system than the one saved.
fn progression_word(p: Progression) -> &'static str {
    match p {
        Progression::Flat => "flat",
        Progression::FullPress => "full-press",
        Progression::HalfPress => "half-press",
        Progression::PressAndPull => "press-and-pull",
        Progression::Paroli3 => "paroli-3",
        Progression::S1326 => "1-3-2-6",
        Progression::Martingale => "martingale",
        Progression::GrandMartingale => "grand-martingale",
        Progression::DAlembert => "dalembert",
        Progression::ReverseDAlembert => "reverse-dalembert",
        Progression::Fibonacci => "fibonacci",
        Progression::OscarsGrind => "oscars-grind",
    }
}

/// Words the grammar already owns.
///
/// A memory slot may not take one. `set roll = 1` would write the slot while
/// every *read* of `roll` returned the table's own value, so the name would
/// mean two different things in one strategy and neither would be wrong
/// anywhere the author could see it.
/// Whether this reference names one of the four odds bets.
///
/// They are real in `bet odds on pass`, where the table tops them up toward
/// a target, and nowhere else: they keep no win/loss record of their own and
/// no pressing system of their own, because both belong to the flat they
/// sit behind.
const fn is_odds_ref(bet: BetRef) -> bool {
    matches!(
        bet,
        BetRef::PassOdds | BetRef::DontPassLay | BetRef::ComeOdds(_) | BetRef::DontComeLay(_)
    )
}

/// Only the words that are read *bare*. `hits`, `streak`, `paid` and the
/// rest of the parameterized reads are recognized only when a `(` follows,
/// so `var hits` is a perfectly good memory slot and always was — reserving
/// those would take names people actually want, for no reason at all.
const RESERVED_WORDS: [&str; 40] = [
    // reads spelled as a bare word
    "point",
    "come-out",
    "last-total",
    "roll",
    "rolls-this-shooter",
    "shooter",
    "cash",
    "wealth",
    "profit",
    "peak-profit",
    "drawdown",
    "handle",
    "buy-in",
    "table-min",
    "table-max",
    "live-come",
    "live-dont-come",
    "on-table-face",
    // operators and amounts
    "min",
    "max",
    "not",
    "and",
    "or",
    "base",
    "pressed",
    "units",
    "unit",
    "cents",
    // structure
    "strategy",
    "language",
    "var",
    "on",
    "for",
    "when",
    "each",
    "as",
    "to",
    "by",
    "with",
    "of",
];

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
            let n = self.checked_number(
                |n| crate::place_index(n).is_some(),
                "not a place number (4, 5, 6, 8, 9 or 10)",
            )?;
            return Ok(BetRef::Place(n));
        }
        if self.eat("hard") {
            let n = self.checked_number(
                |n| crate::hard_index(n).is_some(),
                "not a hardway number (4, 6, 8 or 10)",
            )?;
            return Ok(BetRef::Hardway(n));
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
            return Ok(BetRef::DontComeLay(self.box_number()?));
        }
        if self.eat("come") {
            return Ok(BetRef::ComeOdds(self.box_number()?));
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
            let n = self.dice_total()?;
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
            return Ok(Trigger::ComePointEstablished(self.box_number()?));
        }
        if self.peek().eq_ignore_ascii_case("dont")
            && self.peek_at(1).eq_ignore_ascii_case("come")
            && self.peek_at(2).eq_ignore_ascii_case("point")
        {
            self.at += 3;
            self.expect("on")?;
            return Ok(Trigger::DontComePointEstablished(self.box_number()?));
        }
        if self.eat("win") {
            self.expect("of")?;
            return Ok(Trigger::Win(self.recorded_bet_ref("wins and loses")?));
        }
        if self.eat("loss") {
            self.expect("of")?;
            return Ok(Trigger::Loss(self.recorded_bet_ref("wins and loses")?));
        }
        self.err("expected a trigger")
    }

    /// A bet that keeps a record of its own.
    ///
    /// Odds do not: they resolve with the flat they back, and the engine
    /// books the result against that flat. `on win of odds on pass` therefore
    /// meant exactly `on win of pass`, and `paid(odds on pass)` read the line
    /// bet's payout — a distinction the grammar drew and the semantics did
    /// not have, which is the kind of quiet disagreement Principle 4 exists
    /// to refuse.
    fn recorded_bet_ref(&mut self, what: &str) -> Result<BetRef, ParseError> {
        let line = self.line();
        let token = self.peek().to_owned();
        let bet = self.bet_ref()?;
        if is_odds_ref(bet) {
            return Err(ParseError {
                line,
                token,
                what: format!("odds have no record of their own; the bet behind them {what}"),
            });
        }
        Ok(bet)
    }
}

impl Parser {
    fn expr(&mut self) -> Result<Expr, ParseError> {
        self.descend()?;
        let e = self.or_expr();
        self.expr_depth -= 1;
        e
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
        // `not not not …` and `- - - …` recurse here without passing through
        // `expr`, so this arm counts its own descent.
        if self.eat("not") {
            self.descend()?;
            let e = self.unary().map(|e| Expr::Not(Box::new(e)));
            self.expr_depth -= 1;
            return e;
        }
        if self.eat("-") {
            self.descend()?;
            let e = self.unary().map(|e| Expr::Neg(Box::new(e)));
            self.expr_depth -= 1;
            return e;
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
        // Reads that take a bet. Each word carries the read it makes and
        // whether odds may stand in it — paired here rather than matched
        // against a positional code twice, which is a table that breaks
        // silently the day somebody inserts a row in the middle.
        type MakeRead = fn(BetRef) -> Read;
        const ON_THE_FELT: [(&str, MakeRead); 3] = [
            ("stake", Read::Stake),
            ("up", Read::Up),
            ("working", Read::Working),
        ];
        const IN_THE_RECORD: [(&str, MakeRead); 4] = [
            ("wins", Read::Wins),
            ("losses", Read::Losses),
            ("streak", Read::Streak),
            ("paid", Read::Paid),
        ];
        for (word, make) in ON_THE_FELT {
            if self.peek().eq_ignore_ascii_case(word) && self.peek_at(1) == "(" {
                self.at += 2;
                // What is on the felt is a question odds answer for
                // themselves.
                let b = self.bet_ref()?;
                self.expect(")")?;
                return Ok(Expr::Read(make(b)));
            }
        }
        for (word, make) in IN_THE_RECORD {
            if self.peek().eq_ignore_ascii_case(word) && self.peek_at(1) == "(" {
                self.at += 2;
                // What a bet has done is a question odds have no record of.
                let b = self.recorded_bet_ref("keeps the record")?;
                self.expect(")")?;
                return Ok(Expr::Read(make(b)));
            }
        }
        // Reads that take a number. Hits are counted per dice total; come
        // points sit on box numbers. Both index fixed-size tables, so
        // neither may be handed a number the table has no room for — which
        // is why the range each one wants travels beside it.
        type MakeNumRead = fn(u8) -> Read;
        const BY_TOTAL: [(&str, MakeNumRead); 2] = [
            ("hits", Read::Hits),
            ("hits-this-shooter", Read::HitsThisShooter),
        ];
        const BY_BOX: [(&str, MakeNumRead); 2] = [
            ("come-point", Read::ComePoint),
            ("dont-come-point", Read::DontComePoint),
        ];
        for (word, make) in BY_TOTAL {
            if self.peek().eq_ignore_ascii_case(word) && self.peek_at(1) == "(" {
                self.at += 2;
                let n = self.dice_total()?;
                self.expect(")")?;
                return Ok(Expr::Read(make(n)));
            }
        }
        for (word, make) in BY_BOX {
            if self.peek().eq_ignore_ascii_case(word) && self.peek_at(1) == "(" {
                self.at += 2;
                let n = self.box_number()?;
                self.expect(")")?;
                return Ok(Expr::Read(make(n)));
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
            ("buy-in", Read::BuyIn),
            ("table-min", Read::TableMin),
            ("table-max", Read::TableMax),
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

    /// Where a press or a regress is heading: a stake to land on, or a step
    /// to take from wherever the bet stands.
    ///
    /// `press place 6 by $6` is the sentence a player actually says at a
    /// table — "press it" is a step, not a destination, and computing the
    /// destination by hand (`to stake(place 6) + 600`, with the payout unit
    /// worked out in cents) is the language asking the author to do the
    /// table's arithmetic.
    fn press_target(&mut self, verb: &str) -> Result<PressTarget, ParseError> {
        if self.eat("by") {
            return Ok(PressTarget::By(self.required_amount(verb)?));
        }
        self.expect("to")?;
        Ok(PressTarget::To(self.required_amount(verb)?))
    }

    /// An amount that has to be written down.
    ///
    /// Leaving it off `bet` means "whatever this stream presses to", which is
    /// how a player says it. Leaving it off `press … to` is a sentence that
    /// stopped mid-word, and giving it the same silent default would hand an
    /// obviously unfinished line a meaning its author never chose.
    fn required_amount(&mut self, verb: &str) -> Result<AmountExpr, ParseError> {
        if self.at_statement_boundary() {
            return self.err(&format!("expected the amount to {verb} to"));
        }
        self.amount()
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
        // `1 unit` and `2 units` are both how a person writes it, and the
        // singular used to fall through to cents and then choke on the word.
        if self.eat("units") || self.eat("unit") || self.eat("u") {
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
        if self.eat("everything") {
            return Some(Group::Everything.members());
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
        // One list, or several walked in step. The second form is how a pair
        // of numbers says something about each other: the 6 and the 8 are
        // partners in half the strategies anybody writes, and without it
        // "when this one wins, take the other one down" had to be written
        // out per number — four sentences becoming twelve rules, each one a
        // chance to type the wrong box number.
        let mut lists: Vec<(String, Vec<i64>)> = Vec::new();
        loop {
            let mut values = vec![self.number()?];
            while self.eat(",") {
                values.push(self.number()?);
            }
            self.expect("as")?;
            let name_line = self.line();
            let name = self.next();
            if name.is_empty() || parse_money(&name).is_some() {
                return Err(ParseError {
                    line: name_line,
                    token: name,
                    what: "expected a name to bind each value to".into(),
                });
            }
            if self.vars.contains(&name) {
                return Err(ParseError {
                    line: name_line,
                    token: name,
                    what: "already the name of a memory slot; a binding is a \
                           number, not memory, so give it its own name"
                        .into(),
                });
            }
            if lists.iter().any(|(n, _)| *n == name) {
                return Err(ParseError {
                    line: name_line,
                    token: name,
                    what: "already bound by this block".into(),
                });
            }
            if let Some((_, first)) = lists.first() {
                if first.len() != values.len() {
                    return Err(ParseError {
                        line: name_line,
                        token: name,
                        what: format!(
                            "lists walked together must be the same length; \
                             the first has {} and this has {}",
                            first.len(),
                            values.len()
                        ),
                    });
                }
            }
            lists.push((name, values));
            if !self.eat("with") {
                break;
            }
        }
        let (name, values) = lists[0].clone();
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
        // Blocks nest by re-reading their body once per value, so depth is
        // multiplicative in rules and recursive in stack frames. Two levels
        // is what a real strategy uses; this is the backstop that keeps a
        // pasted file from being either.
        if self.depth > MAX_BLOCK_DEPTH {
            return self.err("blocks nest too deeply");
        }
        let body_start = self.at;

        let mut out = Vec::new();
        let mut per_iteration = 0usize;
        for (k, _) in values.iter().enumerate() {
            self.at = body_start;
            for (n, vs) in &lists {
                self.bindings.push((n.clone(), vs[k]));
            }
            let rules = self.rules_until_end();
            for _ in &lists {
                self.bindings.pop();
            }
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
            // Everything after the first list, so the block can write its own
            // header back out exactly as it was read.
            partners: lists[1..].to_vec(),
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
                let by = self.press_target("press")?;
                return Ok(members
                    .iter()
                    .map(|b| Stmt::Press(*b, by.amount_for(*b)))
                    .collect());
            }
            let b = self.bet_ref()?;
            let by = self.press_target("press")?;
            return Ok(vec![Stmt::Press(b, by.amount_for(b))]);
        }
        if self.eat("regress") {
            if let Some(members) = self.group() {
                let by = self.press_target("regress")?;
                return Ok(members
                    .iter()
                    .map(|b| Stmt::Regress(*b, by.amount_for(*b)))
                    .collect());
            }
            let b = self.bet_ref()?;
            let by = self.press_target("regress")?;
            return Ok(vec![Stmt::Regress(b, by.amount_for(b))]);
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
        expr_depth: 0,
    };

    p.expect("strategy")?;
    let name_tok = p.next();
    let name = name_tok
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(&name_tok)
        .to_owned();
    // The text form is the save format, and these two characters break it:
    // a `#` starts a comment, so the header would truncate mid-quote and the
    // file would not read back; a `"` closes the name early and it would read
    // back as a different strategy. Refused at the door rather than escaped,
    // because a name nobody can type into the editor is not worth a escape
    // syntax nobody would remember.
    if name.contains('#') || name.contains('"') {
        return Err(ParseError {
            line: p.line(),
            token: name.clone(),
            what: "a strategy name cannot contain \" or #".to_owned(),
        });
    }
    p.expect("language")?;
    let version = p.number()?;
    // Newer is refused, older is read.
    //
    // Refusing a version this engine has never seen is the whole point of the
    // header: a strategy written against a grammar with words in it that are
    // not here would be misread rather than rejected, and misreading is the
    // one failure a save format may not have.
    //
    // Refusing an *older* one is a different thing entirely, and the first
    // version of this gate did both. Every grammar change so far has been
    // additive — a trigger, a read, an amount — and additive changes leave old
    // files meaning exactly what they meant. Turning them away would have
    // faced the next release with a choice between refusing every file on
    // every user's disk and never bumping the number at all, which is how a
    // version gate becomes decorative. So: a breaking change bumps
    // [`LANGUAGE_VERSION`] and old files are migrated or refused deliberately;
    // an additive one leaves it alone and old files keep reading.
    if version > LANGUAGE_VERSION as i64 || version < 1 {
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
    let mut var_init: Vec<i64> = Vec::new();
    while p.peek().eq_ignore_ascii_case("var") {
        p.at += 1;
        let name_line = p.line();
        let name = p.next();
        if name.is_empty() {
            return p.err("expected a name for the memory slot");
        }
        if RESERVED_WORDS.iter().any(|w| name.eq_ignore_ascii_case(w)) {
            return Err(ParseError {
                line: name_line,
                token: name,
                what: "that word already means something in the language".to_owned(),
            });
        }
        if p.vars.iter().any(|v| v.eq_ignore_ascii_case(&name)) {
            return Err(ParseError {
                line: name_line,
                token: name,
                what: "that memory slot is already declared".to_owned(),
            });
        }
        // The initial value is honoured, not documentation. It read as
        // initialization to everyone who wrote one, and a slot that started
        // at zero regardless was a strategy quietly playing a different
        // system than the one on the page.
        let init = if p.eat("=") { p.number()? } else { 0 };
        p.vars.push(name);
        var_init.push(init);
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
            let line = p.line();
            let token = p.peek().to_owned();
            let bet = p.bet_ref()?;
            // Odds have no stream of their own — `stream_of` answers with the
            // flat's — so attaching a system to them would quietly attach it
            // to the pass line instead. Naming a distinction the engine does
            // not have is worse than refusing it.
            if is_odds_ref(bet) {
                return Err(ParseError {
                    line,
                    token,
                    what: "odds ride the bet behind them and press with it".to_owned(),
                });
            }
            match stream_of(bet) {
                Some(i) => progressions[i] = prog,
                None => return p.err("that bet has no pressing system of its own"),
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
        var_init,
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
        Read::BuyIn => "buy-in".into(),
        Read::TableMin => "table-min".into(),
        Read::TableMax => "table-max".into(),
        Read::LiveCome => "live-come".into(),
        Read::LiveDontCome => "live-dont-come".into(),
        Read::OnTableFace => "on-table-face".into(),
        Read::Stake(b) => format!("stake({})", bet_text(b)),
        Read::Up(b) => format!("up({})", bet_text(b)),
        Read::Working(b) => format!("working({})", bet_text(b)),
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
            // A bare number compared against a money read is money, and is
            // written as money. Confined to a comparison's own operands: the
            // 2 in `stake(place 6) * 2` is a multiplier, not two cents, and
            // the round-trip law would catch it if this guessed wider.
            let money = is_comparison(*o) && (is_money_expr(a) || is_money_expr(b));
            let text = format!(
                "{} {} {}",
                operand_text(a, vars, left, money),
                op_text(*o),
                operand_text(b, vars, p + 1, money)
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
        // An amount is money, so a plain number in one is written the way
        // money is written. The engine's unit is the cent and always was;
        // handing `15000` back to somebody who typed `$150` was the text
        // form describing the engine rather than the game.
        AmountExpr::Cents(Expr::Const(v)) if *v >= 0 => money_text(*v),
        AmountExpr::Cents(e) => expr_text(e, vars),
    }
}

/// A press written as a destination or as a step.
///
/// A step is sugar: it becomes the destination it names, computed from what
/// is on the bet. Desugaring here rather than in the tree keeps the AST one
/// shape — the round-trip law is about the tree, and two ways to say the same
/// press would be two trees the rule editor would have to learn.
enum PressTarget {
    To(AmountExpr),
    By(AmountExpr),
}

impl PressTarget {
    fn amount_for(&self, bet: BetRef) -> AmountExpr {
        match self {
            PressTarget::To(a) => a.clone(),
            PressTarget::By(step) => {
                let step = match step {
                    // `by 1 unit` is a step of one table minimum, so it has
                    // to become cents before it can be added to a stake.
                    AmountExpr::Units(e) => {
                        Expr::bin(BinOp::Mul, e.clone(), Expr::Read(Read::TableMin))
                    }
                    AmountExpr::Cents(e) => e.clone(),
                    // `base`, `pressed` and `max` are answers the table
                    // gives, not distances. A step of one of those is read as
                    // a step of one table minimum, which is what "press it"
                    // means when nobody says by how much.
                    _ => Expr::Read(Read::TableMin),
                };
                AmountExpr::Cents(Expr::bin(BinOp::Add, Expr::Read(Read::Stake(bet)), step))
            }
        }
    }
}

/// A block's header, written the way it was read.
///
/// Built in one place because two of them need it — the renderer, and the
/// check that re-reads a block to ask whether it is still one — and a header
/// those two spelled differently would make every block dissolve on save.
fn block_header(name: &str, values: &[i64], partners: &[(String, Vec<i64>)]) -> String {
    let list = |vs: &[i64]| {
        vs.iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut out = format!("for each of {} as {name}", list(values));
    for (n, vs) in partners {
        out += &format!(" with {} as {n}", list(vs));
    }
    out
}

/// Cents as a person writes them: `$150`, `$12.50`.
///
/// Only ever used for a value that reads back as the same number — a
/// negative constant is left in plain cents, because `-$200` re-reads as a
/// negation applied to a positive literal, which is a different tree from a
/// negative literal and the round-trip law is about the tree.
fn money_text(cents: i64) -> String {
    let whole = cents / 100;
    let frac = (cents % 100).abs();
    if frac == 0 {
        format!("${whole}")
    } else {
        format!("${whole}.{frac:02}")
    }
}

/// Reads whose value is money, so a number compared against one is money too.
fn is_money_expr(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Read(
            Read::Cash
                | Read::Wealth
                | Read::Profit
                | Read::PeakProfit
                | Read::Drawdown
                | Read::Handle
                | Read::OnTableFace
                | Read::Stake(_)
                | Read::Paid(_)
        )
    )
}

/// One side of a comparison, written as money when the other side is money.
fn operand_text(e: &Expr, vars: &[String], parent: u8, money: bool) -> String {
    if money {
        match e {
            Expr::Const(v) if *v >= 0 => return money_text(*v),
            Expr::Neg(inner) => {
                if let Expr::Const(v) = inner.as_ref() {
                    if *v >= 0 {
                        return format!("-{}", money_text(*v));
                    }
                }
            }
            _ => {}
        }
    }
    expr_prec(e, vars, parent)
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
        // The strategy's memory has to be in scope: a block body that reads
        // or sets a slot cannot be parsed without it, and an earlier version
        // tried the bare parse first and gave up when it failed — so every
        // block that touched memory silently stopped being a block on the
        // first save, taking the author's comments with it.
        // One value from each list, walked in step, so a paired block is
        // re-read exactly as it was written.
        let slice: Vec<(String, Vec<i64>)> = b
            .partners
            .iter()
            .map(|(n, vs)| (n.clone(), vec![vs[k]]))
            .collect();
        let src = format!(
            "strategy \"x\" language {LANGUAGE_VERSION}\n{}\n{} {{\n{}\n}}\n",
            s.vars
                .iter()
                .map(|n| format!("var {n} = 0"))
                .collect::<Vec<_>>()
                .join("\n"),
            block_header(&b.name, &[*v], &slice),
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
        for (i, v) in s.vars.iter().enumerate() {
            let init = s.var_init.get(i).copied().unwrap_or(0);
            out += &format!("var {v} = {init}\n");
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
                "\n{} {{\n{}\n}}\n",
                block_header(&b.name, &b.values, &b.partners),
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
        // Refused where it is written, so the message can point at the line
        // the 7 is on. The compiler keeps its own check for trees that were
        // built rather than typed.
        let e =
            parse("strategy \"v\" language 1\non come point on 7:\n    bet pass\n").unwrap_err();
        assert!(e.message().contains("not a box number"), "{}", e.message());
        assert_eq!(e.line, 2);

        let mut s =
            parse("strategy \"v\" language 1\non come point on 6:\n    bet pass\n").unwrap();
        s.rules[0].trigger = Trigger::ComePointEstablished(7);
        let e = crate::strategy::compile(&s).unwrap_err();
        assert!(e.message().contains("not a box number"), "{}", e.message());
    }

    #[test]
    fn a_number_out_of_range_is_refused_rather_than_wrapped() {
        // `place 260` used to wrap through `as u8` into a perfectly legal
        // place 4, and `total(263)` into a trigger that fired on every seven.
        for (src, expect) in [
            (
                "strategy \"x\" language 1\non roll:\n bet place 260 base\n",
                "not a place number",
            ),
            (
                "strategy \"x\" language 1\non total(263):\n bet pass\n",
                "not a dice total",
            ),
            (
                "strategy \"x\" language 1\non total(13):\n bet pass\n",
                "not a dice total",
            ),
            (
                "strategy \"x\" language 1\non roll:\n bet odds on come 7\n",
                "not a box number",
            ),
            (
                "strategy \"x\" language 1\non roll when hits(400) > 0:\n bet pass\n",
                "not a dice total",
            ),
        ] {
            let e = parse(src).unwrap_err();
            assert!(e.message().contains(expect), "{}: {}", src, e.message());
        }
    }

    #[test]
    fn odds_are_refused_where_they_would_quietly_mean_the_flat() {
        // The grammar drew a distinction the engine does not have: odds keep
        // no record of their own and press with the bet behind them, so
        // these read as the flat and said nothing about it.
        for src in [
            "strategy \"x\" language 1\non win of odds on pass:\n bet pass\n",
            "strategy \"x\" language 1\non roll when paid(odds on pass) > 0:\n bet pass\n",
            "strategy \"x\" language 1\npress martingale for odds on pass\non roll:\n bet pass\n",
        ] {
            let e = parse(src).unwrap_err();
            assert!(e.message().contains("odds"), "{}: {}", src, e.message());
        }
        // But the felt reads still answer for them, and so does `bet`.
        assert!(parse(
            "strategy \"x\" language 1\non roll when up(odds on pass) == 0:\n bet odds on pass max\n"
        )
        .is_ok());
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
            // The number itself, not whatever word follows it. The offending
            // token is the one the author typed.
            (
                "strategy \"x\" language 1\non roll:\n bet place 7 base\n",
                "7",
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
        // Several seeds rather than one three-line strategy: a truncation
        // sweep can only reach the grammar its seed uses, and the old seed
        // had no memory, no guard, no block, no money and no quoted name —
        // which is most of what a person actually mistypes.
        let seeds = [
            "strategy \"s\" language 1\non come-out:\n    bet pass\n",
            "strategy \"s\" language 1\n\nvar hits = 2\n\npress martingale for pass\n\n\
             on win of place 6 when hits >= 2 and profit > $150:\n    \
             press place 6 to stake(place 6) * 2\n    set hits = 0\n",
            "strategy \"s\" language 1\n\nfor each of 6, 8 as n {\n    \
             on roll when point != 0 and point != n:  # a note\n        \
             bet place n base\n}\n",
            "strategy \"s\" language 1\non roll when min(cash, $1,000) <= -$20.50:\n    \
             down all place\n    leave \"enough\"\n",
        ];
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
        for seed in seeds {
            // Every truncation of a valid strategy, which is what a
            // half-typed one looks like.
            for k in 0..seed.len() {
                if seed.is_char_boundary(k) {
                    cases.push(seed[..k].to_owned());
                }
            }
            // And every single-byte deletion, which is what a typo looks
            // like.
            for k in 0..seed.len() {
                if seed.is_char_boundary(k) {
                    let mut c = seed.to_owned();
                    c.remove(k);
                    cases.push(c);
                }
            }
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

    /// Nesting is bounded, because the parser walks it on the native stack
    /// and strategy text arrives by paste.
    #[test]
    fn deep_nesting_is_refused_rather_than_fatal() {
        for body in [
            format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000)),
            "not ".repeat(100_000) + "1",
            "-".repeat(100_000) + "1",
            "(".repeat(100_000),
        ] {
            let src = format!("strategy \"x\" language 1\non roll when {body} == 1:\n bet pass\n");
            let e = parse(&src).expect_err("should refuse, not overflow the stack");
            assert!(e.message().contains("nests too deeply"), "{}", e.message());
        }
        // Blocks nest by re-reading their body once per value, so they are
        // multiplicative in rules as well as recursive in frames.
        let deep = "for each of 1, 2 as x { ".repeat(200) + &"}".repeat(200);
        let e = parse(&format!("strategy \"x\" language 1\n{deep}\n"))
            .expect_err("should refuse, not overflow the stack");
        assert!(e.message().contains("nest too deeply"), "{}", e.message());
    }

    /// Memory starts where the strategy says it starts.
    #[test]
    fn a_declared_initial_value_is_kept() {
        let s = parse("strategy \"x\" language 1\nvar mult = 3\non roll:\n bet pass $5 * mult\n")
            .unwrap();
        assert_eq!(s.var_init, vec![3]);
        assert_eq!(crate::strategy::compile(&s).unwrap().var_init, vec![3]);
        round_trip(&s);
        // And a slot with no initializer starts at nothing, as it always did.
        let s = parse("strategy \"x\" language 1\nvar n\non roll:\n bet pass\n").unwrap();
        assert_eq!(s.var_init, vec![0]);
    }

    /// A memory slot may not take a name the grammar already reads.
    #[test]
    fn a_memory_slot_cannot_take_a_word_the_language_owns() {
        for name in ["point", "cash", "profit", "roll", "min", "max", "base"] {
            let src = format!("strategy \"x\" language 1\nvar {name} = 0\non roll:\n bet pass\n");
            let e = parse(&src).expect_err("{name} should be refused");
            assert!(e.message().contains("already means"), "{name}");
        }
        // But the parameterized reads are only reads when a `(` follows, so
        // these are perfectly good names and people use them.
        for name in ["hits", "streak", "paid", "stake", "wins"] {
            let src = format!(
                "strategy \"x\" language 1\nvar {name} = 0\n\
                 on roll when {name} < 2:\n set {name} = {name} + 1\n bet pass\n"
            );
            assert!(parse(&src).is_ok(), "{name} should be a usable name");
        }
        let twice = "strategy \"x\" language 1\nvar a = 0\nvar a = 1\non roll:\n bet pass\n";
        assert!(parse(twice)
            .expect_err("a duplicate slot is a mistake")
            .message()
            .contains("already declared"));
    }

    /// The text form is the save format, so a name has to survive it.
    #[test]
    fn a_name_that_would_break_the_file_is_refused() {
        for name in ["a # b", "a \" b"] {
            let src = format!("strategy \"{name}\" language 1\non roll:\n bet pass\n");
            assert!(parse(&src).is_err(), "{name:?} should be refused");
        }
        // Everything else a person would call a strategy still works.
        for name in ["44 Inside, regressed", "don't-pass darling", "Ünïcødé 🎲"] {
            let src = format!("strategy \"{name}\" language 1\non roll:\n bet pass\n");
            let s = parse(&src).unwrap_or_else(|e| panic!("{name:?}: {}", e.message()));
            assert_eq!(s.name, name);
            round_trip(&s);
        }
    }

    /// `press place 6 to` is a sentence that stopped mid-word.
    #[test]
    fn a_press_with_no_amount_is_refused() {
        let e = parse(
            "strategy \"x\" language 1\non win of place 6:\n press place 6 to\non roll:\n bet pass\n",
        )
        .expect_err("a dangling press should not quietly mean `to pressed`");
        assert!(e.message().contains("amount to press"), "{}", e.message());
        // `bet` keeps its optional amount, which is how a player says it.
        assert!(parse("strategy \"x\" language 1\non roll:\n bet pass\n").is_ok());
    }

    /// A block that touches memory is still a block.
    #[test]
    fn a_block_that_uses_memory_survives_a_round_trip() {
        // It did not: `block_holds` re-read the body without the strategy's
        // memory in scope, that parse failed, and the block silently unrolled
        // into copies — taking the author's comments with it.
        let src = "strategy \"x\" language 1\n\nvar hits = 0\n\n\
                   for each of 6, 8 as n {\n    \
                   on win of place n:  # counts both numbers\n        \
                   set hits = hits + 1\n}\n\non roll when point != 0:\n    bet place 6 base\n";
        let s = parse(src).unwrap_or_else(|e| panic!("{}", e.message()));
        assert_eq!(s.blocks.len(), 1, "the block was recorded");
        let out = render(&s);
        assert!(
            out.contains("for each of 6, 8 as n"),
            "still a block:\n{out}"
        );
        assert!(
            out.contains("# counts both numbers"),
            "comment kept:\n{out}"
        );
        round_trip(&s);
    }

    /// Money is written the way money is written.
    #[test]
    fn money_renders_as_money() {
        let src = "strategy \"x\" language 1\n\n\
                   on roll when profit >= $150 or profit <= -$200:\n    leave\n\
                   on roll when cash > $12.50:\n    bet place 6 $18\n";
        let s = parse(src).unwrap_or_else(|e| panic!("{}", e.message()));
        let out = render(&s);
        assert!(out.contains("profit >= $150"), "{out}");
        assert!(out.contains("profit <= -$200"), "{out}");
        assert!(out.contains("cash > $12.50"), "{out}");
        assert!(out.contains("bet place 6 $18"), "{out}");
        round_trip(&s);
        // A count compared against a count stays a count: the 2 in
        // `hits(8) >= 2` is two hits, not two cents.
        let s =
            parse("strategy \"x\" language 1\non roll when hits(8) >= 2:\n bet pass\n").unwrap();
        let out = render(&s);
        assert!(out.contains("hits(8) >= 2"), "{out}");
        round_trip(&s);
    }

    /// A comma inside money is a thousands separator only when it separates
    /// thousands — otherwise it is the argument list's own punctuation.
    #[test]
    fn a_comma_after_money_still_separates_arguments() {
        let s =
            parse("strategy \"x\" language 1\non roll when min($5, cash) > 0:\n bet pass $1,000\n")
                .unwrap_or_else(|e| panic!("{}", e.message()));
        round_trip(&s);
        assert_eq!(parse_money("$1,000"), Some(100_000));
        assert_eq!(parse_money("$5"), Some(500));
    }

    /// The reads a strategy needs to be written about a table rather than
    /// about the one table it was typed at.
    #[test]
    fn the_table_answers_for_itself() {
        let s = parse(
            "strategy \"x\" language 1\n\
             on roll when profit <= 0 - buy-in / 2:\n    leave\n\
             on come-out when table-min <= $10 and table-max >= $1,000:\n    bet pass base\n",
        )
        .unwrap_or_else(|e| panic!("{}", e.message()));
        round_trip(&s);
        // None of them needs an accumulator: the table already knows.
        let p = crate::strategy::compile(&s).unwrap();
        assert!(p.features.is_empty(), "{:?}", p.features);
    }

    /// `press it` is a step, and the language can say so.
    #[test]
    fn a_press_can_be_a_step_rather_than_a_destination() {
        let by = parse(
            "strategy \"x\" language 1\non win of place 6:\n press place 6 by 1 unit\n\
             on roll:\n bet place 6\n",
        )
        .unwrap_or_else(|e| panic!("{}", e.message()));
        // Sugar: it becomes the destination it names, so there is one tree
        // and the rule editor has one shape to learn.
        let to = parse(
            "strategy \"x\" language 1\non win of place 6:\n \
             press place 6 to stake(place 6) + 1 * table-min\non roll:\n bet place 6\n",
        )
        .unwrap();
        assert_eq!(by.rules, to.rules);
        round_trip(&by);
        // And the singular reads, which it did not.
        assert!(parse("strategy \"x\" language 1\non roll:\n bet pass 1 unit\n").is_ok());
    }

    /// A block can say something about a pair.
    #[test]
    fn lists_can_be_walked_in_step() {
        let s = parse(
            "strategy \"x\" language 1\n\n\
             for each of 6, 8 as n with 8, 6 as other {\n    \
             on win of place n:\n        \
             down place other\n}\n",
        )
        .unwrap_or_else(|e| panic!("{}", e.message()));
        assert_eq!(s.rules.len(), 2);
        // The 6 winning takes down the 8, and the 8 the 6 — which is the
        // whole point, and was four rules of hand-written box numbers.
        assert_eq!(s.rules[0].body, vec![Stmt::Down(BetRef::Place(8))]);
        assert_eq!(s.rules[1].body, vec![Stmt::Down(BetRef::Place(6))]);
        assert!(render(&s).contains("for each of 6, 8 as n with 8, 6 as other"));
        round_trip(&s);

        // Lists walked together have to be the same length, and a name may
        // not be bound twice.
        for bad in [
            "strategy \"x\" language 1\nfor each of 6, 8 as n with 4 as m {\n on roll:\n bet pass\n}\n",
            "strategy \"x\" language 1\nfor each of 6, 8 as n with 4, 5 as n {\n on roll:\n bet pass\n}\n",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    /// The two pieces of vocabulary §3 promised and never had.
    #[test]
    fn everything_and_working_are_sayable() {
        let s = parse(
            "strategy \"x\" language 1\n\
             on roll when working(place 6) == 0:\n    working place 6 on\n\
             on point-established:\n    bet place inside base\n\
             on roll when profit >= $100:\n    down everything\n",
        )
        .unwrap_or_else(|e| panic!("{}", e.message()));
        round_trip(&s);
        // `everything` is what a dealer would sweep: the numbers and the
        // hardways, not the contract bets or the one-roll propositions.
        assert_eq!(s.rules[2].body.len(), 10);
        assert!(s.rules[2].body.contains(&Stmt::Down(BetRef::Hardway(10))));
        assert!(!s.rules[2].body.contains(&Stmt::Down(BetRef::Pass)));
    }

    /// Money past what a cent can hold is refused, not wrapped.
    #[test]
    fn money_that_does_not_fit_is_refused() {
        assert_eq!(parse_money("$92233720368547759"), None);
        // And a third decimal place is a figure this table cannot take,
        // rather than one silently rounded down to one it can.
        assert_eq!(parse_money("$4.999"), None);
        assert_eq!(parse_money("$4.99"), Some(499));
        assert_eq!(parse_money("$4.9"), Some(490));
    }

    /// Older grammars still read; newer ones are refused.
    #[test]
    fn the_version_gate_admits_the_past_and_refuses_the_future() {
        assert!(parse("strategy \"x\" language 1\non roll:\n bet pass\n").is_ok());
        for bad in ["2", "99", "0", "-1"] {
            let src = format!("strategy \"x\" language {bad}\non roll:\n bet pass\n");
            assert!(parse(&src).is_err(), "version {bad} should be refused");
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
        // Every bet reference the grammar spells, the numbered odds included
        // — an earlier version covered nine of thirteen, and the law went
        // untested on exactly the shapes whose rendering is fiddliest.
        const BETS: [BetRef; 13] = [
            BetRef::Pass,
            BetRef::PassOdds,
            BetRef::DontPass,
            BetRef::DontPassLay,
            BetRef::Come,
            BetRef::DontCome,
            BetRef::ComeOdds(6),
            BetRef::DontComeLay(10),
            BetRef::Place(6),
            BetRef::Place(10),
            BetRef::Hardway(8),
            BetRef::Field,
            BetRef::AnySeven,
        ];
        /// Bets that keep a record of their own, so may stand in a win/loss
        /// trigger or a history read.
        const RECORDED: [BetRef; 8] = [
            BetRef::Pass,
            BetRef::DontPass,
            BetRef::Come,
            BetRef::DontCome,
            BetRef::Place(5),
            BetRef::Place(9),
            BetRef::Hardway(4),
            BetRef::AnyCraps,
        ];
        const TRIGGERS: [Trigger; 11] = [
            Trigger::SessionStart,
            Trigger::ComeOut,
            Trigger::Roll,
            Trigger::PointEstablished,
            Trigger::PointMade,
            Trigger::SevenOut,
            Trigger::Total(7),
            Trigger::Total(12),
            Trigger::ComePointEstablished(6),
            Trigger::DontComePointEstablished(4),
            Trigger::Win(BetRef::Place(6)),
        ];
        const OPS: [BinOp; 14] = [
            BinOp::Add,
            BinOp::Sub,
            BinOp::Mul,
            BinOp::Div,
            BinOp::Min,
            BinOp::Max,
            BinOp::Lt,
            BinOp::Le,
            BinOp::Gt,
            BinOp::Ge,
            BinOp::Eq,
            BinOp::Ne,
            BinOp::And,
            BinOp::Or,
        ];
        // All twenty-five, so the money-rendering rule is exercised against
        // both the reads it applies to and the reads it must leave alone.
        const READS: [Read; 25] = [
            Read::Point,
            Read::ComeOut,
            Read::LastTotal,
            Read::Roll,
            Read::RollsThisShooter,
            Read::Shooter,
            Read::Cash,
            Read::Wealth,
            Read::Profit,
            Read::PeakProfit,
            Read::Drawdown,
            Read::Handle,
            Read::LiveCome,
            Read::LiveDontCome,
            Read::OnTableFace,
            Read::Hits(8),
            Read::HitsThisShooter(4),
            Read::ComePoint(6),
            Read::DontComePoint(10),
            Read::Streak(BetRef::Pass),
            Read::Stake(BetRef::Place(6)),
            Read::Up(BetRef::PassOdds),
            Read::Wins(BetRef::Come),
            Read::Losses(BetRef::Field),
            Read::Paid(BetRef::Place(8)),
        ];
        // Names that stress the tokenizer rather than flatter it.
        const NAMES: [&str; 6] = [
            "case",
            "44 Inside, regressed",
            "don't-pass darling",
            "  padded  ",
            "press martingale for pass",
            "Ünïcødé 🎲",
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
                let trigger = match g.next() % 8 {
                    0 => Trigger::Win(g.pick(&RECORDED)),
                    1 => Trigger::Loss(g.pick(&RECORDED)),
                    _ => g.pick(&TRIGGERS),
                };
                let mut r = Rule::new(trigger, body);
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
                name: format!("{} {case}", g.pick(&NAMES)),
                vars,
                // Non-zero initial values, because a slot that starts
                // somewhere has to survive the round trip like everything
                // else does.
                var_init: vec![(g.next() % 400) as i64 - 200, 0],
                rules,
                progressions,
                blocks: Vec::new(),
            };
            round_trip(&s);
        }
    }
}
