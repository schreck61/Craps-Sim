// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The rule editor: a strategy as rows of typed slots.
//!
//! `STRATEGY_DSL.md` §5.1. Every slot is a dropdown or a validated number,
//! so a syntax error is not something this surface can produce — which is
//! the whole reason it exists beside the text form rather than instead of
//! it.
//!
//! One AST, two editors (Principle 2): these rows edit the tree, the tree
//! is rendered back to text, and the text form parses to the same tree.
//! Nothing here writes text and nothing here parses it — the round-trip law
//! is what keeps the two surfaces from drifting apart, and it can only do
//! that if the tree is the one thing both of them touch.
//!
//! **Where the claim needed narrowing.** Principle 2 says anything the text
//! expresses, the rows can render and edit. Rendering, yes — every rule
//! shows. Editing, not quite: a condition outside the shapes below is shown
//! as its own text and edited in the text form. Slots for arbitrary nested
//! arithmetic would be a worse editor than a line of prose, and pretending
//! otherwise would be building a tree widget nobody asked for.

use craps_engine::strategy::{
    render, AmountExpr, BinOp, Expr, Read, Rule, Stmt, Strategy, Trigger,
};
use craps_engine::{BetRef, HARD_NUMS, PLACE_NUMS};
use egui::{FontId, RichText, Stroke};

use crate::app::App;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

// --- the vocabulary the slots offer -------------------------------------

const TRIGGERS: [(&str, Trigger); 6] = [
    ("the session starts", Trigger::SessionStart),
    ("it is a come-out", Trigger::ComeOut),
    ("a point is established", Trigger::PointEstablished),
    ("the point is made", Trigger::PointMade),
    ("the shooter sevens out", Trigger::SevenOut),
    ("every roll", Trigger::Roll),
];

/// Bets a rule can name, in the order they sit on a layout.
fn bets() -> Vec<BetRef> {
    let mut v = vec![
        BetRef::Pass,
        BetRef::PassOdds,
        BetRef::DontPass,
        BetRef::DontPassLay,
        BetRef::Come,
        BetRef::DontCome,
    ];
    v.extend(PLACE_NUMS.iter().map(|&n| BetRef::Place(n)));
    v.extend(PLACE_NUMS.iter().map(|&n| BetRef::ComeOdds(n)));
    v.extend(PLACE_NUMS.iter().map(|&n| BetRef::DontComeLay(n)));
    v.extend(HARD_NUMS.iter().map(|&n| BetRef::Hardway(n)));
    v.extend([BetRef::Field, BetRef::AnySeven, BetRef::AnyCraps]);
    v
}

/// Bets with a win/loss record of their own — odds resolve with the flat
/// behind them, so they cannot trigger a rule.
fn streamed_bets() -> Vec<BetRef> {
    bets()
        .into_iter()
        .filter(|b| {
            !matches!(
                b,
                BetRef::PassOdds
                    | BetRef::DontPassLay
                    | BetRef::ComeOdds(_)
                    | BetRef::DontComeLay(_)
            )
        })
        .collect()
}

/// What a condition can look at. Reads that take a bet or a number are
/// offered per bet and per box number, so the slot stays a flat list.
fn reads() -> Vec<(String, Read)> {
    let mut v: Vec<(String, Read)> = vec![
        ("the point".into(), Read::Point),
        ("the last total".into(), Read::LastTotal),
        ("rolls this shooter".into(), Read::RollsThisShooter),
        ("the roll number".into(), Read::Roll),
        ("shooters so far".into(), Read::Shooter),
        ("cash in hand".into(), Read::Cash),
        ("wealth".into(), Read::Wealth),
        ("profit".into(), Read::Profit),
        ("best profit so far".into(), Read::PeakProfit),
        ("drawdown from the peak".into(), Read::Drawdown),
        ("come bets working".into(), Read::LiveCome),
        ("don't come bets working".into(), Read::LiveDontCome),
    ];
    for &n in PLACE_NUMS.iter().chain([2u8, 3, 7, 11, 12].iter()) {
        v.push((format!("times {n} has come"), Read::Hits(n)));
        v.push((
            format!("times {n} has come this shooter"),
            Read::HitsThisShooter(n),
        ));
    }
    for b in bets() {
        let name = craps_engine::strategy::bet_name(b);
        v.push((format!("what is on {name}"), Read::Stake(b)));
    }
    for b in streamed_bets() {
        let name = craps_engine::strategy::bet_name(b);
        v.push((format!("{name}'s run of wins or losses"), Read::Streak(b)));
        v.push((format!("times {name} has won"), Read::Wins(b)));
        v.push((format!("times {name} has lost"), Read::Losses(b)));
    }
    v
}

