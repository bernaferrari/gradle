//! Abstract Syntax Tree definitions for Gradle Groovy/Kotlin DSL build scripts.
//!
//! This module defines the full AST node types needed to represent Gradle build
//! scripts written in either Groovy (`.gradle`) or Kotlin DSL (`.gradle.kts`).
//! All node types are serializable via `serde` for caching and RPC transport.
//!
//! The AST is designed to be a faithful representation of the *syntactic* structure
//! of a build script, not its semantics. Semantic analysis (resolving method calls,
//! type inference, etc.) is handled at a later stage that lowers this AST into the
//! domain types from [`crate::server::build_script_parser`] (e.g. `ParsedDependency`).
//!
//! # Design Decisions
//!
//! - **Span tracking**: Every node carries a `Span` for error reporting and diagnostics.
//! - **Boxed children**: Recursive node types use `Box` to keep enum sizes manageable.
//! - **Groovy/Kotlin duality**: Some constructs exist only in one DSL (e.g. GString
//!   interpolation is Groovy-specific in syntax, but Kotlin has string templates).
//!   The AST uses a single unified model where the constructs overlap, with
//!   discriminators where needed.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Span & Location
// ---------------------------------------------------------------------------

/// Source location of a syntactic element.
///
/// Offsets are byte offsets into the original source text. Lines and columns
/// are 1-based for human-friendly display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// Byte offset of the first character.
    pub start: usize,
    /// Byte offset one past the last character.
    pub end: usize,
    /// 1-based line number of `start`.
    pub line: u32,
    /// 1-based column number of `start`.
    pub column: u32,
}

impl Span {
    /// Create a new span.
    pub fn new(start: usize, end: usize, line: u32, column: u32) -> Self {
        Self {
            start,
            end,
            line,
            column,
        }
    }

    /// Create a placeholder span (used during partial parsing).
    pub fn unknown() -> Self {
        Self {
            start: 0,
            end: 0,
            line: 0,
            column: 0,
        }
    }

    /// Merge two spans into one that covers both.
    pub fn merge(&self, other: &Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            line: self.line.min(other.line),
            column: if self.start <= other.start {
                self.column
            } else {
                other.column
            },
        }
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether this span is empty.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

// ---------------------------------------------------------------------------
// Script (top-level)
// ---------------------------------------------------------------------------

/// The detected dialect of a build script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Dialect {
    /// A Groovy DSL build script (`.gradle`).
    Groovy,
    /// A Kotlin DSL build script (`.gradle.kts`).
    KotlinDsl,
}

/// The top-level AST node representing a complete Gradle build script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    /// Source span of the entire script.
    pub span: Span,
    /// Which dialect this script is written in.
    pub dialect: Dialect,
    /// Top-level statements in the script.
    pub statements: Vec<Stmt>,
    /// Comments extracted during parsing (preserved for round-trip fidelity).
    pub comments: Vec<Comment>,
}

/// A preserved comment from the source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub span: Span,
    /// The comment text including delimiters (`// ...` or `/* ... */`).
    pub text: String,
    /// `true` for block comments `/* ... */`, `false` for line comments `// ...`.
    pub is_block: bool,
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

/// Any top-level or block-level statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Stmt {
    /// An expression used as a statement (method calls, assignments, etc.).
    Expr(ExprStmt),

    /// A variable declaration: Groovy `def x = ...`, Kotlin `val x = ...` / `var x = ...`.
    VarDecl(VarDecl),

    /// An import statement: `import com.example.Foo`.
    Import(ImportStmt),

    /// A semicolon-separated list of statements treated as one.
    Block(Block),
}

/// An expression evaluated for its side effects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExprStmt {
    pub span: Span,
    pub expr: Box<Expr>,
}

