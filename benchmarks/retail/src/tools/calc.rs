//! `calculate`: arithmetic as Python would evaluate it for the characters the tool allows.

use serde_json::{json, Value};

use super::fail::{bad, R};
use super::value::round2;

/// The `calculate` tool: the expression's value rounded to 2 decimals, as Python prints it.
pub(super) fn calculate(expr: &str) -> R<Value> {
    if !expr.chars().all(|c| "0123456789+-*/(). ".contains(c)) {
        return bad("Invalid characters in expression");
    }
    let mut p = Parser {
        s: expr.chars().filter(|c| *c != ' ').collect(),
        i: 0,
    };
    let v = p.expr()?;
    if p.i != p.s.len() || !v.is_finite() {
        return bad("Invalid expression");
    }
    Ok(json!(format!("{:?}", round2(v))))
}

struct Parser {
    s: Vec<char>,
    i: usize,
}
impl Parser {
    fn peek(&self, off: usize) -> Option<char> {
        self.s.get(self.i + off).copied()
    }
    fn expr(&mut self) -> R<f64> {
        let mut v = self.term()?;
        while let Some(c @ ('+' | '-')) = self.peek(0) {
            self.i += 1;
            let r = self.term()?;
            v = if c == '+' { v + r } else { v - r };
        }
        Ok(v)
    }
    fn term(&mut self) -> R<f64> {
        let mut v = self.unary()?;
        loop {
            match (self.peek(0), self.peek(1)) {
                (Some('/'), next) => {
                    let floor = next == Some('/');
                    self.i += if floor { 2 } else { 1 };
                    let r = self.unary()?;
                    if r == 0.0 {
                        return bad("division by zero");
                    }
                    v = if floor { (v / r).floor() } else { v / r };
                }
                (Some('*'), next) if next != Some('*') => {
                    self.i += 1;
                    v *= self.unary()?;
                }
                _ => return Ok(v),
            }
        }
    }
    fn unary(&mut self) -> R<f64> {
        match self.peek(0) {
            Some('-') => {
                self.i += 1;
                Ok(-self.unary()?)
            }
            Some('+') => {
                self.i += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }
    fn power(&mut self) -> R<f64> {
        let base = self.atom()?;
        if self.peek(0) == Some('*') && self.peek(1) == Some('*') {
            self.i += 2;
            let e = self.unary()?;
            return Ok(base.powf(e));
        }
        Ok(base)
    }
    fn atom(&mut self) -> R<f64> {
        match self.peek(0) {
            Some('(') => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek(0) != Some(')') {
                    return bad("Invalid expression");
                }
                self.i += 1;
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == '.' => {
                let start = self.i;
                while self.peek(0).is_some_and(|c| c.is_ascii_digit() || c == '.') {
                    self.i += 1;
                }
                let t: String = self.s[start..self.i].iter().collect();
                t.parse().or_else(|_| bad("Invalid expression"))
            }
            _ => bad("Invalid expression"),
        }
    }
}