const OPS: [(&str, BinOp); 6] = [
    ("is at least", BinOp::Ge),
    ("is more than", BinOp::Gt),
    ("is at most", BinOp::Le),
    ("is less than", BinOp::Lt),
    ("is exactly", BinOp::Eq),
    ("is not", BinOp::Ne),
];

// --- conditions as clauses ----------------------------------------------

/// One test in a condition. A condition the rows can edit is a run of these
/// joined by "and" — which is what nearly every real rule is.
#[derive(Clone, PartialEq, Debug)]
enum Clause {
    Cmp {
        read: Read,
        op: BinOp,
        value: i64,
    },
    /// A bare read used as a truth: "there is a come point on the 6".
    Truthy(Read),
}

fn clause_of(e: &Expr) -> Option<Clause> {
    match e {
        Expr::Read(r) => Some(Clause::Truthy(*r)),
        Expr::Bin(op, a, b) if is_cmp(*op) => match (a.as_ref(), b.as_ref()) {
            (Expr::Read(r), Expr::Const(v)) => Some(Clause::Cmp {
                read: *r,
                op: *op,
                value: *v,
            }),
            _ => None,
        },
        _ => None,
    }
}

fn is_cmp(o: BinOp) -> bool {
    matches!(
        o,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne
    )
}

/// Break a condition into clauses, or decline — a condition this editor
/// cannot take apart is one it will not pretend to own.
fn as_clauses(e: &Expr) -> Option<Vec<Clause>> {
    match e {
        Expr::Bin(BinOp::And, a, b) => {
            let mut left = as_clauses(a)?;
            left.extend(as_clauses(b)?);
            Some(left)
        }
        other => clause_of(other).map(|c| vec![c]),
    }
}

fn from_clauses(cs: &[Clause]) -> Option<Expr> {
    let mut it = cs.iter().map(|c| match c {
        Clause::Truthy(r) => Expr::Read(*r),
        Clause::Cmp { read, op, value } => Expr::bin(*op, Expr::Read(*read), Expr::Const(*value)),
    });
    let first = it.next()?;
    Some(it.fold(first, |acc, e| Expr::bin(BinOp::And, acc, e)))
}

// --- the editor ----------------------------------------------------------

/// Draw the rule rows. Returns true when the tree changed.
pub fn show(app: &mut App, ui: &mut egui::Ui) -> bool {
    let t = app.theme.clone();
    let Some(mut strategy) = app.bench.parsed.clone() else {
        ui.label(
            RichText::new(
                "Nothing to show yet — take the current player, open a saved \
                 strategy, or pick an example.",
            )
            .font(FontId::new(type_scale::BODY, theme::sans()))
            .color(t.ink2),
        );
        return false;
    };

    let mut changed = false;
    let mut delete: Option<usize> = None;
    let mut move_up: Option<usize> = None;

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(&strategy.name)
                .font(FontId::new(type_scale::SECTION, theme::sans_semibold()))
                .color(t.ink),
        );
        ui.label(
            RichText::new(format!(
                "{} rule{}",
                strategy.rules.len(),
                if strategy.rules.len() == 1 { "" } else { "s" }
            ))
            .font(FontId::new(type_scale::CAPTION, theme::sans()))
            .color(t.ink2),
        );
    });
    ui.add_space(6.0);

    for i in 0..strategy.rules.len() {
        let fired = app
            .replay
            .bench
            .as_ref()
            .and_then(|b| b.fire_counts.get(i).copied());
        egui::Frame::NONE
            .fill(t.surface)
            .stroke(Stroke::new(1.0, t.hairline))
            .corner_radius(6)
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{i:>2}"))
                            .font(FontId::new(type_scale::CAPTION, theme::mono()))
                            .color(t.ink2),
                    );
                    // The count from the last night stepped on Replay. A
                    // zero is the dead-rule diagnostic, arrived at by
                    // running rather than by proving.
                    if let Some(n) = fired {
                        ui.label(
                            RichText::new(format!("{n}× last night"))
                                .font(FontId::new(type_scale::CAPTION, theme::mono()))
                                .color(t.ink2),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("×")
                            .on_hover_text("delete this rule")
                            .clicked()
                        {
                            delete = Some(i);
                        }
                        if i > 0 && ui.small_button("↑").on_hover_text("move up").clicked() {
                            move_up = Some(i);
                        }
                    });
                });
                changed |= rule_row(ui, &t, &mut strategy.rules[i], &strategy.vars);
            });
        ui.add_space(4.0);
    }

    ui.horizontal(|ui| {
        if ui.button("Add a rule").clicked() {
            strategy.rules.push(Rule::new(
                Trigger::Roll,
                vec![Stmt::Bet(BetRef::Pass, AmountExpr::Pressed)],
            ));
            changed = true;
        }
    });

    if let Some(i) = delete {
        strategy.rules.remove(i);
        changed = true;
    }
    if let Some(i) = move_up {
        strategy.rules.swap(i - 1, i);
        changed = true;
    }

    if changed {
        // The tree is the truth; the text is written from it and reparsed,
        // so the two editors cannot drift.
        app.bench.source = render(&strategy);
        app.bench.build();
    }
    changed
}