/// A variable declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarDecl {
    pub span: Span,
    /// `def`, `val`, `var`, or explicit type name.
    pub kind: VarKind,
    /// The declared name.
    pub name: String,
    /// Kotlin `by` delegation target (e.g. `val implementation by deps`).
    pub delegate: Option<Box<Expr>>,
    /// The initializer expression, if present.
    pub initializer: Option<Box<Expr>>,
}

/// The kind of variable declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VarKind {
    /// Groovy `def`.
    Def,
    /// Kotlin `val` (immutable).
    Val,
    /// Kotlin `var` (mutable).
    Var,
    /// Explicit type: e.g. `String x = "hello"`.
    Typed { type_name: String },
}

/// An import statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportStmt {
    pub span: Span,
    /// The full import path, e.g. `com.example.Foo`.
    pub path: String,
    /// Whether this is a star import (`import pkg.*`).
    pub is_wildcard: bool,
    /// Whether this is a static import (`import static Math.sqrt`).
    pub is_static: bool,
    /// Optional alias: `import foo.Bar as Baz`.
    pub alias: Option<String>,
}

/// A block of statements enclosed in braces `{ ... }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub span: Span,
    pub statements: Vec<Stmt>,
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

/// The core expression type. Covers literals, operators, calls, access, and
/// all Gradle-relevant syntactic forms.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Expr {
    // -- Literals ----------------------------------------------------------
    /// A string literal, possibly with interpolation.
    /// - Groovy: `"Hello ${name}"` or `'plain'`
    /// - Kotlin: `"Hello $name"` or `"Hello ${expr}"` or `"plain"`
    String(StringLiteral),

    /// A numeric literal (integer or floating point).
    Number(NumberLiteral),

    /// A boolean literal: `true` or `false`.
    Boolean(BooleanLiteral),

    /// The `null` literal.
    Null(NullLiteral),

    // -- Collections -------------------------------------------------------
    /// A list literal: `[1, 2, 3]`.
    List(ListLiteral),

    /// A map literal: `[key: value, ...]` (Groovy) or `mapOf(...)` (Kotlin).
    Map(MapLiteral),

    // -- Lambdas / Closures ------------------------------------------------
    /// A closure (Groovy) or lambda (Kotlin).
    /// - Groovy: `{ param -> ... }` or `{ -> ... }`
    /// - Kotlin: `{ param -> ... }` or `{ (a, b) -> ... }`
    Closure(Closure),

    // -- Operators ---------------------------------------------------------
    /// A binary expression: `a + b`, `x == y`, `a && b`, etc.
    Binary(BinaryExpr),

    /// A unary expression: `-x`, `!flag`, `++i`, etc.
    Unary(UnaryExpr),

    /// A ternary expression: `condition ? thenExpr : elseExpr`.
    Ternary(TernaryExpr),

    /// The Elvis operator: `value ?: default`.
    Elvis(ElvisExpr),

    /// A type cast: `expr as Type` (Kotlin) or `(Type) expr` (Groovy).
    Cast(CastExpr),

    // -- Access ------------------------------------------------------------
    /// Property access: `obj.property`.
    PropertyAccess(PropertyAccess),

    /// Safe navigation: `obj?.property` (Groovy & Kotlin).
    SafeNavigation(SafeNavigation),

    /// Index access: `list[0]` or `map["key"]`.
    IndexAccess(IndexAccess),

    // -- Calls -------------------------------------------------------------
    /// A method or function call: `foo()`, `bar(arg1, arg2)`.
    MethodCall(MethodCall),

    // -- Assignment --------------------------------------------------------
    /// An assignment: `target = value`.
    Assignment(Assignment),

    // -- Spread / Star -----------------------------------------------------
    /// The spread operator: `*list` in argument lists or `*[1, 2, 3]`.
    Spread(SpreadOperator),

    // -- Misc --------------------------------------------------------------
    /// A plain identifier: `foo`, `VERSION_17`.
    Identifier(Identifier),

    /// A `this` reference.
    This(ThisExpr),

    /// A parenthesized expression: `(expr)`.
    Paren(ParenExpr),
}

