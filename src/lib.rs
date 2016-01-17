//! Rust implementation of JMESPath, a query language for JSON.
//!
//! # Usage
//!
//! This crate is [on crates.io](https://crates.io/crates/jmespath) and
//! can be used by adding `jmespath` to the dependencies in your
//! project's `Cargo.toml`.
//!
//! ```toml
//! [dependencies]
//! jmespath = "0.0.1"
//! ```
//!
//! and this to your crate root:
//!
//! ```rust
//! extern crate jmespath;
//! ```
//!
//! # Compiling JMESPath expressions
//!
//! Use the `jmespath::Expression` struct to compile and execute JMESPath
//! expressions. The `Expression` struct can be used multiple times on
//! different values without having to recompile the expression.
//!
//! ```
//! use jmespath;
//!
//! let expr = jmespath::Expression::new("foo.bar | baz").unwrap();
//!
//! // Parse some JSON data into a JMESPath variable
//! let json_str = "{\"foo\":{\"bar\":{\"baz\":true}}}";
//! let data = jmespath::Variable::from_json(json_str).unwrap();
//!
//! // Search the data with the compiled expression
//! let result = expr.search(data).unwrap();
//! assert_eq!(true, result.as_boolean().unwrap());
//! ```
//!
//! You can get the original expression as a string and the parsed expression
//! AST from the `Expression` struct:
//!
//! ```
//! use jmespath;
//! use jmespath::ast::Ast;
//!
//! let expr = jmespath::Expression::new("foo").unwrap();
//! assert_eq!("foo", expr.as_str());
//! assert_eq!(&Ast::Field {name: "foo".to_string(), offset: 0}, expr.as_ast());
//! ```
//!
//! # Using `jmespath::search`
//!
//! The `jmespath::search` function can be used for more simplified searching
//! when expression reuse is not important. `jmespath::search` will compile
//! the given expression and evaluate the expression against the provided
//! data.
//!
//! ```
//! use jmespath;
//!
//! let data = jmespath::Variable::from_json("{\"foo\":null}").unwrap();
//! let result = jmespath::search("foo", data).unwrap();
//! assert!(result.is_null());
//! ```
//!
//! ## JMESPath variables
//!
//! In order to evaluate expressions against a known data type, the
//! `jmespath::Variable` enum is used as both the input and output type.
//! More specifically, `Rc<Variable>` (or `jmespath::RcVar`) is used to allow
//! shared, reference counted data to be used by the JMESPath interpreter at
//! runtime.
//!
//! Because `jmespath::Variable` implements `serde::ser::Serialize`, many
//! existing types can be searched without needing an explicit coercion,
//! and any type that needs coercion can be implemented using serde's macros
//! or code generation capabilities. Any value that implements the
//! `serde::ser::Serialize` trait can be searched without needing explicit
//! coercions. This includes a number of common types, including serde's
//! `serde_json::Value` enum.

extern crate serde;
extern crate serde_json;

pub use parser::{parse, ParseResult};
pub use variable::Variable;

use std::fmt;
use std::rc::Rc;

use self::serde::Serialize;

use ast::Ast;
use variable::Serializer;
use interpreter::{TreeInterpreter, Context, SearchResult};

pub mod ast;
pub mod functions;
mod parser;
mod lexer;
pub mod interpreter;
mod variable;

pub type RcVar = Rc<Variable>;

/// Parses an expression and performs a search over the data.
pub fn search<T: Serialize>(expression: &str, data: T) -> Result<RcVar, Error> {
    Expression::new(expression).and_then(|expr| expr.search(data))
}

/// JMESPath error
#[derive(Clone,Debug,PartialEq)]
pub struct Error {
    /// Coordinates to where the error was encountered in the original
    /// expression string.
    pub coordinates: Coordinates,
    /// Expression being evaluated.
    pub expression: String,
    /// Error reason information.
    pub error_reason: ErrorReason
}

impl Error {
    /// Create a new JMESPath Error
    pub fn new(expr: &str, offset: usize, error_reason: ErrorReason) -> Error {
        Error {
            expression: expr.to_string(),
            coordinates: Coordinates::from_offset(expr, offset),
            error_reason: error_reason
        }
    }