/// One rule: `on <trigger> when <clauses>: <actions>`.
fn rule_row(
    ui: &mut egui::Ui,
    t: &crate::ui::theme::Theme,
    rule: &mut Rule,
    vars: &[String],
) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(word(t, "When"));
        changed |= trigger_slot(ui, &mut rule.trigger);
    });
    changed |= guard_slots(ui, t, rule);
    ui.add_space(2.0);
    let mut drop: Option<usize> = None;
    let count = rule.body.len();
    for (k, stmt) in rule.body.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.label(word(t, if k == 0 { "then" } else { "and" }));
            changed |= stmt_slots(ui, t, stmt, vars);
            if count > 1 && ui.small_button("×").clicked() {
                drop = Some(k);
            }
        });
    }
    if let Some(k) = drop {
        rule.body.remove(k);
        changed = true;
    }
    ui.horizontal(|ui| {
        if ui.small_button("+ action").clicked() {
            rule.body.push(Stmt::Bet(BetRef::Pass, AmountExpr::Pressed));
            changed = true;
        }
    });
    changed
}

fn word(t: &crate::ui::theme::Theme, s: &str) -> RichText {
    RichText::new(s)
        .font(FontId::new(type_scale::BODY, theme::sans()))
        .color(t.ink2)
}

fn trigger_slot(ui: &mut egui::Ui, trigger: &mut Trigger) -> bool {
    let mut changed = false;
    let current = trigger_label(*trigger);
    egui::ComboBox::from_id_salt(("trigger", ui.next_auto_id()))
        .selected_text(current)
        .width(230.0)
        .show_ui(ui, |ui| {
            for (label, t) in TRIGGERS {
                if ui.selectable_label(*trigger == t, label).clicked() {
                    *trigger = t;
                    changed = true;
                }
            }
            for n in [2u8, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] {
                let t = Trigger::Total(n);
                if ui
                    .selectable_label(*trigger == t, format!("the roll totals {n}"))
                    .clicked()
                {
                    *trigger = t;
                    changed = true;
                }
            }
            for b in streamed_bets() {
                let name = craps_engine::strategy::bet_name(b);
                for (verb, t) in [("wins", Trigger::Win(b)), ("loses", Trigger::Loss(b))] {
                    if ui
                        .selectable_label(*trigger == t, format!("{name} {verb}"))
                        .clicked()
                    {
                        *trigger = t;
                        changed = true;
                    }
                }
            }
        });
    changed
}

fn trigger_label(t: Trigger) -> String {
    if let Some((label, _)) = TRIGGERS.iter().find(|(_, x)| *x == t) {
        return (*label).to_owned();
    }
    match t {
        Trigger::Total(n) => format!("the roll totals {n}"),
        Trigger::Win(b) => format!("{} wins", craps_engine::strategy::bet_name(b)),
        Trigger::Loss(b) => format!("{} loses", craps_engine::strategy::bet_name(b)),
        _ => "every roll".into(),
    }
}