// ---------------------------------------------------------------------------
// Literal Types
// ---------------------------------------------------------------------------

/// A string literal with optional interpolation parts.
///
/// Supports both Groovy GString interpolation (`"Hello ${name}"`) and Kotlin
/// string templates (`"Hello $name"` / `"Hello ${expr}"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringLiteral {
    pub span: Span,
    /// The quote style used.
    pub quote: QuoteStyle,
    /// Ordered parts: alternating between literal text and interpolation expressions.
    /// Always starts and ends with literal text (which may be empty).
    pub parts: Vec<StringPart>,
}

/// The quoting style of a string literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuoteStyle {
    /// Double quotes: `"..."`. Supports interpolation in both Groovy and Kotlin.
    Double,
    /// Single quotes: `'...'`. No interpolation (plain string).
    Single,
    /// Triple double quotes: `"""..."""`. Multi-line, supports interpolation.
    TripleDouble,
    /// Triple single quotes: `'''...'''`. Groovy multi-line, no interpolation.
    TripleSingle,
    /// Dollar-slashy: `$/.../$`. Groovy slashy string with interpolation.
    DollarSlashy,
    /// Slashy: `/.../`. Groovy regex/pattern literal.
    Slashy,
}

/// A segment of a string literal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StringPart {
    /// A literal text segment.
    Literal { text: String },
    /// An interpolated expression.
    Interpolation { expr: Box<Expr>, span: Span },
}

/// A numeric literal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberLiteral {
    pub span: Span,
    /// The raw source text of the number, e.g. `"42"`, `"3.14"`, `"0xFF"`, `"1_000"`.
    pub raw: String,
    /// The numeric kind inferred from the suffix or format.
    pub kind: NumberKind,
}

/// Kind of numeric literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NumberKind {
    /// Plain integer: `42`
    Integer,
    /// Long integer: `42L`
    Long,
    /// Floating point: `3.14`
    Float,
    /// Double: `3.14d` or `3.14D`
    Double,
    /// BigInteger: `42G` or `42g` (Groovy)
    BigInteger,
    /// BigDecimal: `3.14G` or `3.14g` (Groovy)
    BigDecimal,
    /// Hex: `0xFF`
    Hex,
    /// Octal: `0o77` or `077`
    Octal,
    /// Binary: `0b1010`
    Binary,
}

/// A boolean literal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BooleanLiteral {
    pub span: Span,
    pub value: bool,
}

/// The null literal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NullLiteral {
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Collection Types
// ---------------------------------------------------------------------------

/// A list literal: `[1, 2, 3]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListLiteral {
    pub span: Span,
    /// The list elements. May contain `Expr::Spread` nodes.
    pub elements: Vec<Expr>,
}

/// A map literal: `[key: value, ...]` (Groovy) or `mapOf(key to value, ...)` (Kotlin).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapLiteral {
    pub span: Span,
    /// The map entries.
    pub entries: Vec<MapEntry>,
}

/// A single entry in a map literal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapEntry {
    pub span: Span,
    /// The key expression. For plain Groovy keys (unquoted identifiers), this is
    /// an `Expr::Identifier`.
    pub key: Box<Expr>,
    /// The value expression.
    pub value: Box<Expr>,
}

// ---------------------------------------------------------------------------
// Closure / Lambda
// ---------------------------------------------------------------------------

/// A closure (Groovy) or lambda (Kotlin).
///
/// Examples:
/// - Groovy: `{ x -> println(x) }`, `{ -> println("no params") }`, `{ println(it) }`
/// - Kotlin: `{ x -> println(x) }`, `{ (a: Int, b: Int) -> a + b }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Closure {
    pub span: Span,
    /// Declared parameters. Empty if the closure uses the implicit `it` parameter.
    pub params: Vec<ClosureParam>,
    /// The body of the closure.
    pub body: Vec<Stmt>,
}

