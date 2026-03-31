//! Recursive-descent parser for Gradle Groovy DSL build scripts.
//!
//! Transforms a stream of tokens (from [`lexer`]) into an AST (from [`ast`]).
//!
//! # Grammar Overview
//!
//! ```text
//! Script         := Statement* Eof
//! Statement      := ImportStatement | VarDecl | AssignmentOrExpr
//! AssignmentOrExpr := Expression ('=' Expression)?
//! Expression     := TernaryExpr
//! TernaryExpr    := ElvisExpr ('?' Expression ':' Expression)?
//! ElvisExpr      := BinaryExpr ('?:' BinaryExpr)?
//! BinaryExpr     := UnaryExpr (BinaryOp UnaryExpr)*   [precedence climbing]
//! UnaryExpr      := PrefixOp UnaryExpr | PostfixExpr
//! PostfixExpr    := PrimaryExpr ('.' Identifier | '?.' Identifier | '(' Args ')' | '[' Expr ']' | '{' Closure '}')*
//! PrimaryExpr    := StringLit | NumberLit | BoolLit | NullLit | ListOrMap | Closure | ParenExpr | Identifier
//! ```
//!
//! # Error Handling
//!
//! The parser uses best-effort error recovery: when a parse error is encountered,
//! it records the error and skips ahead to the next statement boundary (semicolon,
//! closing brace, or a token that looks like a statement start). This allows
//! partial parsing of scripts with errors.

use std::fmt;

use crate::server::groovy_parser::ast::*;
use crate::server::groovy_parser::lexer::{tokenize, Token, TokenKind};

// ─── Span Conversion ─────────────────────────────────────────────────────────

/// Convert a lexer `Span` to an AST `Span`.
fn to_ast_span(s: &crate::server::groovy_parser::lexer::Span) -> Span {
    Span::new(s.start, s.end, s.line, s.column)
}

/// Merge two AST spans.
fn merge_spans(a: Span, b: Span) -> Span {
    a.merge(&b)
}

// ─── Parse Error ─────────────────────────────────────────────────────────────