/// The condition, as clauses joined by "and".
fn guard_slots(ui: &mut egui::Ui, t: &crate::ui::theme::Theme, rule: &mut Rule) -> bool {
    let mut changed = false;
    // No condition at all is an empty list — something to add to. Only a
    // condition these slots cannot take apart is one they decline to own.
    let clauses = match &rule.guard {
        None => Some(Vec::new()),
        Some(g) => as_clauses(g),
    };

    let Some(mut clauses) = clauses else {
        // A condition outside the shapes these slots know. It is shown, not
        // hidden — but it is edited in the text form, because slots for
        // arbitrary arithmetic would read worse than the line of prose they
        // replaced.
        if let Some(g) = &rule.guard {
            ui.horizontal_wrapped(|ui| {
                ui.label(word(t, "and"));
                ui.label(
                    RichText::new(guard_text(g, &[]))
                        .font(FontId::new(type_scale::BODY, theme::mono()))
                        .color(t.ink),
                );
                ui.label(
                    RichText::new("— edit this one as text")
                        .font(FontId::new(type_scale::CAPTION, theme::sans()))
                        .color(t.ink2),
                );
            });
        }
        return false;
    };

    let mut drop: Option<usize> = None;
    for (k, clause) in clauses.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.label(word(t, if k == 0 { "and" } else { "and also" }));
            changed |= clause_slots(ui, clause);
            if ui.small_button("×").clicked() {
                drop = Some(k);
            }
        });
    }
    if let Some(k) = drop {
        clauses.remove(k);
        changed = true;
    }
    ui.horizontal(|ui| {
        if ui.small_button("+ condition").clicked() {
            clauses.push(Clause::Cmp {
                read: Read::Point,
                op: BinOp::Ne,
                value: 0,
            });
            changed = true;
        }
    });
    if changed {
        rule.guard = from_clauses(&clauses);
    }
    changed
}

/// A condition this editor cannot take apart, rendered for reading.
fn guard_text(e: &Expr, vars: &[String]) -> String {
    let s = Strategy {
        name: String::new(),
        vars: vars.to_vec(),
        rules: vec![Rule {
            trigger: Trigger::Roll,
            guard: Some(e.clone()),
            body: vec![Stmt::Leave],
        }],
        progressions: [craps_engine::Progression::Flat; 17],
    };
    // `render_rule` writes `on roll when <guard>: leave`; the condition is
    // the part between.
    let line = craps_engine::strategy::render_rule(&s, 0);
    line.split_once(" when ")
        .and_then(|(_, rest)| rest.split_once(": "))
        .map(|(g, _)| g.to_owned())
        .unwrap_or(line)
}

fn clause_slots(ui: &mut egui::Ui, clause: &mut Clause) -> bool {
    let mut changed = false;
    let all = reads();
    let (mut read, mut op, mut value, mut truthy) = match clause {
        Clause::Cmp { read, op, value } => (*read, *op, *value, false),
        Clause::Truthy(r) => (*r, BinOp::Ne, 0, true),
    };
    let label = all
        .iter()
        .find(|(_, r)| *r == read)
        .map(|(l, _)| l.clone())
        .unwrap_or_else(|| format!("{read:?}"));

    egui::ComboBox::from_id_salt(("read", ui.next_auto_id()))
        .selected_text(label)
        .width(260.0)
        .show_ui(ui, |ui| {
            for (l, r) in &all {
                if ui.selectable_label(read == *r, l).clicked() {
                    read = *r;
                    changed = true;
                }
            }
        });
    if !truthy {
        let op_label = OPS
            .iter()
            .find(|(_, o)| *o == op)
            .map(|(l, _)| *l)
            .unwrap_or("is");
        egui::ComboBox::from_id_salt(("op", ui.next_auto_id()))
            .selected_text(op_label)
            .width(110.0)
            .show_ui(ui, |ui| {
                for (l, o) in OPS {
                    if ui.selectable_label(op == o, l).clicked() {
                        op = o;
                        changed = true;
                    }
                }
            });
        // Money reads are typed in dollars; counts are typed as counts.
        if is_money(read) {
            let mut dollars = value as f64 / 100.0;
            if ui
                .add(egui::DragValue::new(&mut dollars).speed(1.0).prefix("$"))
                .changed()
            {
                value = (dollars * 100.0).round() as i64;
                changed = true;
            }
        } else if ui
            .add(egui::DragValue::new(&mut value).speed(1.0))
            .changed()
        {
            changed = true;
        }
    } else {
        ui.label(
            RichText::new("is there")
                .font(FontId::new(type_scale::BODY, theme::sans()))
                .color(egui::Color32::PLACEHOLDER),
        );
    }
    if changed {
        *clause = if truthy {
            Clause::Truthy(read)
        } else {
            Clause::Cmp { read, op, value }
        };
    }
    let _ = &mut truthy;
    changed
}