/// A parameter declaration in a closure or lambda.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosureParam {
    pub span: Span,
    /// The parameter name.
    pub name: String,
    /// An optional explicit type annotation.
    pub type_annotation: Option<String>,
    /// An optional default value expression.
    pub default_value: Option<Box<Expr>>,
}

// ---------------------------------------------------------------------------
// Operator Expressions
// ---------------------------------------------------------------------------

/// A binary expression with an operator and two operands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryExpr {
    pub span: Span,
    pub left: Box<Expr>,
    pub operator: BinaryOp,
    pub right: Box<Expr>,
}

/// Binary operators, covering arithmetic, comparison, logical, equality,
/// assignment, and Groovy/Kotlin-specific operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BinaryOp {
    // Arithmetic
    Add, // +
    Sub, // -
    Mul, // *
    Div, // /
    Mod, // %
    Pow, // ** (Groovy)

    // Comparison
    Lt, // <
    Gt, // >
    Le, // <=
    Ge, // >=

    // Equality
    Eq,    // == (or `.equals()` in Groovy)
    Ne,    // != (or `!=` in Groovy)
    RefEq, // === (Groovy identity)
    RefNe, // !== (Groovy identity)

    // Logical
    And, // &&
    Or,  // ||

    // Bitwise
    BitAnd, // &
    BitOr,  // |
    BitXor, // ^
    Shl,    // <<
    Shr,    // >>
    Ushr,   // >>> (Groovy unsigned right shift)

    // String / Range
    LeftShift,      // << (also string append in Groovy)
    Range,          // .. (Groovy inclusive range)
    RangeExclusive, // ..< (Groovy/Kotlin exclusive range)

    // Assignment operators
    Assign,    // = (handled via `Expr::Assignment`, but kept for completeness)
    AddAssign, // +=
    SubAssign, // -=
    MulAssign, // *=
    DivAssign, // /=
    ModAssign, // %=

    // Kotlin-specific
    ElvisAssign, // ?= (Kotlin 1.9+)
    In,          // in
    NotIn,       // !in
    Is,          // is
    IsNot,       // !is

    // Groovy-specific
    Spaceship,     // <=> (Groovy comparison)
    RegexFind,     // =~ (Groovy regex find)
    RegexMatch,    // ==~ (Groovy regex match)
    Arrow,         // -> (used in map literals: `key -> value`)
    MemberPointer, // .& (Groovy method pointer)
}

/// A unary expression with a prefix or postfix operator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnaryExpr {
    pub span: Span,
    pub operator: UnaryOp,
    pub operand: Box<Expr>,
    /// Whether the operator is postfix (e.g. `x++`) vs prefix (e.g. `++x`).
    pub is_postfix: bool,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnaryOp {
    Neg,         // - (unary minus)
    Pos,         // + (unary plus)
    Not,         // ! (logical not)
    BitNot,      // ~ (bitwise not)
    PreInc,      // ++x
    PreDec,      // --x
    PostInc,     // x++
    PostDec,     // x--
    Dereference, // * (pointer dereference, rarely used in Gradle scripts)
}

/// A ternary conditional expression: `condition ? thenExpr : elseExpr`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TernaryExpr {
    pub span: Span,
    pub condition: Box<Expr>,
    pub then_expr: Box<Expr>,
    pub else_expr: Box<Expr>,
}

/// The Elvis operator: `value ?: default`.
/// Returns the left side if non-null, otherwise the right side.
/// Common in both Groovy and Kotlin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElvisExpr {
    pub span: Span,
    pub left: Box<Expr>,
    pub right: Box<Expr>,
}

/// A type cast expression.
///
/// - Kotlin: `expr as Type` or `expr as? Type` (safe cast).
/// - Groovy: `(Type) expr`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastExpr {
    pub span: Span,
    pub expr: Box<Expr>,
    /// The target type name.
    pub target_type: String,
    /// Whether this is a safe cast (`as?` in Kotlin).
    pub is_safe: bool,
}