    /// Create a new JMESPath Error from a Context struct.
    pub fn from_ctx(ctx: &Context, error_reason: ErrorReason) -> Error {
        Error {
            expression: ctx.expression.to_string(),
            coordinates: ctx.create_coordinates(),
            error_reason: error_reason
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(fmt, "{} ({})\n{}", self.error_reason, self.coordinates,
            self.coordinates.expression_with_carat(&self.expression))
    }
}

/// Error context provides specific details about an error.
#[derive(Clone,Debug,PartialEq)]
pub enum ErrorReason {
    /// An error occurred while parsing an expression.
    Parse(String),
    /// An error occurred while evaluating an expression.
    Runtime(RuntimeError)
}

impl fmt::Display for ErrorReason {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self {
            &ErrorReason::Parse(ref e) => write!(fmt, "Parse error: {}", e),
            &ErrorReason::Runtime(ref e) => write!(fmt, "Runtime error: {}", e),
        }
    }
}

/// Runtime JMESPath error
#[derive(Clone,Debug,PartialEq)]
pub enum RuntimeError {
    /// Encountered when a slice expression uses a step of 0
    InvalidSlice,
    /// Encountered when a key is not a string.
    InvalidKey(String),
    /// Encountered when too many arguments are provided to a function.
    TooManyArguments {
        expected: usize,
        actual: usize,
    },
    /// Encountered when too few arguments are provided to a function.
    NotEnoughArguments {
        expected: usize,
        actual: usize,
    },
    /// Encountered when an unknown function is called.
    UnknownFunction(String),
    /// Encountered when a type of variable given to a function is invalid.
    InvalidType {
        expected: String,
        actual: String,
        actual_value: RcVar,
        position: usize,
    },
    /// Encountered when an expression reference returns an invalid type.
    InvalidReturnType {
        expected: String,
        actual: String,
        actual_value: RcVar,
        position: usize,
        invocation: usize,
    },
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use self::RuntimeError::*;
        match self {
            &UnknownFunction(ref function) => {
                write!(fmt, "Call to undefined function {}", function)
            },
            &TooManyArguments { ref expected, ref actual } => {
                write!(fmt, "Too many arguments, expected {}, found {}", expected, actual)
            },
            &NotEnoughArguments { ref expected, ref actual } => {
                write!(fmt, "Not enough arguments, expected {}, found {}", expected, actual)
            },
            &InvalidType { ref expected, ref actual, ref position, ref actual_value } => {
                write!(fmt, "Argument {} expects type {}, given {} {}",
                    position, expected, actual, actual_value.to_string())
            },
            &InvalidSlice => {
                write!(fmt, "Invalid slice")
            },
            &InvalidReturnType { ref expected, ref actual, ref position, ref invocation,
                    ref actual_value } => {
                write!(fmt, "Argument {} must return {} but invocation {} returned {} {}",
                    position, expected, invocation, actual, actual_value.to_string())
            },
            &InvalidKey(ref actual) => {
                write!(fmt, "Invalid key. Expected string, found {:?}", actual)
            },
        }
    }
}

/// Defines the coordinates to a position in an expression string.
#[derive(Clone, Debug, PartialEq)]
pub struct Coordinates {
    /// Absolute character position.
    pub offset: usize,
    /// Line number of the coordinate.
    pub line: usize,
    /// Column of the line number.
    pub column: usize,
}

impl Coordinates {
    /// Create an expression coordinates struct based on an offset
    // position in the expression.
    pub fn from_offset(expr: &str, offset: usize) -> Coordinates {
        // Find each new line and create a formatted error message.
        let mut current_line: usize = 0;
        let mut current_col: usize = 0;
        for c in expr.chars().take(offset) {
            match c {
                '\n' => {
                    current_line += 1;
                    current_col = 0;
                },
                _ => current_col += 1
            }
        }
        Coordinates {
            line: current_line,
            column: current_col,
            offset: offset
        }
    }

    fn inject_carat(&self, buff: &mut String) {
        buff.push_str(&(0..self.column).map(|_| ' ').collect::<String>());
        buff.push_str(&"^\n");
    }

    /// Returns a string that shows the expression and a carat pointing to
    /// the coordinate.
    pub fn expression_with_carat(&self, expr: &str) -> String {
        let mut buff = String::new();
        let mut matched = false;
        let mut current_line = 0;
        for c in expr.chars() {
            buff.push(c);
            if c == '\n' {
                current_line += 1;
                if current_line == self.line + 1 {
                    matched = true;
                    self.inject_carat(&mut buff);
                }
            }
        }
        if !matched {
            buff.push('\n');
            self.inject_carat(&mut buff);
        }
        buff
    }
}

impl fmt::Display for Coordinates {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(fmt, "line {}, column {}", self.line, self.column)
    }
}

/// A compiled JMESPath expression.
pub struct Expression<'a> {
    ast: Ast,
    original: String,
    interpreter: Option<&'a TreeInterpreter>
}