/// Reads whose values are cents, so the slot asks for dollars.
fn is_money(r: Read) -> bool {
    matches!(
        r,
        Read::Cash
            | Read::Wealth
            | Read::Profit
            | Read::PeakProfit
            | Read::Drawdown
            | Read::Handle
            | Read::OnTableFace
            | Read::Stake(_)
    )
}

/// One action: a verb, a bet, and how much.
fn stmt_slots(
    ui: &mut egui::Ui,
    t: &crate::ui::theme::Theme,
    stmt: &mut Stmt,
    vars: &[String],
) -> bool {
    let mut changed = false;
    const VERBS: [&str; 6] = ["bet", "press", "regress", "take down", "turn", "leave"];
    let current = match stmt {
        Stmt::Bet(..) => 0,
        Stmt::Press(..) => 1,
        Stmt::Regress(..) => 2,
        Stmt::Down(_) => 3,
        Stmt::Working(..) => 4,
        Stmt::Leave => 5,
        Stmt::Set(..) => 6,
    };
    if current == 6 {
        // Memory is named by the strategy, so a slot cannot offer it until
        // there is a way to declare one here; shown, edited as text.
        ui.label(
            RichText::new(stmt_text(stmt, vars))
                .font(FontId::new(type_scale::BODY, theme::mono()))
                .color(t.ink),
        );
        return false;
    }

    let mut verb = current;
    egui::ComboBox::from_id_salt(("verb", ui.next_auto_id()))
        .selected_text(VERBS[verb])
        .width(100.0)
        .show_ui(ui, |ui| {
            for (i, v) in VERBS.iter().enumerate() {
                if ui.selectable_label(verb == i, *v).clicked() {
                    verb = i;
                    changed = true;
                }
            }
        });

    let bet_now = match stmt {
        Stmt::Bet(b, _) | Stmt::Press(b, _) | Stmt::Regress(b, _) => Some(*b),
        Stmt::Down(b) | Stmt::Working(b, _) => Some(*b),
        _ => None,
    };
    let amount_now = match stmt {
        Stmt::Bet(_, a) | Stmt::Press(_, a) | Stmt::Regress(_, a) => a.clone(),
        _ => AmountExpr::Pressed,
    };
    let on_now = matches!(stmt, Stmt::Working(_, true));

    let mut bet = bet_now.unwrap_or(BetRef::Pass);
    if verb != 5 {
        egui::ComboBox::from_id_salt(("bet", ui.next_auto_id()))
            .selected_text(craps_engine::strategy::bet_name(bet))
            .width(180.0)
            .show_ui(ui, |ui| {
                for b in bets() {
                    if ui
                        .selectable_label(bet == b, craps_engine::strategy::bet_name(b))
                        .clicked()
                    {
                        bet = b;
                        changed = true;
                    }
                }
            });
    }

    let mut amount = amount_now;
    let mut on = on_now;
    match verb {
        0..=2 => {
            if verb > 0 {
                ui.label(word(t, "to"));
            }
            changed |= amount_slot(ui, &mut amount);
        }
        4 => {
            let mut sel = if on { 0 } else { 1 };
            egui::ComboBox::from_id_salt(("onoff", ui.next_auto_id()))
                .selected_text(if on { "on" } else { "off" })
                .width(70.0)
                .show_ui(ui, |ui| {
                    for (i, label) in ["on", "off"].iter().enumerate() {
                        if ui.selectable_label(sel == i, *label).clicked() {
                            sel = i;
                            changed = true;
                        }
                    }
                });
            on = sel == 0;
        }
        _ => {}
    }

    if changed {
        *stmt = match verb {
            0 => Stmt::Bet(bet, amount),
            1 => Stmt::Press(bet, amount),
            2 => Stmt::Regress(bet, amount),
            3 => Stmt::Down(bet),
            4 => Stmt::Working(bet, on),
            _ => Stmt::Leave,
        };
    }
    changed
}