// ---------------------------------------------------------------------------
// Access Expressions
// ---------------------------------------------------------------------------

/// Property access: `obj.property`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyAccess {
    pub span: Span,
    /// The object whose property is being accessed.
    pub object_expr: Box<Expr>,
    /// The property name.
    pub property: String,
}

/// Safe navigation: `obj?.property` or `obj?.method()`.
///
/// In Groovy, `?.` returns `null` instead of throwing NPE if the receiver is null.
/// In Kotlin, the same semantics apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeNavigation {
    pub span: Span,
    /// The object being safely accessed.
    pub object_expr: Box<Expr>,
    /// The property or method name being accessed.
    pub member: String,
    /// Optional arguments if this is a safe method call: `obj?.method(args)`.
    pub arguments: Option<Vec<Arg>>,
}

/// Index access: `list[0]` or `map["key"]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexAccess {
    pub span: Span,
    pub object_expr: Box<Expr>,
    pub index: Box<Expr>,
}

// ---------------------------------------------------------------------------
// Method Call
// ---------------------------------------------------------------------------

/// A method or function call.
///
/// This is the most common expression in Gradle scripts. Covers:
/// - Top-level calls: `plugins { }`, `dependencies { }`, `repositories { }`
/// - Configuration calls: `implementation("...")`, `testImplementation("...")`
/// - Task calls: `tasks.register("...") { }`, `task("...") { }`
/// - Utility calls: `project(":core")`, `file("...")`, `property("...")`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodCall {
    pub span: Span,
    /// The receiver of the method call. `None` for top-level/unqualified calls.
    pub receiver: Option<Box<Expr>>,
    /// The method name.
    pub name: String,
    /// The arguments passed to the method.
    pub arguments: Vec<Arg>,
    /// An optional trailing closure (lambda) argument.
    /// In Groovy: `method("arg") { ... }`
    /// In Kotlin: `method("arg") { ... }`
    pub trailing_closure: Option<Box<Closure>>,
}

/// A single argument in a method call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Arg {
    /// A positional argument: `implementation("com.example:lib:1.0")`.
    Positional { expr: Box<Expr> },
    /// A named argument: `id(id = "java", version = "1.0")` (Kotlin).
    Named(NamedArgument),
}

/// A named (keyword) argument.
///
/// - Kotlin: `name = value` or `name: value` (deprecated)
/// - Groovy: `key: value`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedArgument {
    pub span: Span,
    /// The argument name.
    pub name: String,
    /// The argument value.
    pub value: Box<Expr>,
}

// ---------------------------------------------------------------------------
// Assignment
// ---------------------------------------------------------------------------

/// An assignment expression: `target = value`.
///
/// The target can be a simple identifier, a property access, or an index access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assignment {
    pub span: Span,
    /// The left-hand side (assignable target).
    pub target: Box<Expr>,
    /// The right-hand side (value expression).
    pub value: Box<Expr>,
}

// ---------------------------------------------------------------------------
// Spread Operator
// ---------------------------------------------------------------------------

/// The spread/star operator: `*expr`.
///
/// Used in argument lists and collection literals to expand a collection:
/// - `foo(*items)` — spread items as arguments
/// - `[1, *list, 3]` — spread list into a larger list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpreadOperator {
    pub span: Span,
    /// The expression being spread (should evaluate to a collection/iterable).
    pub expr: Box<Expr>,
}

// ---------------------------------------------------------------------------
// Misc Expression Types
// ---------------------------------------------------------------------------

/// A plain identifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identifier {
    pub span: Span,
    pub name: String,
}

/// A `this` reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThisExpr {
    pub span: Span,
}

/// A parenthesized expression: `(expr)`.
/// Preserved for correct operator precedence during pretty-printing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParenExpr {
    pub span: Span,
    pub expr: Box<Expr>,
}

