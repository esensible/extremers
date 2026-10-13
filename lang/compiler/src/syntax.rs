//! Lexer and parser for the expr subset (spec §2.1).
//!
//! Precedence, loosest first (probed against expr v1.17.8):
//!   ?: and if (right-nesting) | || or | && and | == != < <= > >= | .. | + - |
//!   * / % | unary - ! not | ** (right) | ?? | postfix
//!
//! - `<`, `<=`, `>`, `>=` chain (`1 < x < 5`); `==` and `!=` are plain
//!   left-associative operators at the same level, so `1 == 1 < 2` is
//!   `(1 == 1) < 2`.
//! - A unary operator's operand includes `**` and `??`: `-2 ** 2 == -4`,
//!   `!a ?? true == !(a ?? true)`.
//! - `??` chains with itself, but no other binary operator may follow a `??`
//!   expression without parentheses.

use crate::diag::{Diag, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    /// `#`, `#index`, `#acc`: predicates (not in this slice).
    Pointer(String),
    Op(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

const OPS: &[&str] = &[
    "===", "**", "==", "!=", "<=", ">=", "&&", "||", "??", "?.", "..", "=>", "...", "+", "-", "*", "/", "%", "<", ">",
    "!", "?", ":", ";", "(", ")", "{", "}", "[", "]", ",", ".", "|", "^", "=",
];

pub fn lex(src: &str) -> Result<Vec<Token>, Diag> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if src[i..].starts_with("/*") {
            let end = src[i + 2..].find("*/").ok_or_else(|| {
                Diag::error("E-PARSE", Span::new(i, i + 2), "this comment never ends").help("close it with `*/`")
            })?;
            i += 2 + end + 2;
            continue;
        }
        let start = i;
        if c.is_ascii_digit() {
            let (tok, len) = number(&src[i..]).map_err(|m| Diag::error("E-PARSE", Span::new(i, i + 1), m))?;
            i += len;
            out.push(Token { tok, span: Span::new(start, i) });
            continue;
        }
        if c == b'"' || c == b'\'' {
            let (s, len) = string(&src[i..]).map_err(|m| Diag::error("E-PARSE", Span::new(i, i + 1), m))?;
            i += len;
            out.push(Token { tok: Tok::Str(s), span: Span::new(start, i) });
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$') {
                i += 1;
            }
            out.push(Token { tok: Tok::Ident(src[start..i].to_string()), span: Span::new(start, i) });
            continue;
        }
        if c == b'#' {
            i += 1;
            while i < b.len() && b[i].is_ascii_alphanumeric() {
                i += 1;
            }
            out.push(Token { tok: Tok::Pointer(src[start..i].to_string()), span: Span::new(start, i) });
            continue;
        }
        match OPS.iter().find(|op| src[i..].starts_with(**op)) {
            Some(op) => {
                i += op.len();
                out.push(Token { tok: Tok::Op(op), span: Span::new(start, i) });
            }
            None => {
                let ch = src[i..].chars().next().unwrap();
                return Err(Diag::error("E-PARSE", Span::new(i, i + ch.len_utf8()), format!("unexpected character `{ch}`")));
            }
        }
    }
    out.push(Token { tok: Tok::Eof, span: Span::new(src.len(), src.len()) });
    Ok(out)
}

