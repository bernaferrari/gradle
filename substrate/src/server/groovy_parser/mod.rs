//! Gradle Groovy DSL parser module.
//!
//! This module provides a complete lexer + recursive-descent parser for
//! Gradle Groovy DSL build scripts (build.gradle, settings.gradle).
//!
//! # Architecture
//!
//! - **lexer.rs**: Tokenizer that converts source text into a stream of tokens.
//! - **ast.rs**: Abstract Syntax Tree node definitions.
//! - **parser.rs**: Recursive-descent parser that converts tokens into AST.
//!
//! # Usage
//!
//! ```ignore
//! use substrate::server::groovy_parser;
//!
//! let result = groovy_parser::parse("compileSdk 34");
//! if result.has_errors() {
//!     for err in &result.errors {
//!         eprintln!("{}", err);
//!     }
//! }
//! ```

pub mod ast;
pub mod lexer;
pub mod parser;

pub use ast::*;
pub use lexer::tokenize;
pub use parser::{parse, parse_or_error, ParseError, ParseResult, Parser};