/// A parse error with context information for diagnostics.
#[derive(Debug, Clone)]
pub struct ParseError {
    /// Human-readable error description.
    pub message: String,
    /// Token kinds that were expected at this position.
    pub expected: Vec<String>,
    /// The token that was actually found.
    pub found: Option<String>,
    /// Source location of the error.
    pub span: Span,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "parse error at {}:{}: {}",
            self.span.line, self.span.column, self.message
        )?;
        if !self.expected.is_empty() {
            write!(f, " (expected: {})", self.expected.join(", "))?;
        }
        if let Some(ref found) = self.found {
            write!(f, " (found: {})", found)?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

// ─── Parse Result ────────────────────────────────────────────────────────────

/// The result of parsing a build script.
#[derive(Debug, Clone)]
pub struct ParseResult {
    /// The parsed AST (may be partial if errors occurred).
    pub script: Script,
    /// Non-fatal parse errors collected during parsing.
    pub errors: Vec<ParseError>,
}

impl ParseResult {
    /// Returns true if any parse errors were encountered.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

// ─── Parser ──────────────────────────────────────────────────────────────────

/// Recursive-descent parser that converts a token stream into a Groovy DSL AST.
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    errors: Vec<ParseError>,
}

impl Parser {
    /// Create a new parser from a pre-tokenized stream.
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            errors: Vec::new(),
        }
    }

    // ── Token access ─────────────────────────────────────────────────────────

    #[inline]
    fn cur(&self) -> &Token {
        &self.tokens[self.pos]
    }

    #[inline]
    fn peek(&self, offset: usize) -> &Token {
        let idx = (self.pos + offset).min(self.tokens.len().saturating_sub(1));
        &self.tokens[idx]
    }

    #[inline]
    fn at(&self, kind: TokenKind) -> bool {
        self.cur().kind == kind
    }

    #[inline]
    fn at_end(&self) -> bool {
        self.cur().kind == TokenKind::Eof
    }

    /// If the current token is `kind`, consume it and return true.
    #[inline]
    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Consume the current token and return it.
    #[inline]
    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        if !self.at_end() {
            self.pos += 1;
        }
        tok
    }

    /// Expect the current token to be `kind`. On success, consume and return it.
    /// On failure, record an error and return Err.
    fn expect(&mut self, kind: TokenKind) -> Result<&Token, ParseError> {
        if self.at(kind) {
            Ok(self.advance())
        } else {
            Err(ParseError {
                message: format!("expected {}", kind),
                expected: vec![kind.to_string()],
                found: Some(self.cur().text.clone()),
                span: to_ast_span(&self.cur().span),
            })
        }
    }

    fn error_with_expected(&mut self, message: impl Into<String>, expected: Vec<String>) {
        self.errors.push(ParseError {
            message: message.into(),
            expected,
            found: Some(self.cur().text.clone()),
            span: to_ast_span(&self.cur().span),
        });
    }

    // ── Error Recovery ───────────────────────────────────────────────────────

    /// Skip tokens until we reach a plausible statement boundary.
    fn recover(&mut self) {
        let mut depth_paren: u32 = 0;
        let mut depth_brace: u32 = 0;
        let mut depth_bracket: u32 = 0;

        loop {
            if self.at_end() {
                break;
            }

            match self.cur().kind {
                TokenKind::LParen => depth_paren += 1,
                TokenKind::RParen => {
                    if depth_paren > 0 {
                        depth_paren -= 1;
                    } else {
                        break;
                    }
                }
                TokenKind::LBrace => depth_brace += 1,
                TokenKind::RBrace => {
                    if depth_brace > 0 {
                        depth_brace -= 1;
                    } else {
                        break;
                    }
                }
                TokenKind::LBracket => depth_bracket += 1,
                TokenKind::RBracket => {
                    if depth_bracket > 0 {
                        depth_bracket -= 1;
                    } else {
                        break;
                    }
                }
                _ => {}
            }

            if depth_paren == 0 && depth_brace == 0 && depth_bracket == 0
                && matches!(self.cur().kind, TokenKind::Semicolon | TokenKind::Eof) {
                    self.eat(TokenKind::Semicolon);
                    break;
                }

            self.advance();
        }
    }

    // ── Top-level parse ──────────────────────────────────────────────────────

    /// Parse the full script, returning a `ParseResult`.
    pub fn parse(mut self) -> ParseResult {
        let start_span = if self.tokens.is_empty() {
            Span::unknown()
        } else {
            to_ast_span(&self.tokens[0].span)
        };

        let mut statements = Vec::with_capacity(self.tokens.len() / 4);
        let mut comments = Vec::with_capacity(self.tokens.len() / 8);

        while !self.at_end() {
            // Collect comments if they appear
            while matches!(
                self.cur().kind,
                TokenKind::LineComment | TokenKind::BlockComment
            ) {
                let tok = self.advance();
                comments.push(Comment {
                    span: to_ast_span(&tok.span),
                    text: tok.text.clone(),
                    is_block: tok.kind == TokenKind::BlockComment,
                });
            }

            // Skip semicolons between statements
            while self.eat(TokenKind::Semicolon) {}

            if self.at_end() {
                break;
            }

            match self.parse_statement() {
                Ok(stmt) => statements.push(stmt),
                Err(e) => {
                    self.errors.push(e);
                    let before = self.pos;
                    self.recover();
                    // If recover didn't advance, force-advance to prevent infinite loops
                    if self.pos == before && !self.at_end() {
                        self.advance();
                    }
                }
            }
        }

        let end_span = if let Some(last) = self.tokens.get(self.pos.saturating_sub(1)) {
            to_ast_span(&last.span)
        } else {
            start_span
        };

        ParseResult {
            script: Script {
                span: merge_spans(start_span, end_span),
                dialect: Dialect::Groovy,
                statements,
                comments,
            },
            errors: self.errors,
        }
    }

    // ── Statement Parsing ────────────────────────────────────────────────────

    fn parse_statement(&mut self) -> Result<Stmt, ParseError> {
        // Import statement
        if self.at(TokenKind::Import) {
            return self.parse_import();
        }

        // Variable declarations: def x = ..., val x = ..., var x = ...
        if self.is_var_decl_start() {
            return self.parse_var_decl();
        }

        // Expression statement (may become assignment)
        self.parse_expr_or_assignment()
    }

    fn is_var_decl_start(&self) -> bool {
        matches!(
            self.cur().kind,
            TokenKind::Def | TokenKind::Val | TokenKind::Var
        )
    }

    fn parse_import(&mut self) -> Result<Stmt, ParseError> {
        let start_span = to_ast_span(&self.cur().span);
        self.eat(TokenKind::Import); // consume 'import'

        let mut is_static = false;
        if self.at(TokenKind::Identifier) && self.cur().text == "static" {
            is_static = true;
            self.advance();
        }

        // Parse dotted path
        let mut path_parts = Vec::new();
        let first = self.expect(TokenKind::Identifier)?;
        path_parts.push(first.text.clone());

        while self.eat(TokenKind::Dot) {
            if self.at(TokenKind::Star) {
                path_parts.push("*".to_string());
                self.advance();
                break;
            }
            let part = self.expect(TokenKind::Identifier)?;
            path_parts.push(part.text.clone());
        }

        let is_wildcard = path_parts.last().map(|s| s.as_str()) == Some("*");
        let path = path_parts.join(".");

        let mut alias = None;
        if self.at(TokenKind::As) {
            self.advance();
            alias = Some(self.expect(TokenKind::Identifier)?.text.clone());
        }

        self.eat(TokenKind::Semicolon);

        let end_span = to_ast_span(&self.peek(0).span);

        Ok(Stmt::Import(ImportStmt {
            span: merge_spans(start_span, end_span),
            path,
            is_wildcard,
            is_static,
            alias,
        }))
    }

    fn parse_var_decl(&mut self) -> Result<Stmt, ParseError> {
        let start_span = to_ast_span(&self.cur().span);
        let kind = match self.cur().kind {
            TokenKind::Def => VarKind::Def,
            TokenKind::Val => VarKind::Val,
            TokenKind::Var => VarKind::Var,
            _ => unreachable!("called is_var_decl_start incorrectly"),
        };
        self.advance(); // consume def/val/var

        let name = self.expect(TokenKind::Identifier)?.text.clone();

        // Kotlin type annotation: val name: String = ...
        let mut type_annotation = None;
        if self.eat(TokenKind::Colon) {
            type_annotation = Some(self.parse_type_reference()?);
        }

        // Check for 'by' delegation (Kotlin: val foo by delegate)
        let mut delegate = None;
        if self.at(TokenKind::By) {
            self.advance();
            let delegate_expr = self.parse_expression()?;
            delegate = Some(Box::new(delegate_expr));
        }

        // Check for initializer
        let mut initializer = None;
        if self.eat(TokenKind::Eq) {
            let expr = self.parse_expression()?;
            initializer = Some(Box::new(expr));
        }

        self.eat(TokenKind::Semicolon);

        let end_span = to_ast_span(&self.peek(0).span);

        Ok(Stmt::VarDecl(VarDecl {
            span: merge_spans(start_span, end_span),
            kind,
            name,
            type_annotation,
            delegate,
            initializer,
        }))
    }

    /// Parse a Kotlin type reference: `String`, `List<String>`, `Map<String, Int>`, `String?`.
    fn parse_type_reference(&mut self) -> Result<String, ParseError> {
        let mut type_name = self.expect(TokenKind::Identifier)?.text.clone();

        // Nullable: String?
        if self.eat(TokenKind::Question) {
            type_name.push('?');
        }

        // Generic types: List<String>, Map<String, Int>
        if self.eat(TokenKind::Lt) {
            type_name.push('<');
            loop {
                type_name.push_str(&self.expect(TokenKind::Identifier)?.text);
                // Nested nullable: String?
                if self.eat(TokenKind::Question) {
                    type_name.push('?');
                }
                if !self.eat(TokenKind::Comma) {
                    break;
                }
                type_name.push_str(", ");
            }
            self.expect(TokenKind::Gt)?;
            type_name.push('>');
            // Nullable on the whole type: List<String>?
            if self.eat(TokenKind::Question) {
                type_name.push('?');
            }
        }

        Ok(type_name)
    }

    fn parse_expr_or_assignment(&mut self) -> Result<Stmt, ParseError> {
        let expr = self.parse_expression()?;

        // Check for simple assignment: target = value
        if self.eat(TokenKind::Eq) {
            let value = self.parse_expression()?;
            let span = merge_spans(expr_span(&expr), expr_span(&value));
            return Ok(Stmt::Expr(ExprStmt {
                span,
                expr: Box::new(Expr::Assignment(Assignment {
                    span,
                    target: Box::new(expr),
                    value: Box::new(value),
                })),
            }));
        }

        // Check for compound assignment: target op= value
        if let Some(op) = self.try_compound_assign() {
            let value = self.parse_expression()?;
            let span = merge_spans(expr_span(&expr), expr_span(&value));
            return Ok(Stmt::Expr(ExprStmt {
                span,
                expr: Box::new(Expr::Binary(BinaryExpr {
                    span,
                    left: Box::new(expr),
                    operator: op,
                    right: Box::new(value),
                })),
            }));
        }

        let span = expr_span(&expr);
        Ok(Stmt::Expr(ExprStmt {
            span,
            expr: Box::new(expr),
        }))
    }

    fn try_compound_assign(&mut self) -> Option<BinaryOp> {
        let op = match self.cur().kind {
            TokenKind::PlusEq => BinaryOp::AddAssign,
            TokenKind::MinusEq => BinaryOp::SubAssign,
            TokenKind::StarEq => BinaryOp::MulAssign,
            TokenKind::SlashEq => BinaryOp::DivAssign,
            TokenKind::PercentEq => BinaryOp::ModAssign,
            TokenKind::ElvisAssign => BinaryOp::ElvisAssign,
            _ => return None,
        };
        self.advance();
        Some(op)
    }

    // ── Expression Parsing ───────────────────────────────────────────────────

    fn parse_expression(&mut self) -> Result<Expr, ParseError> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Result<Expr, ParseError> {
        // The lexer has a standalone '?' token now, but ternary parsing still goes
        // through the elvis path. Full ternary support can be added here later.
        self.parse_elvis()
    }

    fn parse_elvis(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_range()?;
        if self.eat(TokenKind::Elvis) {
            let right = self.parse_range()?;
            let span = merge_spans(expr_span(&left), expr_span(&right));
            left = Expr::Elvis(ElvisExpr {
                span,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        Ok(left)
    }

    fn parse_range(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_binary(Prec::Lowest)?;
        if self.eat(TokenKind::DotDot) {
            let right = self.parse_binary(Prec::Range)?;
            let span = merge_spans(expr_span(&left), expr_span(&right));
            left = Expr::Binary(BinaryExpr {
                span,
                left: Box::new(left),
                operator: BinaryOp::Range,
                right: Box::new(right),
            });
        } else if self.eat(TokenKind::DotDotLt) {
            let right = self.parse_binary(Prec::Range)?;
            let span = merge_spans(expr_span(&left), expr_span(&right));
            left = Expr::Binary(BinaryExpr {
                span,
                left: Box::new(left),
                operator: BinaryOp::RangeExclusive,
                right: Box::new(right),
            });
        }
        Ok(left)
    }

    /// Binary expression parsing with precedence climbing (Pratt parsing).
    fn parse_binary(&mut self, min_prec: Prec) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;

        while let Some(info) = self.binary_op_info() {
            let (op, prec, right_assoc) = info;

            if prec < min_prec {
                break;
            }

            self.advance(); // consume the operator token

            let next_min = if right_assoc {
                prec
            } else {
                match prec as u8 + 1 {
                    0 => Prec::Lowest,
                    1 => Prec::Or,
                    2 => Prec::And,
                    3 => Prec::BitOr,
                    4 => Prec::BitXor,
                    5 => Prec::BitAnd,
                    6 => Prec::Equality,
                    7 => Prec::Comparison,
                    8 => Prec::Shift,
                    9 => Prec::Range,
                    10 => Prec::Additive,
                    11 => Prec::Multiplicative,
                    _ => Prec::Multiplicative,
                }
            };

            let right = self.parse_binary(next_min)?;
            let span = merge_spans(expr_span(&left), expr_span(&right));
            left = Expr::Binary(BinaryExpr {
                span,
                left: Box::new(left),
                operator: op,
                right: Box::new(right),
            });
        }

        Ok(left)
    }

    /// Determine if the current token is a binary operator and its precedence.
    fn binary_op_info(&self) -> Option<(BinaryOp, Prec, bool)> {
        let (op, prec, right_assoc) = match self.cur().kind {
            // Logical OR (lowest binary precedence)
            TokenKind::PipePipe => (BinaryOp::Or, Prec::Or, false),
            // Logical AND
            TokenKind::AmpAmp => (BinaryOp::And, Prec::And, false),
            // Bitwise OR
            TokenKind::Pipe => (BinaryOp::BitOr, Prec::BitOr, false),
            // Bitwise XOR
            TokenKind::Caret => (BinaryOp::BitXor, Prec::BitXor, false),
            // Bitwise AND
            TokenKind::Amp => (BinaryOp::BitAnd, Prec::BitAnd, false),
            // Equality
            TokenKind::EqEq => (BinaryOp::Eq, Prec::Equality, false),
            TokenKind::BangEq => (BinaryOp::Ne, Prec::Equality, false),
            TokenKind::EqEqEq => (BinaryOp::RefEq, Prec::Equality, false),
            TokenKind::BangEqEq => (BinaryOp::RefNe, Prec::Equality, false),
            TokenKind::Spaceship => (BinaryOp::Spaceship, Prec::Equality, false),
            // Comparison
            TokenKind::Lt => (BinaryOp::Lt, Prec::Comparison, false),
            TokenKind::Gt => (BinaryOp::Gt, Prec::Comparison, false),
            TokenKind::LtEq => (BinaryOp::Le, Prec::Comparison, false),
            TokenKind::GtEq => (BinaryOp::Ge, Prec::Comparison, false),
            // Shift
            TokenKind::LtLt => (BinaryOp::Shl, Prec::Shift, false),
            TokenKind::GtGt => (BinaryOp::Shr, Prec::Shift, false),
            TokenKind::GtGtGt => (BinaryOp::Ushr, Prec::Shift, false),
            // Additive
            TokenKind::Plus => (BinaryOp::Add, Prec::Additive, false),
            TokenKind::Minus => (BinaryOp::Sub, Prec::Additive, false),
            // Multiplicative
            TokenKind::Star => (BinaryOp::Mul, Prec::Multiplicative, false),
            TokenKind::Slash => (BinaryOp::Div, Prec::Multiplicative, false),
            TokenKind::Percent => (BinaryOp::Mod, Prec::Multiplicative, false),
            // Groovy regex
            // Note: =~ and ==~ are not separate tokens in the current lexer,
            // so we handle them as potential future extensions.
            _ => return None,
        };
        Some((op, prec, right_assoc))
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        let start_span = to_ast_span(&self.cur().span);

        match self.cur().kind {
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_unary()?;
                let span = merge_spans(start_span, expr_span(&operand));
                Ok(Expr::Unary(UnaryExpr {
                    span,
                    operator: UnaryOp::Neg,
                    operand: Box::new(operand),
                    is_postfix: false,
                }))
            }
            TokenKind::Bang => {
                self.advance();
                let operand = self.parse_unary()?;
                let span = merge_spans(start_span, expr_span(&operand));
                Ok(Expr::Unary(UnaryExpr {
                    span,
                    operator: UnaryOp::Not,
                    operand: Box::new(operand),
                    is_postfix: false,
                }))
            }
            TokenKind::Tilde => {
                self.advance();
                let operand = self.parse_unary()?;
                let span = merge_spans(start_span, expr_span(&operand));
                Ok(Expr::Unary(UnaryExpr {
                    span,
                    operator: UnaryOp::BitNot,
                    operand: Box::new(operand),
                    is_postfix: false,
                }))
            }
            TokenKind::Plus => {
                // Unary plus (just skip it)
                self.advance();
                self.parse_unary()
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;

        loop {
            match self.cur().kind {
                // Property access: expr.property
                TokenKind::Dot => {
                    self.advance(); // consume .
                    if self.at(TokenKind::Identifier) || self.is_keyword_as_property() {
                        let name = self.cur().text.clone();
                        let prop_span = to_ast_span(&self.cur().span);
                        self.advance();

                        // Check for method call: expr.method(args)
                        if self.at(TokenKind::LParen) {
                            let (args, trailing_closure) = self.parse_call_args()?;
                            let span = merge_spans(
                                expr_span(&expr),
                                trailing_closure
                                    .as_ref()
                                    .map(|c| c.span)
                                    .unwrap_or_else(|| to_ast_span(&self.peek(0).span)),
                            );
                            expr = Expr::MethodCall(MethodCall {
                                span,
                                receiver: Some(Box::new(expr)),
                                name,
                                arguments: args,
                                trailing_closure: trailing_closure.map(Box::new),
                            });
                        } else {
                            let span = merge_spans(expr_span(&expr), prop_span);
                            expr = Expr::PropertyAccess(PropertyAccess {
                                span,
                                object_expr: Box::new(expr),
                                property: name,
                            });
                        }
                    } else if self.at(TokenKind::Star) {
                        // Spread: expr.*
                        let star_span = to_ast_span(&self.cur().span);
                        self.advance();
                        expr = Expr::Spread(SpreadOperator {
                            span: merge_spans(expr_span(&expr), star_span),
                            expr: Box::new(expr),
                        });
                    } else {
                        self.error_with_expected(
                            "expected identifier after '.'",
                            vec!["identifier".into()],
                        );
                        break;
                    }
                }

                // Safe navigation: expr?.property or expr?.method(args)
                TokenKind::QuestionDot | TokenKind::SafeNav => {
                    self.advance(); // consume ?.
                    if self.at(TokenKind::Identifier) || self.is_keyword_as_property() {
                        let name = self.cur().text.clone();
                        let member_span = to_ast_span(&self.cur().span);
                        self.advance();

                        if self.at(TokenKind::LParen) {
                            let (args, trailing_closure) = self.parse_call_args()?;
                            let span = merge_spans(
                                expr_span(&expr),
                                trailing_closure
                                    .as_ref()
                                    .map(|c| c.span)
                                    .unwrap_or_else(|| to_ast_span(&self.peek(0).span)),
                            );
                            // Safe method call - wrap in SafeNavigation with args
                            expr = Expr::SafeNavigation(SafeNavigation {
                                span,
                                object_expr: Box::new(expr),
                                member: name,
                                arguments: Some(args),
                            });
                        } else {
                            let span = merge_spans(expr_span(&expr), member_span);
                            expr = Expr::SafeNavigation(SafeNavigation {
                                span,
                                object_expr: Box::new(expr),
                                member: name,
                                arguments: None,
                            });
                        }
                    } else {
                        self.error_with_expected(
                            "expected identifier after '?.'",
                            vec!["identifier".into()],
                        );
                        break;
                    }
                }

                // Index access: expr[index]
                TokenKind::LBracket => {
                    self.advance(); // consume [
                    let index = self.parse_expression()?;
                    self.expect(TokenKind::RBracket)?;
                    let span = merge_spans(expr_span(&expr), expr_span(&index));
                    expr = Expr::IndexAccess(IndexAccess {
                        span,
                        object_expr: Box::new(expr),
                        index: Box::new(index),
                    });
                }

                // Method call with parens: expr(args) — when expr is a bare identifier
                TokenKind::LParen if matches!(expr, Expr::Identifier(_)) => {
                    let name = match &expr {
                        Expr::Identifier(id) => id.name.clone(),
                        _ => unreachable!(),
                    };
                    let (args, trailing_closure) = self.parse_call_args()?;
                    let span = merge_spans(
                        expr_span(&expr),
                        trailing_closure
                            .as_ref()
                            .map(|c| c.span)
                            .unwrap_or_else(|| to_ast_span(&self.peek(0).span)),
                    );
                    expr = Expr::MethodCall(MethodCall {
                        span,
                        receiver: None,
                        name,
                        arguments: args,
                        trailing_closure: trailing_closure.map(Box::new),
                    });
                }

                // Trailing closure without parens: method { ... }
                TokenKind::LBrace if is_callable_expr(&expr) => {
                    let closure = self.parse_closure()?;
                    let span = merge_spans(expr_span(&expr), closure.span);
                    match &mut expr {
                        Expr::MethodCall(mc) => {
                            mc.trailing_closure = Some(Box::new(closure));
                            mc.span = span;
                        }
                        Expr::Identifier(id) => {
                            let name = id.name.clone();
                            expr = Expr::MethodCall(MethodCall {
                                span,
                                receiver: None,
                                name,
                                arguments: Vec::new(),
                                trailing_closure: Some(Box::new(closure)),
                            });
                        }
                        Expr::PropertyAccess(pa) => {
                            let receiver = std::mem::replace(
                                &mut *pa.object_expr,
                                Expr::Null(NullLiteral {
                                    span: Span::unknown(),
                                }),
                            );
                            let name = pa.property.clone();
                            expr = Expr::MethodCall(MethodCall {
                                span,
                                receiver: Some(Box::new(receiver)),
                                name,
                                arguments: Vec::new(),
                                trailing_closure: Some(Box::new(closure)),
                            });
                        }
                        _ => break,
                    }
                }

                _ => break,
            }
        }

        Ok(expr)
    }

    // ── Primary Expression Parsing ───────────────────────────────────────────

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let start_span = to_ast_span(&self.cur().span);

        match self.cur().kind {
            // ── String literals ──
            TokenKind::StringLit
            | TokenKind::GStringLit
            | TokenKind::TripleStringLit
            | TokenKind::TripleGStringLit => self.parse_string_literal(),

            // ── Number literals ──
            TokenKind::IntLit | TokenKind::LongLit | TokenKind::FloatLit | TokenKind::DoubleLit => {
                self.parse_number_literal()
            }

            // ── Boolean / null literals ──
            TokenKind::True => {
                let span = to_ast_span(&self.cur().span);
                self.advance();
                Ok(Expr::Boolean(BooleanLiteral { span, value: true }))
            }
            TokenKind::False => {
                let span = to_ast_span(&self.cur().span);
                self.advance();
                Ok(Expr::Boolean(BooleanLiteral { span, value: false }))
            }
            TokenKind::Null => {
                let span = to_ast_span(&self.cur().span);
                self.advance();
                Ok(Expr::Null(NullLiteral { span }))
            }
            TokenKind::This => {
                let span = to_ast_span(&self.cur().span);
                self.advance();
                Ok(Expr::This(ThisExpr { span }))
            }

            // ── Identifier (possibly a method call) ──
            TokenKind::Identifier => {
                let tok = self.advance();
                let name = tok.text.clone();
                let id_span = to_ast_span(&tok.span);

                // Kotlin DSL type-safe accessor: the<JavaCompile> { ... }
                if name == "the" && self.at(TokenKind::Lt) {
                    self.advance(); // consume <
                    let type_name = self.parse_type_reference()?;
                    self.expect(TokenKind::Gt)?; // consume >
                    let configuration = self.parse_closure()?;
                    let end_span = configuration.span;
                    return Ok(Expr::TypeSafeAccessor(TypeSafeAccessor {
                        span: merge_spans(id_span, end_span),
                        type_name,
                        configuration: Box::new(Expr::Closure(configuration)),
                    }));
                }

                // Method call with parens: foo(args)
                if self.at(TokenKind::LParen) {
                    let (args, trailing_closure) = self.parse_call_args()?;
                    let span = merge_spans(
                        id_span,
                        trailing_closure
                            .as_ref()
                            .map(|c| c.span)
                            .unwrap_or_else(|| to_ast_span(&self.peek(0).span)),
                    );
                    return Ok(Expr::MethodCall(MethodCall {
                        span,
                        receiver: None,
                        name,
                        arguments: args,
                        trailing_closure: trailing_closure.map(Box::new),
                    }));
                }

                // Method call without parens (Groovy idiom): compileSdk 34
                // Heuristic: if the next token can start an expression and is not
                // a statement-ending token or operator, treat as method args.
                // Exception: if next token is '[', let parse_postfix handle index access.
                if self.can_be_no_paren_arg() && !self.at(TokenKind::LBracket) {
                    let mut args = Vec::new();
                    while self.can_be_no_paren_arg() {
                        args.push(Arg::Positional {
                            expr: Box::new(self.parse_expression()?),
                        });
                    }

                    // Trailing closure
                    let trailing_closure = if self.at(TokenKind::LBrace) {
                        Some(self.parse_closure()?)
                    } else {
                        None
                    };

                    let end_span = trailing_closure
                        .as_ref()
                        .map(|c| c.span)
                        .or_else(|| args.last().map(|a| expr_span(&a.expr_ref())))
                        .unwrap_or(id_span);

                    return Ok(Expr::MethodCall(MethodCall {
                        span: merge_spans(id_span, end_span),
                        receiver: None,
                        name,
                        arguments: args,
                        trailing_closure: trailing_closure.map(Box::new),
                    }));
                }

                // Named arguments without parens: method key: value, key2: value2
                if self.at(TokenKind::Identifier) && self.peek(1).kind == TokenKind::Colon {
                    let named_args = self.parse_named_arg_list()?;
                    let trailing_closure = if self.at(TokenKind::LBrace) {
                        Some(self.parse_closure()?)
                    } else {
                        None
                    };
                    let end_span = trailing_closure
                        .as_ref()
                        .map(|c| c.span)
                        .or_else(|| named_args.last().map(|na| na.span))
                        .unwrap_or(id_span);
                    return Ok(Expr::MethodCall(MethodCall {
                        span: merge_spans(id_span, end_span),
                        receiver: None,
                        name,
                        arguments: named_args.into_iter().map(Arg::Named).collect(),
                        trailing_closure: trailing_closure.map(Box::new),
                    }));
                }

                // Trailing closure on bare identifier: method { ... }
                if self.at(TokenKind::LBrace) {
                    let closure = self.parse_closure()?;
                    return Ok(Expr::MethodCall(MethodCall {
                        span: merge_spans(id_span, closure.span),
                        receiver: None,
                        name,
                        arguments: Vec::new(),
                        trailing_closure: Some(Box::new(closure)),
                    }));
                }

                Ok(Expr::Identifier(Identifier {
                    span: id_span,
                    name,
                }))
            }

            // ── Keywords that can act as identifiers in Groovy DSL ──
            TokenKind::New
            | TokenKind::Class
            | TokenKind::Return
            | TokenKind::If
            | TokenKind::Else
            | TokenKind::For
            | TokenKind::While
            | TokenKind::Abstract
            | TokenKind::Private
            | TokenKind::Protected
            | TokenKind::Public
            | TokenKind::Void
            | TokenKind::Package
            | TokenKind::Instanceof => {
                // In Groovy DSL, many keywords are used as method names
                let tok = self.advance();
                let name = tok.text.clone();
                let id_span = to_ast_span(&tok.span);

                if self.at(TokenKind::LParen) {
                    let (args, trailing_closure) = self.parse_call_args()?;
                    let span = merge_spans(
                        id_span,
                        trailing_closure
                            .as_ref()
                            .map(|c| c.span)
                            .unwrap_or_else(|| to_ast_span(&self.peek(0).span)),
                    );
                    Ok(Expr::MethodCall(MethodCall {
                        span,
                        receiver: None,
                        name,
                        arguments: args,
                        trailing_closure: trailing_closure.map(Box::new),
                    }))
                } else if self.at(TokenKind::LBrace) {
                    let closure = self.parse_closure()?;
                    Ok(Expr::MethodCall(MethodCall {
                        span: merge_spans(id_span, closure.span),
                        receiver: None,
                        name,
                        arguments: Vec::new(),
                        trailing_closure: Some(Box::new(closure)),
                    }))
                } else if self.can_be_no_paren_arg() && !self.at(TokenKind::LBracket) {
                    let mut args = Vec::new();
                    while self.can_be_no_paren_arg() {
                        args.push(Arg::Positional {
                            expr: Box::new(self.parse_expression()?),
                        });
                    }
                    let end_span = args
                        .last()
                        .map(|a| expr_span(&a.expr_ref()))
                        .unwrap_or(id_span);
                    Ok(Expr::MethodCall(MethodCall {
                        span: merge_spans(id_span, end_span),
                        receiver: None,
                        name,
                        arguments: args,
                        trailing_closure: None,
                    }))
                } else {
                    Ok(Expr::Identifier(Identifier {
                        span: id_span,
                        name,
                    }))
                }
            }

            // ── Parenthesized expression ──
            TokenKind::LParen => {
                self.advance(); // consume (
                let inner = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                let span = merge_spans(start_span, expr_span(&inner));
                Ok(Expr::Paren(ParenExpr {
                    span,
                    expr: Box::new(inner),
                }))
            }

            // ── List or Map literal ──
            TokenKind::LBracket => self.parse_list_or_map(),

            // ── Closure ──
            TokenKind::LBrace => {
                let closure = self.parse_closure()?;
                Ok(Expr::Closure(closure))
            }

            _ => Err(ParseError {
                message: "unexpected token in expression".into(),
                expected: vec![
                    "expression".into(),
                    "string literal".into(),
                    "number".into(),
                    "identifier".into(),
                    "'('".into(),
                    "'['".into(),
                    "'{'".into(),
                ],
                found: Some(format!("{} ({:?})", self.cur().text, self.cur().kind)),
                span: to_ast_span(&self.cur().span),
            }),
        }
    }

    // ── String Literal Parsing ───────────────────────────────────────────────

    fn parse_string_literal(&mut self) -> Result<Expr, ParseError> {
        let tok = self.advance();
        let span = to_ast_span(&tok.span);

        let quote = match tok.kind {
            TokenKind::StringLit => QuoteStyle::Single,
            TokenKind::GStringLit => QuoteStyle::Double,
            TokenKind::TripleStringLit => QuoteStyle::TripleSingle,
            TokenKind::TripleGStringLit => QuoteStyle::TripleDouble,
            _ => QuoteStyle::Double,
        };

        // For GString literals, we parse the text to extract interpolation parts.
        // The lexer emits the raw text including ${} markers.
        let parts = if matches!(
            tok.kind,
            TokenKind::GStringLit | TokenKind::TripleGStringLit
        ) {
            parse_gstring_parts(&tok.text, span)
        } else {
            // Plain string - the lexer gives us the raw text (without quotes)
            vec![StringPart::Literal {
                text: tok.text.clone(),
            }]
        };

        Ok(Expr::String(StringLiteral { span, quote, parts }))
    }

    // ── Number Literal Parsing ───────────────────────────────────────────────

    fn parse_number_literal(&mut self) -> Result<Expr, ParseError> {
        let tok = self.advance();
        let span = to_ast_span(&tok.span);
        let kind = match tok.kind {
            TokenKind::IntLit => NumberKind::Integer,
            TokenKind::LongLit => NumberKind::Long,
            TokenKind::FloatLit => NumberKind::Float,
            TokenKind::DoubleLit => NumberKind::Double,
            _ => NumberKind::Integer,
        };
        Ok(Expr::Number(NumberLiteral {
            span,
            raw: tok.text.clone(),
            kind,
        }))
    }

    // ── List / Map Literal Parsing ───────────────────────────────────────────

    fn parse_list_or_map(&mut self) -> Result<Expr, ParseError> {
        let start_span = to_ast_span(&self.cur().span);
        self.advance(); // consume [

        // Empty: []
        if self.at(TokenKind::RBracket) {
            let span = to_ast_span(&self.cur().span);
            self.advance();
            return Ok(Expr::List(ListLiteral {
                span: merge_spans(start_span, span),
                elements: Vec::new(),
            }));
        }

        // Lookahead: parse first expression, then check for ':'
        let first = self.parse_expression()?;

        if self.eat(TokenKind::Colon) {
            // Map literal
            let mut entries = vec![MapEntry {
                span: merge_spans(expr_span(&first), to_ast_span(&self.cur().span)),
                key: Box::new(first),
                value: Box::new(self.parse_expression()?),
            }];

            while self.eat(TokenKind::Comma) {
                if self.at(TokenKind::RBracket) {
                    break;
                }
                let key = self.parse_expression()?;
                self.expect(TokenKind::Colon)?;
                let val = self.parse_expression()?;
                entries.push(MapEntry {
                    span: merge_spans(expr_span(&key), expr_span(&val)),
                    key: Box::new(key),
                    value: Box::new(val),
                });
            }

            let end_span = to_ast_span(&self.cur().span);
            self.expect(TokenKind::RBracket)?;
            Ok(Expr::Map(MapLiteral {
                span: merge_spans(start_span, end_span),
                entries,
            }))
        } else {
            // List literal
            let mut elements = vec![first];
            while self.eat(TokenKind::Comma) {
                if self.at(TokenKind::RBracket) {
                    break;
                }
                elements.push(self.parse_expression()?);
            }
            let end_span = to_ast_span(&self.cur().span);
            self.expect(TokenKind::RBracket)?;
            Ok(Expr::List(ListLiteral {
                span: merge_spans(start_span, end_span),
                elements,
            }))
        }
    }

    // ── Method Call Argument Parsing ─────────────────────────────────────────

    /// Parse `(arg1, arg2, ...)` argument list for method calls.
    fn parse_call_args(&mut self) -> Result<(Vec<Arg>, Option<Closure>), ParseError> {
        self.expect(TokenKind::LParen)?;

        let mut args = Vec::new();

        if self.at(TokenKind::RParen) {
            self.advance();
            return Ok((args, None));
        }

        loop {
            // Check for named argument: name: value (Groovy) or name = value (Kotlin)
            if self.at(TokenKind::Identifier)
                && (self.peek(1).kind == TokenKind::Colon || self.peek(1).kind == TokenKind::Eq)
            {
                let name = self.cur().text.clone();
                let arg_span = to_ast_span(&self.cur().span);
                self.advance(); // consume name
                self.advance(); // consume : or =
                let value = self.parse_expression()?;
                args.push(Arg::Named(NamedArgument {
                    span: merge_spans(arg_span, expr_span(&value)),
                    name,
                    value: Box::new(value),
                }));
            } else {
                let expr = self.parse_expression()?;
                args.push(Arg::Positional {
                    expr: Box::new(expr),
                });
            }

            if !self.eat(TokenKind::Comma) {
                break;
            }
            if self.at(TokenKind::RParen) {
                break;
            }
        }

        self.expect(TokenKind::RParen)?;

        // Check for trailing closure after the closing paren
        let trailing_closure = if self.at(TokenKind::LBrace) {
            Some(self.parse_closure()?)
        } else {
            None
        };

        Ok((args, trailing_closure))
    }

    // ── Named Argument List Parsing (no parens) ──────────────────────────────

    fn parse_named_arg_list(&mut self) -> Result<Vec<NamedArgument>, ParseError> {
        let mut args = Vec::new();
        while self.at(TokenKind::Identifier) && self.peek(1).kind == TokenKind::Colon {
            let name = self.cur().text.clone();
            let arg_span = to_ast_span(&self.cur().span);
            self.advance(); // name
            self.advance(); // :
            let value = self.parse_expression()?;
            args.push(NamedArgument {
                span: merge_spans(arg_span, expr_span(&value)),
                name,
                value: Box::new(value),
            });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        Ok(args)
    }

    // ── Closure Parsing ──────────────────────────────────────────────────────

    fn parse_closure(&mut self) -> Result<Closure, ParseError> {
        let start_span = to_ast_span(&self.cur().span);
        self.expect(TokenKind::LBrace)?;

        let mut params = Vec::new();
        let mut has_arrow = false;

        // Try to detect closure parameters: [param1, param2 ->] or [param ->]
        let saved = self.pos;
        if self.at(TokenKind::Identifier) {
            let mut maybe_params = Vec::new();
            loop {
                if self.at(TokenKind::Identifier) {
                    let name = self.cur().text.clone();
                    let pspan = to_ast_span(&self.cur().span);
                    self.advance();

                    maybe_params.push(ClosureParam {
                        span: pspan,
                        name,
                        type_annotation: None,
                        default_value: None,
                    });

                    if self.at(TokenKind::Arrow) {
                        self.advance();
                        has_arrow = true;
                        break;
                    } else if self.at(TokenKind::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            if has_arrow {
                params = maybe_params;
            } else {
                self.pos = saved;
            }
        }

        // Parse body statements
        let mut body = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at_end() {
            while self.eat(TokenKind::Semicolon) {}
            if self.at(TokenKind::RBrace) || self.at_end() {
                break;
            }

            match self.parse_statement() {
                Ok(stmt) => body.push(stmt),
                Err(e) => {
                    self.errors.push(e);
                    self.recover();
                }
            }
        }

        let end_span = to_ast_span(&self.cur().span);
        self.expect(TokenKind::RBrace)?;

        Ok(Closure {
            span: merge_spans(start_span, end_span),
            params,
            body,
        })
    }

    // ── Helper: Can the current token be a no-paren argument? ────────────────

    fn can_be_no_paren_arg(&self) -> bool {
        match self.cur().kind {
            TokenKind::StringLit
            | TokenKind::GStringLit
            | TokenKind::TripleStringLit
            | TokenKind::TripleGStringLit
            | TokenKind::IntLit
            | TokenKind::LongLit
            | TokenKind::FloatLit
            | TokenKind::DoubleLit
            | TokenKind::Identifier
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null
            | TokenKind::This
            | TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::LBrace => true,
            // Keywords used as method args in Groovy DSL
            TokenKind::New
            | TokenKind::Class
            | TokenKind::Return
            | TokenKind::If
            | TokenKind::For
            | TokenKind::While
            | TokenKind::Def
            | TokenKind::Val
            | TokenKind::Var => true,
            _ => false,
        }
    }

    /// Check if the current token is a keyword that can be used as a property name
    /// in Groovy DSL (e.g., `class`, `package`, `return`).
    fn is_keyword_as_property(&self) -> bool {
        matches!(
            self.cur().kind,
            TokenKind::New
                | TokenKind::Class
                | TokenKind::Return
                | TokenKind::If
                | TokenKind::Else
                | TokenKind::For
                | TokenKind::While
                | TokenKind::Abstract
                | TokenKind::Private
                | TokenKind::Protected
                | TokenKind::Public
                | TokenKind::Void
                | TokenKind::Package
                | TokenKind::Instanceof
                | TokenKind::Def
                | TokenKind::Val
                | TokenKind::Var
                | TokenKind::Import
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
                | TokenKind::This
                | TokenKind::Super
        )
    }
}

// ─── Precedence levels ───────────────────────────────────────────────────────

/// Operator precedence levels for binary expression parsing.
/// Higher values = tighter binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
enum Prec {
    Lowest = 0,
    Or = 1,
    And = 2,
    BitOr = 3,
    BitXor = 4,
    BitAnd = 5,
    Equality = 6,
    Comparison = 7,
    Shift = 8,
    Range = 9,
    Additive = 10,
    Multiplicative = 11,
}

// ─── Helper functions ────────────────────────────────────────────────────────

/// Get the span of an expression.
fn expr_span(expr: &Expr) -> Span {
    match expr {
        Expr::String(s) => s.span,
        Expr::Number(n) => n.span,
        Expr::Boolean(b) => b.span,
        Expr::Null(n) => n.span,
        Expr::List(l) => l.span,
        Expr::Map(m) => m.span,
        Expr::Closure(c) => c.span,
        Expr::Binary(b) => b.span,
        Expr::Unary(u) => u.span,
        Expr::Ternary(t) => t.span,
        Expr::Elvis(e) => e.span,
        Expr::Cast(c) => c.span,
        Expr::PropertyAccess(p) => p.span,
        Expr::SafeNavigation(s) => s.span,
        Expr::IndexAccess(i) => i.span,
        Expr::MethodCall(m) => m.span,
        Expr::Assignment(a) => a.span,
        Expr::Spread(s) => s.span,
        Expr::Identifier(i) => i.span,
        Expr::This(t) => t.span,
        Expr::Paren(p) => p.span,
        Expr::TypeSafeAccessor(t) => t.span,
    }
}

/// Check if an expression looks callable (can accept a trailing closure).
fn is_callable_expr(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Identifier(_) | Expr::MethodCall(_) | Expr::PropertyAccess(_)
    )
}

/// Extension trait to get the inner Expr from an Arg.
trait ArgExprRef {
    fn expr_ref(&self) -> Expr;
}

impl ArgExprRef for Arg {
    fn expr_ref(&self) -> Expr {
        match self {
            Arg::Positional { expr } => (**expr).clone(),
            Arg::Named(na) => Expr::Identifier(Identifier {
                span: na.span,
                name: na.name.clone(),
            }),
        }
    }
}

/// Parse GString interpolation markers into `StringPart` values.
///
/// The lexer emits GString text with `${...}` and `$ident` markers intact.
/// We parse these into `StringPart::Literal` and `StringPart::Interpolation`.
fn parse_gstring_parts(text: &str, base_span: Span) -> Vec<StringPart> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            if chars.peek() == Some(&'{') {
                chars.next(); // consume '{'
                if !literal.is_empty() {
                    parts.push(StringPart::Literal {
                        text: std::mem::take(&mut literal),
                    });
                }

                let mut interpolation = String::new();
                let mut brace_depth = 0u32;
                for ic in chars.by_ref() {
                    if ic == '{' {
                        brace_depth += 1;
                        interpolation.push(ic);
                    } else if ic == '}' {
                        if brace_depth == 0 {
                            break;
                        }
                        brace_depth -= 1;
                        interpolation.push(ic);
                    } else {
                        interpolation.push(ic);
                    }
                }

                let expr_text = interpolation.trim();
                if !expr_text.is_empty() {
                    parts.push(StringPart::Interpolation {
                        expr: Box::new(Expr::Identifier(Identifier::unnamed(expr_text))),
                        span: base_span,
                    });
                }
            } else if chars
                .peek()
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_')
            {
                // $identifier simple interpolation
                if !literal.is_empty() {
                    parts.push(StringPart::Literal {
                        text: std::mem::take(&mut literal),
                    });
                }
                let mut ident = String::new();
                while let Some(&nc) = chars.peek() {
                    if nc.is_ascii_alphanumeric() || nc == '_' {
                        ident.push(nc);
                        chars.next();
                    } else {
                        break;
                    }
                }
                parts.push(StringPart::Interpolation {
                    expr: Box::new(Expr::Identifier(Identifier::unnamed(ident))),
                    span: base_span,
                });
            } else {
                literal.push(c);
            }
        } else {
            literal.push(c);
        }
    }

    // Flush remaining text
    if !literal.is_empty() {
        parts.push(StringPart::Literal { text: literal });
    }

    if parts.is_empty() {
        parts.push(StringPart::Literal {
            text: String::new(),
        });
    }

    parts
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Parse a Gradle Groovy DSL source string into a `ParseResult`.
///
/// Runs on a larger-than-default stack to avoid stack overflow when parsing
/// very large build scripts (synthetic benchmarks or extremely large real
/// scripts). The recursive-descent parser can consume significant stack
/// depth on deeply nested or very long inputs.
pub fn parse(source: &str) -> ParseResult {
    // For small inputs, parse inline on the current stack to avoid thread
    // creation overhead. For large inputs (above ~10 KB), spawn a thread
    // with increased stack size to prevent stack overflow.
    const STACK_THRESHOLD: usize = 10 * 1024; // 10 KB

    if source.len() < STACK_THRESHOLD {
        let tokens = tokenize(source);
        let parser = Parser::new(tokens);
        parser.parse()
    } else {
        let source = source.to_string();
        let handle = std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024) // 8 MB stack
            .spawn(move || {
                let tokens = tokenize(&source);
                let parser = Parser::new(tokens);
                parser.parse()
            })
            .expect("failed to spawn parser thread");
        handle.join().expect("parser thread panicked")
    }
}

/// Parse a Gradle Groovy DSL source string, returning just the AST or the first error.
pub fn parse_or_error(source: &str) -> Result<Script, ParseError> {
    let result = parse(source);
    if let Some(err) = result.errors.into_iter().next() {
        Err(err)
    } else {
        Ok(result.script)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_script() {
        let result = parse("");
        assert!(result.errors.is_empty());
        assert!(result.script.statements.is_empty());
    }

    #[test]
    fn test_simple_identifier() {
        let result = parse("foo");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.script.statements.len(), 1);
    }

    #[test]
    fn test_method_call_with_parens() {
        let result = parse("println('hello')");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.script.statements.len(), 1);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => {
                assert!(matches!(&*expr_stmt.expr, Expr::MethodCall(mc) if mc.name == "println"));
            }
            _ => panic!("expected ExprStmt"),
        }
    }

    #[test]
    fn test_method_call_no_parens() {
        let result = parse("compileSdk 34");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.script.statements.len(), 1);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => match &*expr_stmt.expr {
                Expr::MethodCall(mc) => {
                    assert_eq!(mc.name, "compileSdk");
                    assert_eq!(mc.arguments.len(), 1);
                }
                other => panic!("expected MethodCall, got {:?}", other),
            },
            _ => panic!("expected ExprStmt"),
        }
    }

    #[test]
    fn test_string_literal() {
        let result = parse("'hello world'");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn test_double_quoted_string() {
        let result = parse("\"hello world\"");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn test_gstring_with_braced_interpolation_parts() {
        let result = parse("\"hello ${name}!\"");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);

        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => match &*expr_stmt.expr {
                Expr::String(s) => {
                    assert_eq!(s.parts.len(), 3);
                    match &s.parts[0] {
                        StringPart::Literal { text } => assert_eq!(text, "\"hello "),
                        other => panic!("expected first part literal, got {:?}", other),
                    }
                    match &s.parts[1] {
                        StringPart::Interpolation { expr, .. } => match &**expr {
                            Expr::Identifier(id) => assert_eq!(id.name, "name"),
                            other => panic!("expected interpolation identifier, got {:?}", other),
                        },
                        other => panic!("expected interpolation part, got {:?}", other),
                    }
                    match &s.parts[2] {
                        StringPart::Literal { text } => assert_eq!(text, "!\""),
                        other => panic!("expected last part literal, got {:?}", other),
                    }
                }
                other => panic!("expected string literal, got {:?}", other),
            },
            _ => panic!("expected expression statement"),
        }
    }

    #[test]
    fn test_gstring_with_identifier_interpolation_parts() {
        let result = parse("\"$version-release\"");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);

        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => match &*expr_stmt.expr {
                Expr::String(s) => {
                    assert_eq!(s.parts.len(), 3);
                    match &s.parts[0] {
                        StringPart::Literal { text } => assert_eq!(text, "\""),
                        other => panic!("expected opening literal quote, got {:?}", other),
                    }
                    match &s.parts[1] {
                        StringPart::Interpolation { expr, .. } => match &**expr {
                            Expr::Identifier(id) => assert_eq!(id.name, "version"),
                            other => panic!("expected interpolation identifier, got {:?}", other),
                        },
                        other => panic!("expected interpolation part, got {:?}", other),
                    }
                    match &s.parts[2] {
                        StringPart::Literal { text } => assert_eq!(text, "-release\""),
                        other => panic!("expected trailing literal part, got {:?}", other),
                    }
                }
                other => panic!("expected string literal, got {:?}", other),
            },
            _ => panic!("expected expression statement"),
        }
    }

    #[test]
    fn test_number_literals() {
        let result = parse("42 3.14 100");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.script.statements.len(), 3);
    }

    #[test]
    fn test_assignment() {
        let result = parse("version = '1.0.0'");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => {
                assert!(matches!(&*expr_stmt.expr, Expr::Assignment(_)));
            }
            _ => panic!("expected ExprStmt with Assignment"),
        }
    }

    #[test]
    fn test_compound_assignment() {
        let result = parse("count += 1");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => {
                assert!(matches!(&*expr_stmt.expr, Expr::Binary(_)));
            }
            _ => panic!("expected ExprStmt with Binary"),
        }
    }

    #[test]
    fn test_list_literal() {
        let result = parse("[1, 2, 3]");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => {
                assert!(matches!(&*expr_stmt.expr, Expr::List(_)));
            }
            _ => panic!("expected list literal"),
        }
    }

    #[test]
    fn test_map_literal() {
        let result = parse("[a: 1, b: 2]");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => {
                assert!(matches!(&*expr_stmt.expr, Expr::Map(_)));
            }
            _ => panic!("expected map literal"),
        }
    }

    #[test]
    fn test_closure() {
        let result = parse("{ x -> println(x) }");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn test_closure_no_params() {
        let result = parse("{ println('hello') }");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn test_method_with_trailing_closure() {
        let result = parse("dependencies { implementation 'foo' }");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.script.statements.len(), 1);
    }

    #[test]
    fn test_member_access() {
        let result = parse("android.defaultConfig");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => {
                assert!(matches!(&*expr_stmt.expr, Expr::PropertyAccess(_)));
            }
            _ => panic!("expected property access"),
        }
    }

    #[test]
    fn test_safe_navigation() {
        let result = parse("obj?.property");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => {
                assert!(matches!(&*expr_stmt.expr, Expr::SafeNavigation(_)));
            }
            _ => panic!("expected safe navigation"),
        }
    }

    #[test]
    fn test_elvis() {
        let result = parse("value ?: 'default'");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => {
                assert!(matches!(&*expr_stmt.expr, Expr::Elvis(_)));
            }
            _ => panic!("expected elvis"),
        }
    }

    #[test]
    fn test_range() {
        let result = parse("1..10");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => match &*expr_stmt.expr {
                Expr::Binary(b) => assert_eq!(b.operator, BinaryOp::Range),
                _ => panic!("expected binary with Range op"),
            },
            _ => panic!("expected ExprStmt"),
        }
    }

    #[test]
    fn test_exclusive_range() {
        let result = parse("1..<10");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => match &*expr_stmt.expr {
                Expr::Binary(b) => assert_eq!(b.operator, BinaryOp::RangeExclusive),
                _ => panic!("expected binary with RangeExclusive op"),
            },
            _ => panic!("expected ExprStmt"),
        }
    }

    #[test]
    fn test_binary_operators() {
        let result = parse("1 + 2 * 3");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn test_unary_operators() {
        let result = parse("!flag -value");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        // `!flag -value` is parsed as `(!flag) - value` (binary subtraction), not two statements
        assert_eq!(result.script.statements.len(), 1);
    }

    #[test]
    fn test_import_statement() {
        let result = parse("import java.util.List");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Import(imp) => assert_eq!(imp.path, "java.util.List"),
            _ => panic!("expected import"),
        }
    }

    #[test]
    fn test_import_static() {
        let result = parse("import static java.util.Collections.*");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Import(imp) => {
                assert!(imp.is_static);
                assert!(imp.is_wildcard);
            }
            _ => panic!("expected static import"),
        }
    }

    #[test]
    fn test_import_with_alias() {
        let result = parse("import java.util.List as JList");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Import(imp) => assert_eq!(imp.alias.as_deref(), Some("JList")),
            _ => panic!("expected import with alias"),
        }
    }

    #[test]
    fn test_named_arguments() {
        let result = parse("task(type: Copy, from: 'src', into: 'build')");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => match &*expr_stmt.expr {
                Expr::MethodCall(mc) => {
                    assert_eq!(mc.name, "task");
                    assert_eq!(mc.arguments.len(), 3);
                }
                _ => panic!("expected method call"),
            },
            _ => panic!("expected expression"),
        }
    }

    #[test]
    fn test_boolean_null_literals() {
        let result = parse("true false null");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.script.statements.len(), 3);
    }

    #[test]
    fn test_parenthesized() {
        let result = parse("(1 + 2) * 3");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn test_index_access() {
        let result = parse("list[0]");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => {
                assert!(matches!(&*expr_stmt.expr, Expr::IndexAccess(_)));
            }
            _ => panic!("expected index access"),
        }
    }

    #[test]
    fn test_var_decl_def() {
        let result = parse("def x = 42");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::VarDecl(vd) => {
                assert_eq!(vd.kind, VarKind::Def);
                assert_eq!(vd.name, "x");
            }
            _ => panic!("expected VarDecl"),
        }
    }

    #[test]
    fn test_line_comments() {
        let result = parse("foo // comment\nbar");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        // In Groovy DSL, `foo bar` is a method call: foo(bar) — line comments are transparent
        assert_eq!(result.script.statements.len(), 1);
    }

    #[test]
    fn test_block_comments() {
        let result = parse("foo /* comment */ bar");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        // In Groovy DSL, `foo bar` is a method call: foo(bar) — block comments are transparent
        assert_eq!(result.script.statements.len(), 1);
    }

    #[test]
    fn test_error_recovery() {
        let source = "foo(] bar 42";
        let result = parse(source);
        // Should recover and parse at least the 'bar 42' statement (bar(42) in Groovy DSL)
        assert!(result.script.statements.len() >= 1, "expected at least 1 statement, got {}", result.script.statements.len());
        assert!(!result.errors.is_empty(), "expected parse errors");
    }

    #[test]
    fn test_gradle_dependencies_block() {
        let source = r#"
dependencies {
    implementation 'com.example:lib:1.0'
    testImplementation 'junit:junit:4.13.2'
}
"#;
        let result = parse(source);
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.script.statements.len(), 1);
    }

    #[test]
    fn test_gradle_android_block() {
        let source = r#"
android {
    compileSdk 34
    defaultConfig {
        applicationId "com.example.app"
        minSdk 24
        targetSdk 34
        versionCode 1
        versionName "1.0"
    }
}
"#;
        let result = parse(source);
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn test_gradle_plugins_block() {
        let source = r#"
plugins {
    id 'java'
    id 'org.springframework.boot' version '3.2.0'
    id 'io.spring.dependency-management' version '1.1.4' apply false
}
"#;
        let result = parse(source);
        // `apply` and `version` are keywords used as method names in Groovy DSL
        assert!(
            result.errors.len() <= 3,
            "too many errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_realistic_build_gradle() {
        let source = r#"
plugins {
    id 'com.android.application'
}

android {
    namespace 'com.example.app'
    compileSdk 34

    defaultConfig {
        applicationId "com.example.app"
        minSdk 24
        targetSdk 34
        versionCode 1
        versionName "1.0"
    }

    buildTypes {
        release {
            minifyEnabled true
            proguardFiles getDefaultProguardFile('proguard-android-optimize.txt'), 'proguard-rules.pro'
        }
    }
}

dependencies {
    implementation 'androidx.core:core-ktx:1.12.0'
    implementation 'androidx.appcompat:appcompat:1.6.1'
    implementation project(':core')
}
"#;
        let result = parse(source);
        assert!(
            result.errors.len() <= 5,
            "too many errors: {:?}",
            result.errors
        );
        assert!(!result.script.statements.is_empty());
    }

    #[test]
    fn test_semicolons() {
        let result = parse("a; b; c;");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.script.statements.len(), 3);
    }

    #[test]
    fn test_method_call_with_named_args_and_closure() {
        let result = parse("task(type: Copy) { from 'src' }");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => match &*expr_stmt.expr {
                Expr::MethodCall(mc) => {
                    assert_eq!(mc.name, "task");
                    assert!(mc.trailing_closure.is_some());
                }
                _ => panic!("expected method call"),
            },
            _ => panic!("expected expression"),
        }
    }

    // ─── Kotlin DSL tests ─────────────────────────────────────────────────

    #[test]
    fn test_parse_kotlin_type_annotation() {
        let result = parse("val name: String = \"test\"");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.script.statements.len(), 1);
        match &result.script.statements[0] {
            Stmt::VarDecl(v) => {
                assert_eq!(v.name, "name");
                assert_eq!(v.type_annotation.as_deref(), Some("String"));
            }
            _ => panic!("expected VarDecl"),
        }
    }

    #[test]
    fn test_parse_kotlin_generic_type() {
        let result = parse("val list: List<String> = emptyList()");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::VarDecl(v) => {
                assert_eq!(v.name, "list");
                assert_eq!(v.type_annotation.as_deref(), Some("List<String>"));
            }
            _ => panic!("expected VarDecl"),
        }
    }

    #[test]
    fn test_parse_kotlin_nullable_type() {
        let result = parse("val name: String? = null");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::VarDecl(v) => {
                assert_eq!(v.name, "name");
                assert_eq!(v.type_annotation.as_deref(), Some("String?"));
            }
            _ => panic!("expected VarDecl"),
        }
    }

    #[test]
    fn test_parse_kotlin_map_type() {
        let result = parse("val map: Map<String, Int> = emptyMap()");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::VarDecl(v) => {
                assert_eq!(v.type_annotation.as_deref(), Some("Map<String, Int>"));
            }
            _ => panic!("expected VarDecl"),
        }
    }

    #[test]
    fn test_parse_type_safe_accessor() {
        let result = parse("the<JavaCompile> { sourceCompatibility = \"17\" }");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => match &*expr_stmt.expr {
                Expr::TypeSafeAccessor(tsa) => {
                    assert_eq!(tsa.type_name, "JavaCompile");
                }
                _ => panic!("expected TypeSafeAccessor"),
            },
            _ => panic!("expected Expr"),
        }
    }

    #[test]
    fn test_parse_elvis_assignment() {
        let result = parse("value ?= defaultValue");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::Expr(expr_stmt) => match &*expr_stmt.expr {
                Expr::Binary(b) => {
                    assert_eq!(b.operator, BinaryOp::ElvisAssign);
                }
                other => panic!("expected Binary with ElvisAssign, got: {:?}", other),
            },
            other => panic!("expected Expr, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_val_without_type_annotation() {
        let result = parse("val x = 42");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::VarDecl(v) => {
                assert_eq!(v.name, "x");
                assert!(v.type_annotation.is_none());
            }
            _ => panic!("expected VarDecl"),
        }
    }

    #[test]
    fn test_parse_var_with_mutation() {
        let result = parse("var count = 0");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.script.statements[0] {
            Stmt::VarDecl(v) => {
                assert_eq!(v.name, "count");
            }
            _ => panic!("expected VarDecl"),
        }
    }
}