fn number(s: &str) -> Result<(Tok, usize), String> {
    let b = s.as_bytes();
    if s.starts_with("0x") || s.starts_with("0X") {
        let mut i = 2;
        while i < b.len() && (b[i].is_ascii_hexdigit() || b[i] == b'_') {
            i += 1;
        }
        let digits: String = s[2..i].chars().filter(|&c| c != '_').collect();
        return i64::from_str_radix(&digits, 16)
            .map(|v| (Tok::Int(v), i))
            .map_err(|_| format!("`{}` isn't a valid integer", &s[..i]));
    }
    let mut i = 0;
    while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
        i += 1;
    }
    let mut float = false;
    // `1..3` is a range, not the float `1.` followed by `.3`.
    if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
        float = true;
        i += 1;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
            i += 1;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            float = true;
            i = j;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    let text: String = s[..i].chars().filter(|&c| c != '_').collect();
    if float {
        let v: f64 = text.parse().map_err(|_| format!("`{}` isn't a valid number", &s[..i]))?;
        if v.is_infinite() {
            return Err(format!("`{}` is too large for a float", &s[..i]));
        }
        Ok((Tok::Float(v), i))
    } else {
        text.parse::<i64>()
            .map(|v| (Tok::Int(v), i))
            .map_err(|_| format!("`{}` is too large for an integer (the largest is 9223372036854775807)", &s[..i]))
    }
}

fn string(s: &str) -> Result<(String, usize), String> {
    let quote = s.as_bytes()[0] as char;
    let mut out = String::new();
    let mut chars = s.char_indices().skip(1);
    while let Some((i, c)) = chars.next() {
        match c {
            c if c == quote => return Ok((out, i + 1)),
            '\n' => break,
            '\\' => {
                let (_, e) = chars.next().ok_or("this string never ends")?;
                match e {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    '\'' => out.push('\''),
                    'u' => {
                        let hex: String = (0..4).filter_map(|_| chars.next().map(|(_, c)| c)).collect();
                        let v = u32::from_str_radix(&hex, 16).map_err(|_| "a `\\u` escape needs four hex digits")?;
                        out.push(char::from_u32(v).ok_or("that `\\u` escape isn't a character")?);
                    }
                    other => return Err(format!("unknown escape `\\{other}`")),
                }
            }
            c => out.push(c),
        }
    }
    Err("this string never ends".to_string())
}