// ---------------------------------------------------------------------------
// Convenience constructors
// ---------------------------------------------------------------------------

impl Identifier {
    /// Create an identifier with a given name and unknown span.
    pub fn unnamed(name: impl Into<String>) -> Self {
        Self {
            span: Span::unknown(),
            name: name.into(),
        }
    }

    /// Create an identifier with span information.
    pub fn new(span: Span, name: impl Into<String>) -> Self {
        Self {
            span,
            name: name.into(),
        }
    }
}

impl Expr {
    /// Create an identifier expression.
    pub fn ident(name: impl Into<String>) -> Self {
        Expr::Identifier(Identifier::unnamed(name))
    }

    /// Create a string literal expression (no interpolation).
    pub fn string(text: impl Into<String>) -> Self {
        Expr::String(StringLiteral {
            span: Span::unknown(),
            quote: QuoteStyle::Double,
            parts: vec![StringPart::Literal { text: text.into() }],
        })
    }

    /// Create a boolean literal expression.
    pub fn boolean(value: bool) -> Self {
        Expr::Boolean(BooleanLiteral {
            span: Span::unknown(),
            value,
        })
    }

    /// Create a null literal expression.
    pub fn null() -> Self {
        Expr::Null(NullLiteral {
            span: Span::unknown(),
        })
    }

    /// Create an unqualified method call.
    pub fn call(name: impl Into<String>, args: Vec<Arg>) -> Self {
        Expr::MethodCall(MethodCall {
            span: Span::unknown(),
            receiver: None,
            name: name.into(),
            arguments: args,
            trailing_closure: None,
        })
    }

    /// Create a property access on a receiver.
    pub fn prop(receiver: Expr, property: impl Into<String>) -> Self {
        Expr::PropertyAccess(PropertyAccess {
            span: Span::unknown(),
            object_expr: Box::new(receiver),
            property: property.into(),
        })
    }

    /// Create a simple assignment.
    pub fn assign(target: Expr, value: Expr) -> Self {
        Expr::Assignment(Assignment {
            span: Span::unknown(),
            target: Box::new(target),
            value: Box::new(value),
        })
    }
}

impl Arg {
    /// Create a positional argument from an expression.
    pub fn positional(expr: Expr) -> Self {
        Arg::Positional {
            expr: Box::new(expr),
        }
    }

    /// Create a named argument.
    pub fn named(name: impl Into<String>, value: Expr) -> Self {
        Arg::Named(NamedArgument {
            span: Span::unknown(),
            name: name.into(),
            value: Box::new(value),
        })
    }
}

impl StringLiteral {
    /// The concatenated plain-text value of this string (ignoring interpolation).
    /// Useful for simple strings that are known to have no interpolation.
    pub fn plain_value(&self) -> Option<String> {
        let mut result = String::new();
        for part in &self.parts {
            match part {
                StringPart::Literal { text } => result.push_str(text),
                StringPart::Interpolation { .. } => return None,
            }
        }
        Some(result)
    }
}

impl Closure {
    /// Create a parameterless closure with a body of statements.
    pub fn from_stmts(stmts: Vec<Stmt>) -> Self {
        Self {
            span: Span::unknown(),
            params: Vec::new(),
            body: stmts,
        }
    }

