use std::{fmt, iter::Peekable, str::SplitWhitespace};

use reblessive::Stk;

#[derive(Debug)]
enum UnaryOperator {
    Neg,
    Pos,
}

#[derive(Debug)]
enum BinaryOperator {
    Pow,
    Mul,
    Div,
    Add,
    Sub,
}

#[derive(Debug)]
enum Expression {
    Number(f64),
    Covered(Box<Expression>),
    Binary {
        left: Box<Expression>,
        op: BinaryOperator,
        right: Box<Expression>,
    },
    Unary {
        op: UnaryOperator,
        expr: Box<Expression>,
    },
}

#[derive(Debug)]
pub enum Error {
    Parse,
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse => write!(f, "Failed to parse expression"),
        }
    }
}

async fn parse(
    ctx: &mut Stk,
    tokens: &mut Peekable<SplitWhitespace<'_>>,
    binding_power: u8,
) -> Result<Expression, Error> {
    let peek = tokens.peek().copied();
    let mut lhs = match peek {
        Some("+") => {
            tokens.next();
            let expr = ctx.run(|ctx| parse(ctx, tokens, 7)).await?;
            Expression::Unary {
                op: UnaryOperator::Pos,
                expr: Box::new(expr),
            }
        }
        Some("-") => {
            tokens.next();
            let expr = ctx.run(|ctx| parse(ctx, tokens, 7)).await?;
            Expression::Unary {
                op: UnaryOperator::Neg,
                expr: Box::new(expr),
            }
        }
        Some("(") => {
            tokens.next();
            let expr = ctx.run(|ctx| parse(ctx, tokens, 0)).await?;
            let Some(")") = tokens.next() else {
                return Err(Error::Parse);
            };
            Expression::Covered(Box::new(expr))
        }
        Some(x) => {
            tokens.next();
            let num = x.parse::<f64>().map_err(|_| Error::Parse)?;
            Expression::Number(num)
        }
        _ => {
            return Err(Error::Parse);
        }
    };

    loop {
        let token = tokens.peek().copied();
        let (op, bp) = match token {
            Some("**") => (BinaryOperator::Pow, (5, 6)),
            Some("*") => (BinaryOperator::Mul, (3, 4)),
            Some("/") => (BinaryOperator::Div, (3, 4)),
            Some("+") => (BinaryOperator::Add, (1, 2)),
            Some("-") => (BinaryOperator::Sub, (1, 2)),
            _ => break,
        };

        if bp.0 < binding_power {
            break;
        }

        tokens.next();

        let rhs = ctx.run(|ctx| parse(ctx, tokens, bp.1)).await?;

        lhs = Expression::Binary {
            left: Box::new(lhs),
            op,
            right: Box::new(rhs),
        }
    }

    Ok(lhs)
}

async fn eval(ctx: &mut Stk, expr: &Expression) -> f64 {
    match expr {
        Expression::Number(x) => *x,
        Expression::Covered(ref x) => ctx.run(|ctx| eval(ctx, x)).await,
        Expression::Binary { left, op, right } => {
            let left = ctx.run(|ctx| eval(ctx, left)).await;
            let right = ctx.run(|ctx| eval(ctx, right)).await;
            match op {
                BinaryOperator::Pow => left.powf(right),
                BinaryOperator::Mul => left * right,
                BinaryOperator::Div => left / right,
                BinaryOperator::Add => left + right,
                BinaryOperator::Sub => left - right,
            }
        }
        Expression::Unary { op, expr } => {
            let expr = ctx.run(|ctx| eval(ctx, expr)).await;
            match op {
                UnaryOperator::Neg => -expr,
                UnaryOperator::Pos => expr,
            }
        }
    }
}

// A recursively defined simple calculater which can parse arbitrary depth expressions without
// ever overflowing the stack.
fn main() -> Result<(), Error> {
    let expr = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if expr.is_empty() {
        return Ok(());
    }
    let mut stack = reblessive::Stack::new();
    let mut tokens = expr.split_whitespace().peekable();
    let expr = stack.run(|ctx| parse(ctx, &mut tokens, 0)).finish()?;

    eprintln!("EXPRESSION:\n{:#?}", expr);

    println!("{}", stack.run(|ctx| eval(ctx, &expr)).finish());

    Ok(())
}