impl<'a> Expression<'a> {
    /// Creates a new JMESPath expression from an expression string.
    pub fn new(expression: &str) -> Result<Expression<'a>, Error> {
        Expression::with_interpreter(expression, None)
    }

    /// Creates a new JMESPath expression using a custom tree interpreter.
    /// Customer interpreters may be desired when you wish to utilize custom
    /// JMESPath functions in your expressions.
    #[inline]
    pub fn with_interpreter(expression: &str,
                            interpreter: Option<&'a TreeInterpreter>)
                            -> Result<Expression<'a>, Error> {
        Ok(Expression {
            original: expression.to_string(),
            ast: try!(parse(expression)),
            interpreter: interpreter
        })
    }

    /// Returns the result of searching data with the compiled expression.
    pub fn search<T: Serialize>(&self, data: T) -> SearchResult {
        let mut ser = Serializer::new();
        data.serialize(&mut ser).ok().unwrap();
        let data = Rc::new(ser.unwrap());
        match self.interpreter {
            Some(i) => {
                let mut ctx = Context::new(i, &self.original);
                i.interpret(&data, &self.ast, &mut ctx)
            },
            None => {
                let interpreter = TreeInterpreter::new();
                let mut ctx = Context::new(&interpreter, &self.original);
                interpreter.interpret(&data, &self.ast, &mut ctx)
            }
        }
    }

    /// Returns the JMESPath expression from which the Expression was compiled.
    pub fn as_str(&self) -> &str {
        &self.original
    }

    /// Returns the AST of the parsed JMESPath expression.
    pub fn as_ast(&self) -> &Ast {
        &self.ast
    }
}

impl<'a> fmt::Display for Expression<'a> {
    /// Shows the original jmespath expression.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl<'a> fmt::Debug for Expression<'a> {
    /// Shows the original jmespath expression.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Equality comparison is based on the original string.
impl<'a> PartialEq for Expression<'a> {
    fn eq(&self, other: &Expression) -> bool {
        self.as_str() == other.as_str()
    }
}

#[cfg(test)]
mod test {
    use std::rc::Rc;

    use super::*;
    use super::ast::Ast;

    #[test]
    fn formats_expression_as_string_or_debug() {
        let expr = Expression::new("foo | baz").unwrap();
        assert_eq!("foo | baz/foo | baz", format!("{}/{:?}", expr, expr));
    }

    #[test]
    fn implements_partial_eq() {
        let a = Expression::new("@").unwrap();
        let b = Expression::new("@").unwrap();
        assert!(a == b);
    }

    #[test]
    fn can_evaluate_jmespath_expression() {
        let expr = Expression::new("foo.bar").unwrap();
        let var = Variable::from_json("{\"foo\":{\"bar\":true}}").unwrap();
        assert_eq!(Rc::new(Variable::Bool(true)), expr.search(var).unwrap());
    }

    #[test]
    fn can_search() {
        assert_eq!(Rc::new(Variable::Bool(true)), search("`true`", ()).unwrap());
    }

    #[test]
    fn can_get_expression_ast() {
        let expr = Expression::new("foo").unwrap();
        assert_eq!(&Ast::Field {offset: 0, name: "foo".to_string()}, expr.as_ast());
    }

    #[test]
    fn coordinates_can_be_created_from_string_with_new_lines() {
        let expr = "foo\n..bar";
        let coords = Coordinates::from_offset(expr, 5);
        assert_eq!(1, coords.line);
        assert_eq!(1, coords.column);
        assert_eq!(5, coords.offset);
        assert_eq!("foo\n..bar\n ^\n", coords.expression_with_carat(expr));
    }

    #[test]
    fn coordinates_can_be_created_from_string_with_new_lines_pointing_to_non_last() {
        let expr = "foo\n..bar\nbaz";
        let coords = Coordinates::from_offset(expr, 5);
        assert_eq!(1, coords.line);
        assert_eq!(1, coords.column);
        assert_eq!(5, coords.offset);
        assert_eq!("foo\n..bar\n ^\nbaz", coords.expression_with_carat(expr));
    }

    #[test]
    fn coordinates_can_be_created_from_string_with_no_new_lines() {
        let expr = "foo..bar";
        let coords = Coordinates::from_offset(expr, 4);
        assert_eq!(0, coords.line);
        assert_eq!(4, coords.column);
        assert_eq!(4, coords.offset);
        assert_eq!("foo..bar\n    ^\n", coords.expression_with_carat(expr));
    }
}