// ---------------------------------------------------------------------------
// The tree

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Coalesce,
    /// `x in xs` (`x not in xs` is `!(x in xs)`).
    In,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Pow => "**",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::Coalesce => "??",
            BinOp::In => "in",
        }
    }
    fn relational(self) -> bool {
        matches!(self, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Name(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `a < b <= c …`: each operand evaluated once, short-circuiting.
    Chain(Box<Expr>, Vec<(BinOp, Expr)>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Let { name: String, name_span: Span, value: Box<Expr>, body: Box<Expr> },
    Call { name: String, name_span: Span, args: Vec<Expr> },
    /// `{a: e, "b": e}`: (key, key span, value).
    Record(Vec<(String, Span, Expr)>),
    /// `base.f?.g["h"]`: member steps applied to `base`. A `?.` whose target
    /// is nil makes the rest of the chain nil (probe: `r.o?.x.y` is nil).
    Path { base: Box<Expr>, steps: Vec<Step> },
    /// `#`, `#index` or `#acc` (the name without `#`; `#` alone is "").
    Pointer(String),
    /// `[a, b, …]`.
    List(Vec<Expr>),
    /// `xs[i]` with a computed index.
    Index(Box<Expr>, Box<Expr>),
    /// `xs[a:b]`, either bound left out.
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),
    /// `a..b`: the ints from `a` to `b`.
    Range(Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug)]
pub struct Step {
    pub field: String,
    /// `?.` rather than `.`.
    pub optional: bool,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// The parser

pub fn parse(src: &str) -> Result<Expr, Diag> {
    let toks = lex(src)?;
    let mut p = Parser { toks, at: 0, parens: Vec::new() };
    let e = p.expr()?;
    let t = p.peek();
    if t.tok != Tok::Eof {
        return Err(p.unexpected("the end of the expression"));
    }
    Ok(e)
}

struct Parser {
    toks: Vec<Token>,
    at: usize,
    /// Spans of parenthesised expressions, parentheses included.
    parens: Vec<Span>,
}

const BP_OR: u8 = 1;
const BP_AND: u8 = 2;
const BP_CMP: u8 = 3;
const BP_RANGE: u8 = 4;
const BP_ADD: u8 = 5;
const BP_MUL: u8 = 6;
const BP_POW: u8 = 8; // a unary operand is parsed at this level, so ** and ?? bind tighter than unary
const BP_COALESCE: u8 = 9;

impl Parser {
    fn peek(&self) -> &Token {
        &self.toks[self.at]
    }
    fn next(&mut self) -> Token {
        let t = self.toks[self.at].clone();
        if t.tok != Tok::Eof {
            self.at += 1;
        }
        t
    }
    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek().tok, Tok::Op(o) if o == op)
    }
    fn is_word(&self, w: &str) -> bool {
        matches!(&self.peek().tok, Tok::Ident(s) if s == w)
    }
    fn expect_op(&mut self, op: &'static str, hint: Option<&str>) -> Result<Token, Diag> {
        if self.is_op(op) {
            return Ok(self.next());
        }
        let mut d = self.unexpected(&format!("`{op}`"));
        if d.help.is_none() {
            if let Some(h) = hint {
                d = d.help(h);
            }
        }
        Err(d)
    }

    /// An error at the current token, with a fixed hint for the common slips (spec §5.1).
    fn unexpected(&self, wanted: &str) -> Diag {
        let t = self.peek();
        let found = match &t.tok {
            Tok::Eof => "the end of the input".to_string(),
            Tok::Int(v) => format!("`{v}`"),
            Tok::Float(v) => format!("`{v}`"),
            Tok::Str(s) => format!("\"{s}\""),
            Tok::Ident(s) | Tok::Pointer(s) => format!("`{s}`"),
            Tok::Op(o) => format!("`{o}`"),
        };
        let d = Diag::error("E-PARSE", t.span, format!("expected {wanted}, found {found}"));
        match &t.tok {
            Tok::Ident(s) if s == "then" => d.help("write `c ? a : b`, or `if c { a } else { b }`"),
            Tok::Op("=>") => d.help("expr has no lambdas: use a predicate with `#`"),
            Tok::Op("...") => d.help("expr has no spread: write every field"),
            Tok::Op("===") => d.help("use `==`"),
            Tok::Op("=") => d.help("names are bound once, with `let name = value; …`; to compare, use `==`"),
            _ => d,
        }
    }

    fn unsupported(&self, span: Span, what: &str, help: &str) -> Diag {
        Diag::error("E-UNSUPPORTED", span, format!("{what} aren't implemented yet")).help(help)
    }

    /// expression := `let` name `=` expression `;` expression | conditional
    fn expr(&mut self) -> Result<Expr, Diag> {
        if self.is_word("let") {
            let start = self.next().span;
            let name_tok = self.next();
            let name = match name_tok.tok {
                Tok::Ident(n) if !is_keyword(&n) => n,
                _ => {
                    self.at -= 1;
                    return Err(self.unexpected("a name after `let`"));
                }
            };
            self.expect_op("=", None)?;
            let value = self.expr()?;
            if !self.is_op(";") {
                let mut d = self.unexpected("`;` after the `let` value");
                d = d.help("each `let` ends with `;`: `let a = 1; let b = 2; …`");
                return Err(d);
            }
            self.next();
            let body = self.expr()?;
            let span = start.to(body.span);
            return Ok(Expr {
                kind: ExprKind::Let { name, name_span: name_tok.span, value: Box::new(value), body: Box::new(body) },
                span,
            });
        }
        if self.is_word("if") {
            return self.if_expr();
        }
        let mut c = self.binary(0)?;
        // `a | f(b)` is `f(a, b)`; pipes are the loosest operator.
        while self.is_op("|") {
            let bar = self.next().span;
            let name = self.next();
            let Tok::Ident(f) = name.tok else {
                self.at -= 1;
                return Err(self.unexpected("a builtin call after `|`").help("a pipe feeds a call: `xs | filter(.ok) | map(.v)`"));
            };
            if !self.is_op("(") {
                return Err(Diag::error("E-PARSE", bar.to(name.span), format!("`{f}` after `|` needs its arguments")).help(format!("`… | {f}(…)`")));
            }
            let call = self.call(f, name.span)?;
            let ExprKind::Call { name, name_span, mut args } = call.kind else { unreachable!() };
            let span = c.span.to(call.span);
            args.insert(0, c);
            c = Expr { kind: ExprKind::Call { name, name_span, args }, span };
        }
        if self.is_op("?") {
            self.next();
            let a = self.expr()?;
            self.expect_op(":", Some("a conditional is `c ? a : b`"))?;
            let b = self.expr()?;
            let span = c.span.to(b.span);
            return Ok(Expr { kind: ExprKind::Cond(Box::new(c), Box::new(a), Box::new(b)), span });
        }
        Ok(c)
    }

    /// `if c { a } else { b }`, `else if …` nesting.
    fn if_expr(&mut self) -> Result<Expr, Diag> {
        let start = self.next().span;
        let c = self.binary(0)?;
        self.expect_op("{", Some("`if c { a } else { b }`"))?;
        let a = self.expr()?;
        self.expect_op("}", None)?;
        if !self.is_word("else") {
            return Err(self.unexpected("`else`").help("`if` needs an `else`: `if c { a } else { b }`"));
        }
        self.next();
        let b = if self.is_word("if") {
            self.if_expr()?
        } else {
            self.expect_op("{", Some("`else { b }` or `else if …`"))?;
            let b = self.expr()?;
            self.expect_op("}", None)?;
            b
        };
        let span = start.to(b.span);
        Ok(Expr { kind: ExprKind::Cond(Box::new(c), Box::new(a), Box::new(b)), span })
    }

    fn binop(&self) -> Option<(BinOp, u8)> {
        let op = match &self.peek().tok {
            Tok::Op(o) => match *o {
                "||" => BinOp::Or,
                "&&" => BinOp::And,
                "==" => BinOp::Eq,
                "!=" => BinOp::Ne,
                "<" => BinOp::Lt,
                "<=" => BinOp::Le,
                ">" => BinOp::Gt,
                ">=" => BinOp::Ge,
                "+" => BinOp::Add,
                "-" => BinOp::Sub,
                "*" => BinOp::Mul,
                "/" => BinOp::Div,
                "%" => BinOp::Mod,
                "**" => BinOp::Pow,
                "??" => BinOp::Coalesce,
                _ => return None,
            },
            Tok::Ident(w) => match w.as_str() {
                "or" => BinOp::Or,
                "and" => BinOp::And,
                "in" => BinOp::In,
                _ => return None,
            },
            _ => return None,
        };
        let bp = match op {
            BinOp::Or => BP_OR,
            BinOp::And => BP_AND,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In => BP_CMP,
            BinOp::Add | BinOp::Sub => BP_ADD,
            BinOp::Mul | BinOp::Div | BinOp::Mod => BP_MUL,
            BinOp::Pow => BP_POW,
            BinOp::Coalesce => BP_COALESCE,
        };
        Some((op, bp))
    }

    /// Binary operators binding tighter than `min` (Pratt parsing).
    fn binary(&mut self, min: u8) -> Result<Expr, Diag> {
        let mut lhs = self.unary()?;
        loop {
            // `a..b` binds looser than `+` and tighter than comparisons.
            if self.is_op("..") {
                if BP_RANGE <= min {
                    break;
                }
                self.next();
                let rhs = self.binary(BP_RANGE)?;
                let span = lhs.span.to(rhs.span);
                lhs = Expr { kind: ExprKind::Range(Box::new(lhs), Box::new(rhs)), span };
                continue;
            }
            // `x not in xs`: `!(x in xs)`.
            if self.is_word("not") && self.peek_word_at(1, "in") && BP_CMP > min {
                let not = self.next().span;
                self.next();
                let rhs = self.binary(BP_CMP)?;
                let span = lhs.span.to(rhs.span);
                let inner = Expr { kind: ExprKind::Binary(BinOp::In, Box::new(lhs), Box::new(rhs)), span };
                lhs = Expr { kind: ExprKind::Unary(UnOp::Not, Box::new(inner)), span: span.to(not) };
                continue;
            }
            let Some((op, bp)) = self.binop() else { break };
            if bp <= min {
                break;
            }
            self.next();
            // `**` is right-associative: its right side may hold another `**`.
            let rhs = if op == BinOp::Pow { self.binary(BP_POW - 1)? } else { self.binary(bp)? };
            if op.relational() && !self.parenthesised(&lhs) {
                // `a < b < c` chains; `(a < b) < c` doesn't.
                match &mut lhs.kind {
                    ExprKind::Chain(_, rest) => {
                        lhs.span = lhs.span.to(rhs.span);
                        rest.push((op, rhs));
                        continue;
                    }
                    ExprKind::Binary(prev, _, _) if prev.relational() => {
                        let span = lhs.span.to(rhs.span);
                        let ExprKind::Binary(prev, a, b) = std::mem::replace(&mut lhs.kind, ExprKind::Nil) else {
                            unreachable!()
                        };
                        lhs = Expr { kind: ExprKind::Chain(a, vec![(prev, *b), (op, rhs)]), span };
                        continue;
                    }
                    _ => {}
                }
            }
            let span = lhs.span.to(rhs.span);
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span };
            if op == BinOp::Coalesce {
                if let Some((next, _)) = self.binop() {
                    if next != BinOp::Coalesce {
                        let t = self.peek();
                        return Err(Diag::error(
                            "E-PARSE",
                            t.span,
                            format!("`{}` can't follow a `??` expression without parentheses", next.symbol()),
                        )
                        .help("wrap the `??`: `(a ?? b) + 1`"));
                    }
                }
            }
        }
        Ok(lhs)
    }

    fn peek_word_at(&self, ahead: usize, w: &str) -> bool {
        matches!(self.toks.get(self.at + ahead).map(|t| &t.tok), Some(Tok::Ident(s)) if s == w)
    }

    /// Whether `e` was written in parentheses. A parenthesised comparison doesn't chain.
    fn parenthesised(&self, e: &Expr) -> bool {
        self.parens.contains(&e.span)
    }

    fn unary(&mut self) -> Result<Expr, Diag> {
        let t = self.peek().clone();
        let op = match &t.tok {
            Tok::Op("-") => Some(UnOp::Neg),
            Tok::Op("+") => {
                self.next();
                return self.binary(BP_POW - 1);
            }
            Tok::Op("!") => Some(UnOp::Not),
            Tok::Ident(w) if w == "not" => Some(UnOp::Not),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            let e = self.binary(BP_POW - 1)?;
            let span = t.span.to(e.span);
            return Ok(Expr { kind: ExprKind::Unary(op, Box::new(e)), span });
        }
        let e = self.primary()?;
        self.postfix(e)
    }

    /// Member access: `.f`, `?.f`, `["f"]`. Steps after a parenthesised path
    /// continue that path's chain, as in expr (probe: `(r.o?.x).y` is nil).
    fn postfix(&mut self, mut e: Expr) -> Result<Expr, Diag> {
        loop {
            let t = self.peek().clone();
            let step = if self.is_op(".") || self.is_op("?.") {
                let optional = self.is_op("?.");
                self.next();
                if self.is_op("[") {
                    return Err(self.unsupported(self.peek().span, "optional indexes (`xs?.[i]`)", "test the list first: `xs != nil ? xs[i] : …`, or use `get(xs, i)`"));
                }
                let name = self.next();
                let Tok::Ident(field) = name.tok else {
                    self.at -= 1;
                    return Err(self.unexpected("a field name"));
                };
                if self.is_op("(") {
                    let n = match &e.kind {
                        ExprKind::Name(n) => n.clone(),
                        _ => "xs".to_string(),
                    };
                    let help = match field.as_str() {
                        "length" => format!("builtins are called as functions: `len({n})`"),
                        _ => format!("builtins are called as functions: `{field}({n}, …)`"),
                    };
                    return Err(Diag::error("E-PARSE", t.span.to(name.span), format!("`.{field}(…)` isn't expr: there are no methods")).help(help));
                }
                if field == "length" {
                    if let ExprKind::Name(n) = &e.kind {
                        return Err(Diag::error("E-PARSE", t.span.to(name.span), format!("`{n}.length` isn't expr")).help(format!("use `len({n})`")));
                    }
                }
                Step { field, optional, span: t.span.to(name.span) }
            } else if self.is_op("[") {
                // `r["f"]` with a literal key is `r.f`; anything else is list indexing.
                match self.toks.get(self.at + 1).map(|t| t.tok.clone()) {
                    Some(Tok::Str(key)) if matches!(self.toks.get(self.at + 2).map(|t| &t.tok), Some(Tok::Op("]"))) => {
                        self.next();
                        self.next();
                        let close = self.next();
                        Step { field: key, optional: false, span: t.span.to(close.span) }
                    }
                    _ => {
                        self.next();
                        // `xs[a:b]`, `xs[a:]`, `xs[:b]`, `xs[:]`.
                        let lo = if self.is_op(":") { None } else { Some(self.expr()?) };
                        if self.is_op(":") {
                            self.next();
                            let hi = if self.is_op("]") { None } else { Some(self.expr()?) };
                            let close = self.expect_op("]", None)?;
                            let span = e.span.to(close.span);
                            e = Expr { kind: ExprKind::Slice(Box::new(e), lo.map(Box::new), hi.map(Box::new)), span };
                            continue;
                        }
                        let i = lo.expect("an index, since `:` didn't follow `[`");
                        let close = self.expect_op("]", None)?;
                        let span = e.span.to(close.span);
                        e = Expr { kind: ExprKind::Index(Box::new(e), Box::new(i)), span };
                        continue;
                    }
                }
            } else {
                return Ok(e);
            };
            let span = e.span.to(step.span);
            e = match e.kind {
                ExprKind::Path { base, mut steps } => {
                    steps.push(step);
                    Expr { kind: ExprKind::Path { base, steps }, span }
                }
                kind => Expr { kind: ExprKind::Path { base: Box::new(Expr { kind, span: e.span }), steps: vec![step] }, span },
            };
        }
    }

    /// `{a: e, "b": e}`, after the `{`.
    fn record(&mut self, open: Span) -> Result<Expr, Diag> {
        let mut fields = Vec::new();
        while !self.is_op("}") {
            let k = self.next();
            let key = match k.tok {
                Tok::Ident(s) | Tok::Str(s) => s,
                _ => {
                    self.at -= 1;
                    return Err(self.unexpected("a field name"));
                }
            };
            if self.is_op(",") || self.is_op("}") {
                return Err(Diag::error("E-PARSE", k.span, format!("the field `{key}` has no value"))
                    .help(format!("write every field in full: `{{{key}: {key}}}`")));
            }
            self.expect_op(":", Some("a record is `{name: value, …}`"))?;
            let v = self.expr()?;
            fields.push((key, k.span, v));
            if self.is_op(",") {
                self.next();
            } else if !self.is_op("}") {
                return Err(self.unexpected("`,` or `}`"));
            }
        }
        let close = self.next();
        Ok(Expr { kind: ExprKind::Record(fields), span: open.to(close.span) })
    }

    fn primary(&mut self) -> Result<Expr, Diag> {
        let t = self.next();
        let kind = match t.tok {
            Tok::Int(v) => ExprKind::Int(v),
            Tok::Float(v) => ExprKind::Float(v),
            Tok::Str(s) => ExprKind::Str(s),
            Tok::Ident(w) => match w.as_str() {
                "true" => ExprKind::Bool(true),
                "false" => ExprKind::Bool(false),
                "nil" => ExprKind::Nil,
                "let" | "if" => {
                    // `let`/`if` in an operand position, e.g. `1 + let x = …`.
                    self.at -= 1;
                    return self.expr();
                }
                _ if self.is_op("(") => return self.call(w, t.span),
                _ => ExprKind::Name(w),
            },
            Tok::Op("(") => {
                let inner = self.expr()?;
                let close = self.expect_op(")", None)?;
                let span = t.span.to(close.span);
                self.parens.push(span);
                return self.postfix(Expr { kind: inner.kind, span });
            }
            Tok::Op("[") => {
                let mut items = Vec::new();
                while !self.is_op("]") {
                    items.push(self.expr()?);
                    if self.is_op(",") {
                        self.next();
                    } else if !self.is_op("]") {
                        return Err(self.unexpected("`,` or `]`"));
                    }
                }
                let close = self.next();
                return Ok(Expr { kind: ExprKind::List(items), span: t.span.to(close.span) });
            }
            Tok::Op("{") => return self.record(t.span),
            Tok::Pointer(p) => {
                let name = p.trim_start_matches('#').to_string();
                if !matches!(name.as_str(), "" | "index" | "acc") {
                    return Err(Diag::error("E-PARSE", t.span, format!("`{p}` isn't a pointer")).help("the pointers are `#`, `#index` and `#acc`"));
                }
                ExprKind::Pointer(name)
            }
            // `.f` at the start of an operand is `#.f`, inside a predicate.
            Tok::Op(".") if matches!(self.peek().tok, Tok::Ident(_)) => {
                self.at -= 1;
                let hash = Expr { kind: ExprKind::Pointer(String::new()), span: t.span };
                return self.postfix(hash);
            }
            _ => {
                self.at -= 1;
                return Err(self.unexpected("a value"));
            }
        };
        Ok(Expr { kind, span: t.span })
    }

    fn call(&mut self, name: String, name_span: Span) -> Result<Expr, Diag> {
        self.next(); // (
        let mut args = Vec::new();
        while !self.is_op(")") {
            // A predicate may be a braced block, `filter(xs, { # > 1 })`; a
            // record literal there needs parentheses, as in expr.
            if args.len() == 1 && PREDICATE_BUILTINS.contains(&name.as_str()) && self.is_op("{") {
                let open = self.next().span;
                let body = self.expr()?;
                if !self.is_op("}") {
                    let d = self.unexpected("`}` closing the predicate");
                    return Err(if self.is_op(":") {
                        Diag::error("E-PARSE", d.span.unwrap(), "a record inside a predicate needs parentheses")
                            .help("write `map(xs, ({v: #}))`")
                    } else {
                        d
                    });
                }
                let close = self.next().span;
                args.push(Expr { kind: body.kind, span: open.to(close) });
                if self.is_op(",") {
                    self.next();
                }
                continue;
            }
            args.push(self.expr()?);
            if self.is_op(",") {
                self.next();
            } else if !self.is_op(")") {
                return Err(self.unexpected("`,` or `)`"));
            }
        }
        let close = self.next();
        Ok(Expr { kind: ExprKind::Call { name, name_span, args }, span: name_span.to(close.span) })
    }
}

/// Builtins whose second argument is a predicate (evaluated per item).
pub const PREDICATE_BUILTINS: &[&str] = &[
    "map", "filter", "all", "any", "none", "one", "count", "find", "findIndex", "findLast", "findLastIndex", "sum",
    "reduce", "sortBy", "groupBy",
];

fn is_keyword(w: &str) -> bool {
    matches!(w, "let" | "if" | "else" | "true" | "false" | "nil" | "and" | "or" | "not" | "in")
}