/// How much, as the four shapes an amount can take.
fn amount_slot(ui: &mut egui::Ui, amount: &mut AmountExpr) -> bool {
    let mut changed = false;
    let label = match amount {
        AmountExpr::Pressed => "whatever it presses to".to_owned(),
        AmountExpr::Base => "the table's own stake".to_owned(),
        AmountExpr::MaxOdds => "the most odds allowed".to_owned(),
        AmountExpr::Units(Expr::Const(n)) => format!("{n} units"),
        AmountExpr::Cents(Expr::Const(c)) => numerals::money_text(*c, false),
        // A computed amount — `stake(place 6) * 2` — is shown and edited as
        // text, for the same reason a computed condition is.
        _ => "a computed amount".to_owned(),
    };
    let computed = matches!(
        amount,
        AmountExpr::Units(e) | AmountExpr::Cents(e) if !matches!(e, Expr::Const(_))
    );
    egui::ComboBox::from_id_salt(("amount", ui.next_auto_id()))
        .selected_text(label)
        .width(200.0)
        .show_ui(ui, |ui| {
            for (l, a) in [
                ("whatever it presses to", AmountExpr::Pressed),
                ("the table's own stake", AmountExpr::Base),
                ("the most odds allowed", AmountExpr::MaxOdds),
                ("a number of units", AmountExpr::Units(Expr::Const(1))),
                ("an amount of money", AmountExpr::Cents(Expr::Const(1000))),
            ] {
                if ui.selectable_label(false, l).clicked() {
                    *amount = a;
                    changed = true;
                }
            }
        });
    if computed {
        return changed;
    }
    match amount {
        AmountExpr::Units(Expr::Const(n)) => {
            let mut v = *n;
            if ui
                .add(egui::DragValue::new(&mut v).speed(1.0).range(1..=100))
                .changed()
            {
                *amount = AmountExpr::Units(Expr::Const(v));
                changed = true;
            }
        }
        AmountExpr::Cents(Expr::Const(c)) => {
            let mut dollars = *c as f64 / 100.0;
            if ui
                .add(egui::DragValue::new(&mut dollars).speed(1.0).prefix("$"))
                .changed()
            {
                *amount = AmountExpr::Cents(Expr::Const((dollars * 100.0).round() as i64));
                changed = true;
            }
        }
        _ => {}
    }
    changed
}