    /// Create a closure with a single expression body (wrapped in an ExprStmt).
    pub fn from_expr(expr: Expr) -> Self {
        Self {
            span: Span::unknown(),
            params: Vec::new(),
            body: vec![Stmt::Expr(ExprStmt {
                span: Span::unknown(),
                expr: Box::new(expr),
            })],
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_span_merge() {
        let a = Span::new(0, 5, 1, 1);
        let b = Span::new(10, 20, 2, 3);
        let merged = a.merge(&b);
        assert_eq!(merged.start, 0);
        assert_eq!(merged.end, 20);
        assert_eq!(merged.line, 1);
    }

    #[test]
    fn test_span_len_and_empty() {
        let full = Span::new(0, 10, 1, 1);
        assert_eq!(full.len(), 10);
        assert!(!full.is_empty());

        let empty = Span::new(5, 5, 1, 5);
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());
    }

    #[test]
    fn test_string_literal_plain_value() {
        let lit = StringLiteral {
            span: Span::unknown(),
            quote: QuoteStyle::Double,
            parts: vec![StringPart::Literal {
                text: "hello".into(),
            }],
        };
        assert_eq!(lit.plain_value(), Some("hello".to_string()));
    }

    #[test]
    fn test_string_literal_with_interpolation() {
        let lit = StringLiteral {
            span: Span::unknown(),
            quote: QuoteStyle::Double,
            parts: vec![
                StringPart::Literal {
                    text: "Hello ".into(),
                },
                StringPart::Interpolation {
                    expr: Box::new(Expr::ident("name")),
                    span: Span::unknown(),
                },
                StringPart::Literal { text: "!".into() },
            ],
        };
        assert_eq!(lit.plain_value(), None);
    }

    #[test]
    fn test_expr_convenience_constructors() {
        let id = Expr::ident("foo");
        assert!(matches!(id, Expr::Identifier(i) if i.name == "foo"));

        let s = Expr::string("bar");
        assert!(matches!(s, Expr::String(lit) if lit.plain_value() == Some("bar".to_string())));

        let b = Expr::boolean(true);
        assert!(matches!(b, Expr::Boolean(lit) if lit.value));

        let n = Expr::null();
        assert!(matches!(n, Expr::Null(_)));

        let c = Expr::call("println", vec![Arg::positional(Expr::string("hi"))]);
        assert!(matches!(c, Expr::MethodCall(mc) if mc.name == "println"));

        let p = Expr::prop(Expr::ident("obj"), "field");
        assert!(matches!(p, Expr::PropertyAccess(pa) if pa.property == "field"));

        let a = Expr::assign(Expr::ident("x"), Expr::string("val"));
        assert!(matches!(a, Expr::Assignment(_)));
    }

    #[test]
    fn test_arg_convenience() {
        let pos = Arg::positional(Expr::string("arg"));
        assert!(matches!(pos, Arg::Positional { .. }));

        let named = Arg::named("key", Expr::string("value"));
        assert!(matches!(named, Arg::Named(na) if na.name == "key"));
    }

    #[test]
    fn test_serde_roundtrip_script() {
        let script = Script {
            span: Span::new(0, 100, 1, 1),
            dialect: Dialect::KotlinDsl,
            statements: vec![Stmt::Expr(ExprStmt {
                span: Span::new(0, 30, 1, 1),
                expr: Box::new(Expr::MethodCall(MethodCall {
                    span: Span::new(0, 30, 1, 1),
                    receiver: None,
                    name: "plugins".to_string(),
                    arguments: vec![],
                    trailing_closure: Some(Box::new(Closure::from_stmts(vec![]))),
                })),
            })],
            comments: vec![],
        };

        let json = serde_json::to_string(&script).unwrap();
        let back: Script = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dialect, Dialect::KotlinDsl);
        assert_eq!(back.statements.len(), 1);
    }

    #[test]
    fn test_serde_roundtrip_complex_expr() {
        let expr = Expr::Binary(BinaryExpr {
            span: Span::new(0, 20, 1, 1),
            left: Box::new(Expr::ident("version")),
            operator: BinaryOp::ElvisAssign,
            right: Box::new(Expr::string("1.0.0")),
        });

        let json = serde_json::to_string(&expr).unwrap();
        let back: Expr = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Expr::Binary(_)));
    }

    #[test]
    fn test_closure_from_expr() {
        let closure = Closure::from_expr(Expr::string("hello"));
        assert_eq!(closure.body.len(), 1);
        assert!(closure.params.is_empty());
    }
}