/// A statement the slots do not own, rendered for reading.
fn stmt_text(stmt: &Stmt, vars: &[String]) -> String {
    let s = Strategy {
        name: String::new(),
        vars: vars.to_vec(),
        rules: vec![Rule::new(Trigger::Roll, vec![stmt.clone()])],
        progressions: [craps_engine::Progression::Flat; 17],
    };
    craps_engine::strategy::render_rule(&s, 0)
        .split_once(": ")
        .map(|(_, rest)| rest.to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use craps_engine::strategy::parse;

    fn strategy(src: &str) -> Strategy {
        parse(src).unwrap_or_else(|e| panic!("{}", e.message()))
    }

    /// The clause model is the editor's claim about what it can take apart.
    /// If it decomposes something it cannot rebuild, the rows would silently
    /// rewrite a user's rule — so the round trip is the claim, tested.
    #[test]
    fn clauses_rebuild_exactly_what_they_took_apart() {
        let s = strategy(
            "strategy \"c\" language 1\n\
             on roll when point != 0:\n    bet pass\n\
             on roll when point != 0 and point != 6:\n    bet place 6\n\
             on roll when point != 0 and up(pass):\n    bet odds on pass max\n\
             on roll when point != 0 and live-come < 2:\n    bet come\n\
             on roll when come-point(6):\n    bet odds on come 6 max\n\
             on roll when profit >= 15000 or profit <= -20000:\n    leave\n",
        );
        for (i, rule) in s.rules.iter().enumerate() {
            let g = rule.guard.as_ref().unwrap();
            match as_clauses(g) {
                Some(cs) => assert_eq!(
                    from_clauses(&cs).as_ref(),
                    Some(g),
                    "rule {i} did not rebuild"
                ),
                // The `or` rule: declined rather than mangled, which is the
                // other half of the claim.
                None => assert_eq!(i, 5, "rule {i} was declined unexpectedly"),
            }
        }
    }

    /// Every shape the slots offer produces a strategy that compiles and
    /// survives the text round trip — the rows cannot build something the
    /// other editor could not read.
    #[test]
    fn every_shape_the_slots_offer_round_trips() {
        use craps_engine::strategy::{compile, render};
        let mut rules = Vec::new();
        for stmt in [
            Stmt::Bet(BetRef::Pass, AmountExpr::Pressed),
            Stmt::Bet(BetRef::Place(6), AmountExpr::Base),
            Stmt::Bet(BetRef::PassOdds, AmountExpr::MaxOdds),
            Stmt::Bet(BetRef::Field, AmountExpr::Units(Expr::Const(2))),
            Stmt::Press(BetRef::Place(8), AmountExpr::Cents(Expr::Const(2400))),
            Stmt::Regress(BetRef::Place(8), AmountExpr::Base),
            Stmt::Down(BetRef::Place(5)),
            Stmt::Working(BetRef::Hardway(6), false),
            Stmt::Leave,
        ] {
            rules.push(Rule::new(Trigger::Roll, vec![stmt]));
        }
        for clause in [
            Clause::Cmp {
                read: Read::Point,
                op: BinOp::Ne,
                value: 0,
            },
            Clause::Cmp {
                read: Read::Profit,
                op: BinOp::Le,
                value: -20_000,
            },
            Clause::Cmp {
                read: Read::HitsThisShooter(6),
                op: BinOp::Ge,
                value: 2,
            },
            Clause::Truthy(Read::Up(BetRef::Pass)),
        ] {
            let mut r = Rule::new(
                Trigger::Roll,
                vec![Stmt::Bet(BetRef::Pass, AmountExpr::Base)],
            );
            r.guard = from_clauses(&[clause]);
            rules.push(r);
        }
        let s = Strategy::new("every shape", rules);
        let text = render(&s);
        assert_eq!(parse(&text).unwrap(), s, "\n{text}");
        compile(&s).unwrap_or_else(|e| panic!("{}\n{text}", e.message()));
    }

    /// The trigger slot offers every trigger the language has, so the rows
    /// cannot be a subset of the text form by omission.
    #[test]
    fn the_trigger_slot_covers_the_language() {
        let offered: Vec<Trigger> = TRIGGERS
            .iter()
            .map(|(_, t)| *t)
            .chain((2u8..=12).map(Trigger::Total))
            .chain(
                streamed_bets()
                    .into_iter()
                    .flat_map(|b| [Trigger::Win(b), Trigger::Loss(b)]),
            )
            .collect();
        for t in [
            Trigger::SessionStart,
            Trigger::ComeOut,
            Trigger::PointEstablished,
            Trigger::PointMade,
            Trigger::SevenOut,
            Trigger::Roll,
            Trigger::Total(7),
            Trigger::Win(BetRef::Place(6)),
            Trigger::Loss(BetRef::Pass),
        ] {
            assert!(offered.contains(&t), "{t:?} is not offered");
            // And every one of them has a label rather than a debug print.
            assert!(!trigger_label(t).contains('('), "{t:?} labelled badly");
        }
    }

    /// A bet with no win/loss record of its own cannot trigger a rule; the
    /// compiler refuses it, so the slot must not offer it.
    #[test]
    fn odds_are_not_offered_as_triggers() {
        for b in streamed_bets() {
            assert!(
                !matches!(
                    b,
                    BetRef::PassOdds
                        | BetRef::DontPassLay
                        | BetRef::ComeOdds(_)
                        | BetRef::DontComeLay(_)
                ),
                "{b:?} has no stream of its own"
            );
            // The compiler agrees, which is the actual authority.
            let mut r = Rule::new(
                Trigger::Win(b),
                vec![Stmt::Bet(BetRef::Pass, AmountExpr::Base)],
            );
            r.guard = None;
            craps_engine::strategy::compile(&Strategy::new("t", vec![r]))
                .unwrap_or_else(|e| panic!("{b:?}: {}", e.message()));
        }
    }
}
